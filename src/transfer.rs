//! Moving programs to another PC.
//!
//! **Export** collects everything that belongs to the chosen programs —
//! precisely from an install log when the program was monitored, otherwise
//! with the leftover scanner (install folder, data folders, shortcuts,
//! registry keys, file associations, services, scheduled tasks) — and packs
//! it into one `.setupsweep` file (a zip with a JSON manifest). Paths inside
//! user profiles and Program Files are stored as `%VARIABLES%`, so they land
//! in the right place even when the user name or drive differs.
//!
//! **Restore** checks compatibility first, then writes files and registry
//! entries through a [`BackupWriter`], so the whole transfer can be undone
//! from the Backups page.

use crate::backup::{ActionResult, BackupWriter};
use crate::leftovers::{find_leftovers, LeftoverScope};
use crate::model::{fold, ChangeSet, KeyData, RegValue, ValueRef};
use crate::platform::{self, expand_env, Registry};
use crate::programs::{InstalledProgram, Source};
use crate::revert::{build_plan, Action};
use crate::util::{is_protected_dir, is_protected_key, protected_dirs};
use anyhow::{Context, Result};
use std::collections::BTreeSet;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

pub const FORMAT: u32 = 1;
pub const EXTENSION: &str = "setupsweep";

/// Folder variables used to make paths portable, longest value first.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Vars(pub Vec<(String, String)>);

// A newtype is written as what it wraps: a list of [name, value] pairs.
impl crate::json::ToJson for Vars {
    fn to_json(&self) -> crate::json::Value {
        crate::json::ToJson::to_json(&self.0)
    }
}
impl crate::json::FromJson for Vars {
    fn from_json(v: &crate::json::Value) -> Result<Self, crate::json::Error> {
        crate::json::FromJson::from_json(v).map(Vars)
    }
}

impl Vars {
    const NAMES: [&'static str; 10] = [
        "LOCALAPPDATA",
        "APPDATA",
        "PUBLIC",
        "ProgramData",
        "CommonProgramFiles(x86)",
        "CommonProgramFiles",
        "ProgramFiles(x86)",
        "ProgramFiles",
        "USERPROFILE",
        "SystemDrive",
    ];

    pub fn current() -> Self {
        Vars::from_pairs(
            Self::NAMES
                .iter()
                .map(|n| (n.to_string(), expand_env(&format!("%{n}%"))))
                .filter(|(_, v)| !v.contains('%') && !v.is_empty())
                .collect(),
        )
    }

    pub fn from_pairs(mut v: Vec<(String, String)>) -> Self {
        v.sort_by_key(|(_, val)| std::cmp::Reverse(val.len()));
        Vars(v)
    }

    pub fn get(&self, name: &str) -> Option<&str> {
        self.0
            .iter()
            .find(|(n, _)| n.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }

    /// `C:\Users\Ann\AppData\Roaming\X` → `%APPDATA%\X`.
    pub fn to_placeholder(&self, path: &str) -> String {
        let f = fold(path);
        for (name, val) in &self.0 {
            let fv = fold(val.trim_end_matches(['\\', '/']));
            if f == fv || f.starts_with(&format!("{fv}\\")) || f.starts_with(&format!("{fv}/")) {
                return format!("%{name}%{}", &path[fv.len()..]);
            }
        }
        path.to_string()
    }

    /// `%APPDATA%\X` → this PC's path.
    pub fn from_placeholder(&self, path: &str) -> String {
        if let Some(rest) = path.strip_prefix('%') {
            if let Some((name, tail)) = rest.split_once('%') {
                if let Some(v) = self.get(name) {
                    return format!("{}{tail}", v.trim_end_matches(['\\', '/']));
                }
            }
        }
        path.to_string()
    }
}

/// Replace the source PC's folders with this PC's inside registry text
/// (case-insensitive). Only folders that differ are touched.
pub fn rewrite_text(s: &str, from: &Vars, to: &Vars) -> String {
    let mut out = s.to_string();
    for (name, old) in &from.0 {
        let Some(new) = to.get(name) else { continue };
        if old.eq_ignore_ascii_case(new) || old.len() < 4 {
            continue;
        }
        let mut res = String::with_capacity(out.len());
        let lower = out.to_lowercase();
        let needle = old.to_lowercase();
        let mut i = 0;
        while let Some(pos) = lower[i..].find(&needle) {
            res.push_str(&out[i..i + pos]);
            res.push_str(new);
            i += pos + needle.len();
        }
        res.push_str(&out[i..]);
        out = res;
    }
    out
}

fn rewrite_value(v: &RegValue, from: &Vars, to: &Vars) -> RegValue {
    use crate::model::reg_type;
    match v.kind {
        reg_type::SZ | reg_type::EXPAND_SZ => {
            let s = v.as_string().unwrap_or_default();
            let mut nv = v.clone();
            nv.data = crate::model::encode_utf16z(&rewrite_text(&s, from, to));
            nv
        }
        reg_type::MULTI_SZ => {
            let items: Vec<String> = crate::model::decode_multi_sz(&v.data)
                .iter()
                .map(|s| rewrite_text(s, from, to))
                .collect();
            let mut nv = v.clone();
            nv.data = crate::model::encode_multi_sz(&items);
            nv
        }
        _ => v.clone(),
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ProgramInfo {
    pub name: String,
    pub publisher: String,
    pub version: String,
}
crate::json_struct!(ProgramInfo, { name, publisher, version });

#[derive(Clone, Debug, PartialEq)]
pub struct FileItem {
    /// Destination with `%VARIABLES%`.
    pub path: String,
    /// Zip entry holding the content.
    pub entry: String,
    pub size: u64,
}
crate::json_struct!(FileItem, { path, entry, size });

#[derive(Clone, Debug, PartialEq)]
pub struct PathEntry {
    pub key: String,
    pub name: String,
    pub entries: Vec<String>,
}
crate::json_struct!(PathEntry, { key, name, entries });

#[derive(Clone, Debug, PartialEq)]
pub struct TaskItem {
    pub name: String,
    pub entry: String,
}
crate::json_struct!(TaskItem, { name, entry });

#[derive(Clone, Debug, Default)]
pub struct Manifest {
    pub format: u32,
    pub created: String,
    pub computer: String,
    pub user: String,
    pub windows_build: u32,
    pub arch: String,
    pub vars: Vars,
    pub programs: Vec<ProgramInfo>,
    pub files: Vec<FileItem>,
    /// Folders to create (with `%VARIABLES%`), including empty ones.
    pub dirs: Vec<String>,
    pub keys: Vec<KeyData>,
    pub values: Vec<ValueRef>,
    pub path_entries: Vec<PathEntry>,
    pub tasks: Vec<TaskItem>,
    pub services: Vec<String>,
    pub warnings: Vec<String>,
}
crate::json_struct!(Manifest, { format, created, computer, user, windows_build, arch, vars, programs, files, dirs, keys, values, path_entries, tasks, services, warnings });

impl Manifest {
    pub fn bytes(&self) -> u64 {
        self.files.iter().map(|f| f.size).sum()
    }
}

/// What was found for the chosen programs, before packing.
#[derive(Debug, Default, Clone)]
pub struct Collected {
    pub programs: Vec<ProgramInfo>,
    pub files: BTreeSet<String>,
    pub dirs: BTreeSet<String>,
    pub keys: BTreeSet<String>,
    pub values: BTreeSet<(String, String)>,
    pub path_entries: Vec<PathEntry>,
    pub tasks: BTreeSet<String>,
    pub services: BTreeSet<String>,
    pub warnings: Vec<String>,
    pub bytes: u64,
}

/// Programs that usually don't work after being copied: they install
/// drivers, are tied to the machine's activation, or protect themselves.
const KNOWN_UNMOVABLE: &[(&str, &str)] = &[
    ("microsoft office", "activation and system integration"),
    ("microsoft 365", "activation and system integration"),
    ("visual studio", "installs shared components and drivers"),
    ("nvidia", "drivers"),
    ("amd software", "drivers"),
    ("intel", "drivers"),
    ("realtek", "drivers"),
    ("norton", "security software protects itself"),
    ("mcafee", "security software protects itself"),
    ("kaspersky", "security software protects itself"),
    ("bitdefender", "security software protects itself"),
    ("avast", "security software protects itself"),
    ("eset", "security software protects itself"),
    ("malwarebytes", "security software protects itself"),
    ("adobe creative cloud", "activation"),
    ("vmware", "drivers"),
    ("virtualbox", "drivers"),
];

pub fn unmovable_reason(p: &InstalledProgram) -> Option<&'static str> {
    let hay = fold(&format!("{} {}", p.name, p.publisher));
    KNOWN_UNMOVABLE
        .iter()
        .find(|(k, _)| hay.contains(k))
        .map(|(_, why)| *why)
}

fn walk_into(dir: &str, c: &mut Collected) {
    for e in crate::fswalk::WalkDir::new(dir)
        .follow_links(false)
        .into_iter()
        .flatten()
    {
        let p = e.path().to_string_lossy().to_string();
        if e.file_type().is_dir() {
            c.dirs.insert(p);
        } else if e.file_type().is_file() && c.files.insert(p) {
            c.bytes += e.metadata().map(|m| m.len()).unwrap_or(0);
        }
    }
}

/// Gather what belongs to each program. `logs` holds the install log's
/// changes for monitored programs (same order as `programs`).
pub fn collect(
    programs: &[InstalledProgram],
    logs: &[Option<ChangeSet>],
    reg: &dyn Registry,
    progress: &dyn Fn(String),
) -> Collected {
    let mut c = Collected::default();
    let protected = protected_dirs();
    let sysroot = fold(&expand_env("%SystemRoot%"));
    let in_windows = |p: &str| !sysroot.contains('%') && fold(p).starts_with(&sysroot);
    let scope = LeftoverScope::default();
    for (i, p) in programs.iter().enumerate() {
        progress(p.name.clone());
        if p.source == Source::Store {
            c.warnings.push(format!(
                "{}: Microsoft Store apps can't be moved; install it from the Store on the new PC.",
                p.name
            ));
            continue;
        }
        if let Some(why) = unmovable_reason(p) {
            c.warnings
                .push(format!("{}: may not work after moving ({why}).", p.name));
        }
        c.programs.push(ProgramInfo {
            name: p.name.clone(),
            publisher: p.publisher.clone(),
            version: p.version.clone(),
        });
        let mut plan = match logs.get(i).and_then(|l| l.as_ref()) {
            Some(cs) => build_plan(cs, reg),
            None => find_leftovers(p, reg, &scope),
        };
        // A monitored program's log may lack its Uninstall entry (if the
        // installer wrote it under a different key); always include it.
        if reg.exists(&p.key) {
            plan.items.push(crate::revert::PlanItem {
                action: Action::DeleteKey {
                    path: p.key.clone(),
                },
                selected: true,
                exists: true,
                note: String::new(),
            });
        }
        for item in plan.items.iter().filter(|i| i.selected && i.exists) {
            match &item.action {
                Action::DeleteFile { path } => {
                    if in_windows(path) {
                        continue;
                    }
                    if c.files.insert(path.clone()) {
                        c.bytes += std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
                    }
                }
                Action::DeleteDir { path } | Action::DeleteTree { path } => {
                    if in_windows(path) || is_protected_dir(path, &protected) {
                        continue;
                    }
                    walk_into(path, &mut c);
                }
                Action::DeleteKey { path } => {
                    if !is_protected_key(path) {
                        c.keys.insert(path.clone());
                    }
                }
                Action::DeleteValue { key, name } => {
                    c.values.insert((key.clone(), name.clone()));
                }
                Action::EditList {
                    key, name, remove, ..
                } => c.path_entries.push(PathEntry {
                    key: key.clone(),
                    name: name.clone(),
                    entries: remove.clone(),
                }),
                Action::StopDeleteService { name } => {
                    c.services.insert(name.clone());
                    c.keys
                        .insert(format!("HKLM\\SYSTEM\\CurrentControlSet\\Services\\{name}"));
                }
                Action::DeleteTask { name } => {
                    c.tasks.insert(name.clone());
                }
                Action::SetValue { .. } | Action::RestoreKey { .. } => {}
            }
        }
    }
    for s in &c.services {
        let ty = reg
            .read_key(&format!("HKLM\\SYSTEM\\CurrentControlSet\\Services\\{s}"))
            .and_then(|k| k.get_u32("Type"))
            .unwrap_or(0);
        if ty & 0x3 != 0 && ty & 0x30 == 0 {
            c.warnings.push(format!(
                "{s} is a driver; drivers usually have to be installed again."
            ));
        }
    }
    if !c.services.is_empty() {
        c.warnings
            .push("Services are registered on the new PC but start only after a restart.".into());
    }
    c
}

/// Write the package.
pub fn export(
    c: &Collected,
    reg: &dyn Registry,
    out: &Path,
    progress: &dyn Fn(String),
) -> Result<Manifest> {
    export_with(c, reg, out, progress, &Vars::current())
}

pub fn export_with(
    c: &Collected,
    reg: &dyn Registry,
    out: &Path,
    progress: &dyn Fn(String),
    vars: &Vars,
) -> Result<Manifest> {
    let vars = vars.clone();
    let mut m = Manifest {
        format: FORMAT,
        created: crate::util::now_string(),
        computer: platform::computer_name(),
        user: platform::user_name(),
        windows_build: crate::platform::windows_build(reg),
        arch: std::env::consts::ARCH.into(),
        vars: vars.clone(),
        programs: c.programs.clone(),
        services: c.services.iter().cloned().collect(),
        warnings: c.warnings.clone(),
        ..Default::default()
    };
    let file =
        std::fs::File::create(out).with_context(|| format!("can't create {}", out.display()))?;
    let mut zip = crate::zip::ZipWriter::new(file)?;
    for (n, path) in c.files.iter().enumerate() {
        let Ok(mut f) = std::fs::File::open(path) else {
            m.warnings
                .push(format!("Skipped {path} (in use or not readable)"));
            continue;
        };
        if n % 50 == 0 {
            progress(path.clone());
        }
        let entry = format!("files/{n}");
        zip.start_file(&entry)?;
        let size = std::io::copy(&mut f, &mut zip)?;
        m.files.push(FileItem {
            path: vars.to_placeholder(path),
            entry,
            size,
        });
    }
    m.dirs = c.dirs.iter().map(|d| vars.to_placeholder(d)).collect();
    for k in &c.keys {
        m.keys.extend(platform::read_tree(reg, k));
    }
    for (key, name) in &c.values {
        if let Some(v) = reg.read_key(key).and_then(|k| k.get(name).cloned()) {
            m.values.push(ValueRef {
                key: key.clone(),
                value: v,
            });
        }
    }
    m.path_entries = c
        .path_entries
        .iter()
        .map(|p| PathEntry {
            entries: p.entries.iter().map(|e| vars.to_placeholder(e)).collect(),
            ..p.clone()
        })
        .collect();
    let tasks_dir = expand_env("%SystemRoot%\\System32\\Tasks");
    for (n, name) in c.tasks.iter().enumerate() {
        let file = format!("{tasks_dir}{}", name.replace('/', "\\"));
        if let Ok(bytes) = std::fs::read(&file) {
            let entry = format!("tasks/{n}.xml");
            zip.start_file(&entry)?;
            zip.write_all(&bytes)?;
            m.tasks.push(TaskItem {
                name: name.clone(),
                entry,
            });
        }
    }
    progress("manifest".into());
    zip.start_file("manifest.json")?;
    zip.write_all(crate::json::compat::to_string(&m)?.as_bytes())?;
    zip.finish()?;
    Ok(m)
}

pub fn read_manifest(pkg: &Path) -> Result<Manifest> {
    let mut zip = crate::zip::ZipArchive::new(std::fs::File::open(pkg)?)
        .context("not an SetupSweep transfer package")?;
    let mut s = String::new();
    zip.by_name("manifest.json")
        .context("not an SetupSweep transfer package")?
        .read_to_string(&mut s)?;
    let m: Manifest = crate::json::compat::from_str(&s)?;
    if m.format > FORMAT {
        anyhow::bail!("This package was made by a newer version of SetupSweep; please update.");
    }
    Ok(m)
}

/// Compatibility check before restoring.
#[derive(Debug, Default, Clone)]
pub struct Check {
    /// Reasons the restore can't go ahead.
    pub problems: Vec<String>,
    pub warnings: Vec<String>,
    /// Files that already exist on this PC.
    pub conflicts: Vec<String>,
    pub needs_admin: bool,
}

pub fn check(m: &Manifest, reg: &dyn Registry, vars: &Vars) -> Check {
    let mut c = Check {
        warnings: m.warnings.clone(),
        ..Default::default()
    };
    let here = std::env::consts::ARCH;
    if m.arch == "aarch64" && here != "aarch64" {
        c.problems
            .push("These programs were moved from an Arm PC and won't run on this one.".into());
    } else if m.arch != here {
        c.warnings.push(format!(
            "The package comes from a {} PC; this one is {here}.",
            m.arch
        ));
    }
    let build = crate::platform::windows_build(reg);
    if m.windows_build > 0 && build > 0 && build < m.windows_build {
        c.warnings.push(format!(
            "This PC runs an older Windows build ({build}) than the source ({}).",
            m.windows_build
        ));
    }
    let mut missing_drives = BTreeSet::new();
    let admin_roots: Vec<String> = [
        "ProgramFiles",
        "ProgramFiles(x86)",
        "ProgramData",
        "SystemDrive",
    ]
    .iter()
    .filter_map(|n| {
        vars.get(n)
            .map(|v| format!("{}\\", fold(v.trim_end_matches('\\'))))
    })
    .collect();
    for f in &m.files {
        let dest = vars.from_placeholder(&f.path);
        if dest.starts_with('%') {
            c.problems.push(format!("Unknown folder in {}", f.path));
            continue;
        }
        let b = dest.as_bytes();
        if b.len() >= 3 && b[0].is_ascii_alphabetic() && b[1] == b':' {
            let drive: String = dest.chars().take(3).collect();
            if !Path::new(&drive).exists() {
                missing_drives.insert(drive);
            }
        }
        let fd = fold(&dest);
        if admin_roots.iter().any(|r| fd.starts_with(r.as_str()))
            && !fd.starts_with(&fold(vars.get("USERPROFILE").unwrap_or("\u{0}")))
        {
            c.needs_admin = true;
        }
        if Path::new(&dest).exists() {
            c.conflicts.push(dest);
        }
    }
    for d in missing_drives {
        c.problems
            .push(format!("Drive {d} doesn't exist on this PC."));
    }
    if m.keys.iter().any(|k| k.path.starts_with("HKLM"))
        || m.values.iter().any(|v| v.key.starts_with("HKLM"))
    {
        c.needs_admin = true;
    }
    c
}

fn ensure_dir(path: &Path, backup: &mut BackupWriter) -> anyhow::Result<()> {
    if path.is_dir() {
        return Ok(());
    }
    if let Some(parent) = path.parent() {
        ensure_dir(parent, backup)?;
    }
    std::fs::create_dir(path)?;
    backup.add_dir_absent(&path.to_string_lossy())?;
    Ok(())
}

/// Restore a package on this PC. Every change is recorded in `backup`.
pub fn restore(
    pkg: &Path,
    reg: &dyn Registry,
    backup: &mut BackupWriter,
    overwrite: bool,
    progress: &dyn Fn(String),
) -> Result<Vec<ActionResult>> {
    let m = read_manifest(pkg)?;
    let to = Vars::current();
    restore_with(&m, pkg, reg, backup, overwrite, &to, progress)
}

pub fn restore_with(
    m: &Manifest,
    pkg: &Path,
    reg: &dyn Registry,
    backup: &mut BackupWriter,
    overwrite: bool,
    to: &Vars,
    progress: &dyn Fn(String),
) -> Result<Vec<ActionResult>> {
    let mut zip = crate::zip::ZipArchive::new(std::fs::File::open(pkg)?)?;
    let mut results = Vec::new();
    let from = &m.vars;

    // Folders, then files.
    for d in &m.dirs {
        let dest = PathBuf::from(to.from_placeholder(d));
        if let Err(e) = ensure_dir(&dest, backup) {
            results.push(ActionResult::err(
                format!("Folder {}", dest.display()),
                e.to_string(),
            ));
        }
    }
    for (n, f) in m.files.iter().enumerate() {
        let dest = PathBuf::from(to.from_placeholder(&f.path));
        let desc = format!("File {}", dest.display());
        if n % 50 == 0 {
            progress(dest.display().to_string());
        }
        if dest.exists() && !overwrite {
            results.push(ActionResult::ok(desc, "kept the existing file"));
            continue;
        }
        let res = (|| -> Result<()> {
            if let Some(parent) = dest.parent() {
                ensure_dir(parent, backup)?;
            }
            if dest.exists() {
                backup.add_file(&dest)?;
            } else {
                backup.add_file_absent(&dest.to_string_lossy())?;
            }
            let mut src = zip.by_name(&f.entry)?;
            let mut out = std::fs::File::create(&dest)?;
            std::io::copy(&mut src, &mut out)?;
            Ok(())
        })();
        if let Err(e) = res {
            results.push(ActionResult::err(desc, e.to_string()));
        }
    }
    let files_ok = m.files.len() - results.iter().filter(|r| !r.ok).count().min(m.files.len());
    results.push(ActionResult::ok(
        "Files".to_string(),
        format!("{files_ok} of {} copied", m.files.len()),
    ));

    // Registry keys (parents come before children in the list).
    progress("registry".into());
    for k in &m.keys {
        let desc = format!("Key {}", k.path);
        let res = (|| -> Result<()> {
            if !reg.exists(&k.path) {
                backup.add_key_absent(&k.path)?;
                reg.create_key(&k.path)?;
            }
            for v in k.values.values() {
                // Records the previous value, or that there was none.
                backup.add_value(reg, &k.path, &v.name)?;
                reg.set_value(&k.path, &rewrite_value(v, from, to))?;
            }
            Ok(())
        })();
        if let Err(e) = res {
            results.push(ActionResult::err(desc, e.to_string()));
        }
    }
    for v in &m.values {
        let desc = format!("Value {}\\{}", v.key, v.value.name);
        let res = (|| -> Result<()> {
            if !reg.exists(&v.key) {
                backup.add_key_absent(&v.key)?;
                reg.create_key(&v.key)?;
            }
            backup.add_value(reg, &v.key, &v.value.name)?;
            reg.set_value(&v.key, &rewrite_value(&v.value, from, to))?;
            Ok(())
        })();
        if let Err(e) = res {
            results.push(ActionResult::err(desc, e.to_string()));
        }
    }
    results.push(ActionResult::ok(
        "Registry".to_string(),
        format!("{} keys, {} values", m.keys.len(), m.values.len()),
    ));

    // PATH entries: append the ones that aren't there yet.
    for p in &m.path_entries {
        let current = reg
            .read_key(&p.key)
            .and_then(|k| k.get(&p.name).cloned())
            .and_then(|v| v.as_string().map(|s| (v.kind, s)));
        let (kind, cur) = current.unwrap_or((crate::model::reg_type::EXPAND_SZ, String::new()));
        let have: BTreeSet<String> = cur.split(';').map(|e| fold(e.trim())).collect();
        let add: Vec<String> = p
            .entries
            .iter()
            .map(|e| to.from_placeholder(e))
            .filter(|e| !have.contains(&fold(e)))
            .collect();
        if add.is_empty() {
            continue;
        }
        let mut new = cur.trim_end_matches(';').to_string();
        for e in &add {
            if !new.is_empty() {
                new.push(';');
            }
            new.push_str(e);
        }
        let mut val = RegValue::string(&p.name, &new);
        val.kind = kind;
        let r = backup
            .add_value(reg, &p.key, &p.name)
            .and_then(|_| reg.set_value(&p.key, &val));
        results.push(match r {
            Ok(()) => ActionResult::ok(
                format!("PATH ({})", p.key),
                format!("added {}", add.join("; ")),
            ),
            Err(e) => ActionResult::err(format!("PATH ({})", p.key), e.to_string()),
        });
    }

    // Scheduled tasks are registered through Task Scheduler.
    for t in &m.tasks {
        let desc = format!("Scheduled task {}", t.name);
        let res = (|| -> Result<()> {
            let mut xml = Vec::new();
            zip.by_name(&t.entry)?.read_to_end(&mut xml)?;
            let tmp =
                std::env::temp_dir().join(format!("setupsweep-task-{}.xml", crate::util::new_id()));
            std::fs::write(&tmp, &xml)?;
            let (code, out) = platform::run_hidden(
                "schtasks.exe",
                &[
                    "/Create",
                    "/TN",
                    &t.name,
                    "/XML",
                    &tmp.to_string_lossy(),
                    "/F",
                ],
            )?;
            let _ = std::fs::remove_file(&tmp);
            if code != 0 {
                anyhow::bail!("{}", out.lines().last().unwrap_or(""));
            }
            Ok(())
        })();
        results.push(match res {
            Ok(()) => ActionResult::ok(
                desc,
                "registered (to undo, delete it on the Startup page under Scheduled tasks)",
            ),
            Err(e) => ActionResult::err(desc, e.to_string()),
        });
    }
    Ok(results)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::platform::MemRegistry;

    fn vars(root: &str, user: &str) -> Vars {
        Vars::from_pairs(vec![
            ("ProgramFiles".into(), format!("{root}\\Program Files")),
            ("USERPROFILE".into(), format!("{root}\\Users\\{user}")),
            (
                "APPDATA".into(),
                format!("{root}\\Users\\{user}\\AppData\\Roaming"),
            ),
            (
                "LOCALAPPDATA".into(),
                format!("{root}\\Users\\{user}\\AppData\\Local"),
            ),
        ])
    }

    #[test]
    fn placeholders_and_rewriting() {
        let a = vars("C:", "ann");
        let b = vars("D:", "bob");
        assert_eq!(
            a.to_placeholder("C:\\Users\\ann\\AppData\\Roaming\\Contoso\\x.ini"),
            "%APPDATA%\\Contoso\\x.ini"
        );
        assert_eq!(
            a.to_placeholder("C:\\Program Files\\Contoso"),
            "%ProgramFiles%\\Contoso"
        );
        assert_eq!(a.to_placeholder("E:\\Games\\x"), "E:\\Games\\x");
        assert_eq!(
            b.from_placeholder("%APPDATA%\\Contoso\\x.ini"),
            "D:\\Users\\bob\\AppData\\Roaming\\Contoso\\x.ini"
        );
        assert_eq!(
            rewrite_text("\"C:\\Program Files\\Contoso\\app.exe\" --data C:\\users\\ANN\\AppData\\Roaming\\Contoso", &a, &b),
            "\"D:\\Program Files\\Contoso\\app.exe\" --data D:\\Users\\bob\\AppData\\Roaming\\Contoso"
        );
    }

    #[test]
    fn export_and_restore_round_trip() {
        let src = crate::util::tempdir().unwrap();
        let dst = crate::util::tempdir().unwrap();
        let s = |p: &Path| p.to_string_lossy().to_string();
        let from = Vars::from_pairs(vec![("ProgramFiles".into(), s(src.path()))]);
        let to = Vars::from_pairs(vec![("ProgramFiles".into(), s(dst.path()))]);

        // A "program" in the source Program Files.
        let app = src.path().join("Contoso");
        std::fs::create_dir_all(app.join("data")).unwrap();
        std::fs::write(app.join("app.exe"), b"binary").unwrap();
        std::fs::write(app.join("data").join("settings.ini"), b"[x]\ny=1").unwrap();
        let reg = MemRegistry::new();
        let key = "HKCU\\Software\\Contoso";
        reg.put(KeyData::new(key).with(RegValue::string("Path", &s(&app))));

        let mut c = Collected::default();
        walk_into(&s(&app), &mut c);
        c.keys.insert(key.into());
        c.programs.push(ProgramInfo {
            name: "Contoso".into(),
            ..Default::default()
        });

        // Export with the source variables (no env dependence).
        let pkg = src.path().join("move.setupsweep");
        let m = export_with(&c, &reg, &pkg, &|_| {}, &from).unwrap();
        assert!(
            m.files.iter().all(|f| f.path.starts_with("%ProgramFiles%")),
            "{:?}",
            m.files
        );

        // Restore on the "new PC".
        let reg2 = MemRegistry::new();
        let backups = crate::util::tempdir().unwrap();
        let mut bw = BackupWriter::new(backups.path(), "Transfer").unwrap();
        let ck = check(&m, &reg2, &to);
        assert!(ck.problems.is_empty(), "{:?}", ck.problems);
        let res = restore_with(&m, &pkg, &reg2, &mut bw, false, &to, &|_| {}).unwrap();
        assert!(res.iter().all(|r| r.ok), "{res:#?}");
        let new_app = dst.path().join("Contoso");
        assert_eq!(std::fs::read(new_app.join("app.exe")).unwrap(), b"binary");
        assert!(new_app.join("data").join("settings.ini").exists());
        let v = reg2.read_key(key).unwrap().get_string("Path").unwrap();
        assert_eq!(v, s(&new_app));

        // Undo the transfer from its backup.
        let b = bw.finish().unwrap();
        let undo = crate::backup::restore_backup(backups.path(), &b.id, &reg2).unwrap();
        assert!(undo.iter().all(|r| r.ok), "{undo:#?}");
        assert!(!new_app.exists());
        assert!(!reg2.exists(key));
    }
}
