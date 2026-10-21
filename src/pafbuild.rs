//! PortableApps Forge: capture a program's installation and turn it into a
//! PortableApps.com Format package, then compile it with the PortableApps.com
//! Launcher Generator and Installer.
//!
//! The capture can run inside Windows Sandbox (the host PC is never touched)
//! or on this PC (the installation is recorded as an install log, so InstallScope
//! can remove it precisely afterwards). In both cases it:
//!
//! 1. takes a snapshot, waits, and takes another to learn background noise;
//! 2. runs the installer (silently when asked);
//! 3. lets the user start the program once and close it, so its first-run
//!    settings are captured;
//! 4. takes a final snapshot and compares;
//! 5. copies the program's files into `App\AppName`, writes `appinfo.ini`,
//!    the Launcher settings, icons and `help.html`.

use crate::model::{ChangeSet, FileEntry, InstallLog, KeyData};
use crate::platform;
use crate::portable::{launcher_ini_for, Folders};
use crate::settings::Settings;
use crate::snapshot::{diff, filter_changes, take_snapshot, Progress};
use anyhow::{anyhow, Context, Result};
use std::path::{Path, PathBuf};

/// Categories allowed in appinfo.ini.
pub const CATEGORIES: [&str; 10] = [
    "Accessibility",
    "Development",
    "Education",
    "Games",
    "Graphics & Pictures",
    "Internet",
    "Music & Video",
    "Office",
    "Security",
    "Utilities",
];

#[derive(Clone, Debug, PartialEq)]
pub struct AppDetails {
    /// The program's name, e.g. "Notepad++". The package is "Notepad++ Portable".
    pub name: String,
    /// The program's publisher.
    pub publisher: String,
    /// Who made the portable package (added to Publisher as "X & you").
    pub packager: String,
    pub version: String,
    pub category: String,
    pub description: String,
    pub homepage: String,
    pub language: String,
    pub open_source: bool,
    pub freeware: bool,
    pub commercial_use: bool,
    pub shareable: bool,
}
crate::json_struct!(AppDetails, default, { name, publisher, packager, version, category, description, homepage, language, open_source, freeware, commercial_use, shareable });

impl Default for AppDetails {
    fn default() -> Self {
        AppDetails {
            name: String::new(),
            publisher: String::new(),
            packager: String::new(),
            version: String::new(),
            category: "Utilities".into(),
            description: String::new(),
            homepage: String::new(),
            language: "Multilingual".into(),
            open_source: false,
            freeware: true,
            commercial_use: true,
            shareable: true,
        }
    }
}

impl AppDetails {
    /// "Notepad++" → "NotepadPlusPlusPortable" (letters, digits, `.`, `-`, `_` only).
    pub fn app_id(&self) -> String {
        let mut id = String::new();
        for c in self.name.chars() {
            match c {
                '+' => id.push_str("Plus"),
                '&' => id.push_str("And"),
                '#' => id.push_str("Sharp"),
                c if c.is_ascii_alphanumeric() || "._-".contains(c) => id.push(c),
                _ => {}
            }
        }
        if id.is_empty() {
            id.push_str("App");
        }
        if !id.ends_with("Portable") {
            id.push_str("Portable");
        }
        id
    }

    /// The folder in `App\` that holds the program's own files.
    pub fn app_dir(&self) -> String {
        let id = self.app_id();
        let dir = id.strip_suffix("Portable").unwrap_or(&id);
        if dir.is_empty() {
            "App".into()
        } else {
            dir.to_string()
        }
    }

    pub fn display_name(&self) -> String {
        let n = self.name.trim();
        if n.to_lowercase().ends_with("portable") {
            n.to_string()
        } else {
            format!("{n} Portable")
        }
    }
}

/// "1.2" → "1.2.0.0"; "v8.6.5 beta" → "8.6.5.0". Always four numbers.
pub fn package_version(v: &str) -> String {
    let nums: Vec<u64> = v
        .split(|c: char| !c.is_ascii_digit())
        .filter(|s| !s.is_empty())
        .take(4)
        .map(|s| s.parse().unwrap_or(0))
        .collect();
    let mut parts: Vec<String> = nums.iter().map(|n| n.to_string()).collect();
    while parts.len() < 4 {
        parts.push("0".into());
    }
    parts.join(".")
}

pub fn appinfo_ini(d: &AppDetails) -> String {
    appinfo_ini_with(d, &crate::portability::Needs::default())
}

pub fn appinfo_ini_with(d: &AppDetails, needs: &crate::portability::Needs) -> String {
    appinfo_ini_with_file_types(d, needs, &[])
}

fn appinfo_ini_with_changes(
    d: &AppDetails,
    needs: &crate::portability::Needs,
    changes: &ChangeSet,
) -> String {
    appinfo_ini_with_associations(
        d,
        needs,
        &registered_file_types(changes),
        &registered_protocols(changes),
    )
}

fn appinfo_ini_with_file_types(
    d: &AppDetails,
    needs: &crate::portability::Needs,
    file_types: &[String],
) -> String {
    appinfo_ini_with_associations(d, needs, file_types, &[])
}

fn appinfo_ini_with_associations(
    d: &AppDetails,
    needs: &crate::portability::Needs,
    file_types: &[String],
    protocols: &[String],
) -> String {
    let name = paf_text(&d.display_name());
    let publisher_name = paf_text(&d.publisher);
    let packager_name = paf_text(&d.packager);
    let publisher = match (publisher_name.as_str(), packager_name.as_str()) {
        ("", "") => "Unknown".to_string(),
        (p, "") => p.to_string(),
        ("", k) => k.to_string(),
        (p, k) => format!("{p} & {k}"),
    };
    let yes_no = |v: bool| if v { "yes" } else { "no" };
    let deps = if needs.java || needs.x64_only || needs.admin {
        format!(
            "[Dependencies]\r\nUsesJava={}\r\nUsesGhostscript=no\r\nRequires64bitOS={}\r\nRequiresAdmin={}\r\n\r\n",
            yes_no(needs.java),
            yes_no(needs.x64_only),
            yes_no(needs.admin)
        )
    } else {
        String::new()
    };
    let associations = if file_types.is_empty() && protocols.is_empty() {
        String::new()
    } else {
        let mut section = String::from("[Associations]\r\n");
        if !file_types.is_empty() {
            section.push_str(&format!("FileTypes={}\r\n", file_types.join(",")));
        }
        if !protocols.is_empty() {
            section.push_str(&format!("Protocols={}\r\n", protocols.join(",")));
        }
        section.push_str("\r\n");
        section
    };
    let b = |v: bool| if v { "true" } else { "false" };
    let category = if CATEGORIES.contains(&d.category.as_str()) {
        d.category.as_str()
    } else {
        "Utilities"
    };
    let description: String = paf_text(&d.description).chars().take(512).collect();
    let display_version = if d.version.trim().is_empty() {
        "1.0"
    } else {
        d.version.trim()
    };
    let display_version = paf_text(display_version);
    let homepage = paf_text(&d.homepage);
    let language = paf_text(if d.language.trim().is_empty() {
        "Multilingual"
    } else {
        d.language.trim()
    });
    format!(
        "[Format]\r\nType=PortableApps.comFormat\r\nVersion=3.9\r\n\r\n\
         [Details]\r\nName={name}\r\nAppID={id}\r\nPublisher={publisher}\r\nHomepage={home}\r\n\
         Category={category}\r\nDescription={description}\r\nLanguage={lang}\r\n\r\n\
         [License]\r\nShareable={sh}\r\nOpenSource={os}\r\nFreeware={fw}\r\nCommercialUse={cu}\r\n\r\n\
         [Version]\r\nPackageVersion={pv}\r\nDisplayVersion={dv}\r\n\r\n\
         {deps}[Control]\r\nIcons=1\r\nStart={id}.exe\r\n\r\n{associations}",
        name = name,
        id = d.app_id(),
        home = homepage,
        lang = language,
        sh = b(d.shareable),
        os = b(d.open_source),
        fw = b(d.freeware),
        cu = b(d.commercial_use),
        pv = package_version(&display_version),
        dv = display_version,
        associations = associations,
    )
}

fn paf_text(value: &str) -> String {
    value
        .chars()
        .map(|c| match c {
            '"' => '\'',
            c if c.is_control() || c.is_whitespace() => ' ',
            c => c,
        })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn registered_file_types(changes: &ChangeSet) -> Vec<String> {
    let mut types = std::collections::BTreeSet::<String>::new();
    let mut add_key = |key: &str| {
        let lower = key.to_ascii_lowercase();
        let ext = ["hkcu\\software\\classes\\", "hklm\\software\\classes\\"]
            .iter()
            .find_map(|root| lower.strip_prefix(root))
            .and_then(|rest| {
                let (ext, child) = rest.split_once('\\').unwrap_or((rest, ""));
                (child.is_empty() && ext.starts_with('.') && ext.len() > 1).then_some(ext)
            });
        if let Some(ext) = ext {
            let value = &ext[1..];
            if value
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"._+-".contains(&b))
            {
                types.insert(value.to_ascii_lowercase());
            }
        }
    };
    for key in &changes.keys_added {
        add_key(&key.path);
    }
    for value in &changes.values_added {
        add_key(&value.key);
    }
    for value in &changes.values_modified {
        add_key(&value.key);
    }
    types.into_iter().collect()
}

fn registered_protocols(changes: &ChangeSet) -> Vec<String> {
    let mut protocols = std::collections::BTreeSet::<String>::new();
    let mut add_key = |key: &str, values: &[crate::model::RegValue]| {
        let lower = key.to_ascii_lowercase();
        let Some(rest) = ["hkcu\\software\\classes\\", "hklm\\software\\classes\\"]
            .iter()
            .find_map(|root| lower.strip_prefix(root))
        else {
            return;
        };
        if rest.contains('\\')
            || !rest
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b".+-".contains(&b))
            || !rest.as_bytes().first().is_some_and(u8::is_ascii_alphabetic)
        {
            return;
        }
        let is_protocol = values.iter().any(|value| {
            value.name.eq_ignore_ascii_case("URL Protocol") && value.as_string().is_some()
        });
        if is_protocol {
            protocols.insert(rest.to_string());
        }
    };
    for key in &changes.keys_added {
        add_key(&key.path, &key.values.values().cloned().collect::<Vec<_>>());
    }
    for value in &changes.values_added {
        add_key(&value.key, std::slice::from_ref(&value.value));
    }
    for value in &changes.values_modified {
        add_key(&value.key, std::slice::from_ref(&value.after));
    }
    protocols.into_iter().collect()
}

// ---- Installers ------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InstallerKind {
    Msi,
    Nsis,
    Inno,
    Other,
}

impl InstallerKind {
    pub fn label(self) -> &'static str {
        match self {
            InstallerKind::Msi => "Windows Installer (MSI)",
            InstallerKind::Nsis => "NSIS",
            InstallerKind::Inno => "Inno Setup",
            InstallerKind::Other => "Unknown installer",
        }
    }
    pub fn can_be_silent(self) -> bool {
        self != InstallerKind::Other
    }
}

pub fn detect_installer(path: &Path) -> InstallerKind {
    let lower = path.to_string_lossy().to_lowercase();
    if lower.ends_with(".msi") {
        return InstallerKind::Msi;
    }
    if crate::silent::is_nsis_file(&path.to_string_lossy()) {
        return InstallerKind::Nsis;
    }
    // Inno Setup stamps its name into the setup loader.
    if let Ok(mut f) = std::fs::File::open(path) {
        use std::io::Read;
        let mut buf = vec![0u8; 4 * 1024 * 1024];
        let n = f.read(&mut buf).unwrap_or(0);
        let hay = &buf[..n];
        let has = |needle: &[u8]| crate::util::find_bytes(hay, needle).is_some();
        if has(b"Inno Setup") || has(b"InnoSetupLdrWindow") {
            return InstallerKind::Inno;
        }
        if has(b"Nullsoft") || has(b"NullsoftInst") {
            return InstallerKind::Nsis;
        }
    }
    InstallerKind::Other
}

/// The command line that runs an installer, silently when possible and asked.
pub fn installer_command(path: &Path, silent: bool) -> String {
    let p = path.to_string_lossy();
    let kind = detect_installer(path);
    match (kind, silent) {
        (InstallerKind::Msi, true) => format!("msiexec.exe /i \"{p}\" /qn /norestart"),
        (InstallerKind::Msi, false) => format!("msiexec.exe /i \"{p}\""),
        (InstallerKind::Nsis, true) => format!("\"{p}\" /S"),
        (InstallerKind::Inno, true) => {
            format!("\"{p}\" /VERYSILENT /SUPPRESSMSGBOXES /NORESTART /SP-")
        }
        _ => format!("\"{p}\""),
    }
}

// ---- Reading the comparison ------------------------------------------------

fn parent(p: &str) -> Option<&str> {
    p.rfind('\\').map(|i| &p[..i])
}

fn leaf(p: &str) -> &str {
    p.rsplit('\\').next().unwrap_or(p)
}

fn inside(p: &str, dir: &str) -> bool {
    p.len() > dir.len()
        && p.as_bytes()[dir.len()] == b'\\'
        && p[..dir.len()].eq_ignore_ascii_case(dir)
}

fn is_exe(p: &str) -> bool {
    p.to_lowercase().ends_with(".exe")
}

/// The folder the program was installed to: the largest newly created folder
/// tree that holds a program file, outside Windows, temp and user settings.
pub fn find_program_dir(cs: &ChangeSet, folders: &Folders) -> Option<String> {
    let new_dirs: Vec<&FileEntry> = cs.files_added.iter().filter(|f| f.is_dir).collect();
    let is_new = |p: &str| new_dirs.iter().any(|d| d.path.eq_ignore_ascii_case(p));
    let windir = std::env::var("windir").unwrap_or_else(|_| "C:\\Windows".into());
    let mut best: Option<(u64, String)> = None;
    for d in &new_dirs {
        if parent(&d.path).is_some_and(is_new) {
            continue;
        }
        if inside(&d.path, &windir) {
            continue;
        }
        let var = folders.to_var(&d.path).unwrap_or_default().to_lowercase();
        // Settings folders are handled by the Launcher; per-user programs
        // (%LOCALAPPDATA%\Programs, or straight in %LOCALAPPDATA%) count.
        if var.starts_with("%temp%")
            || var.starts_with("%appdata%\\microsoft")
            || var.starts_with("%localappdata%\\microsoft")
            || var.starts_with("%allusersappdata%\\package cache")
            || var.starts_with("%allusersappdata%\\microsoft")
            || var.starts_with("%userprofile%\\desktop")
        {
            continue;
        }
        let files = cs
            .files_added
            .iter()
            .filter(|f| !f.is_dir && inside(&f.path, &d.path));
        let (mut bytes, mut has_exe) = (0u64, false);
        for f in files {
            bytes += f.size;
            has_exe |= is_exe(&f.path);
        }
        if has_exe && best.as_ref().is_none_or(|(b, _)| bytes > *b) {
            best = Some((bytes, d.path.clone()));
        }
    }
    best.map(|(_, p)| p)
        .or_else(|| install_location(cs))
        .or_else(|| changed_program_root(cs, &program_roots(folders)))
}

/// The InstallLocation the installer registered, if it exists.
fn install_location(cs: &ChangeSet) -> Option<String> {
    let d = crate::snapshot::detect_program(cs)?;
    let is_loc = |key: &str, name: &str| {
        key.eq_ignore_ascii_case(&d.key) && name.eq_ignore_ascii_case("InstallLocation")
    };
    let loc = cs
        .keys_added
        .iter()
        .find(|k| k.path.eq_ignore_ascii_case(&d.key))
        .and_then(|k| k.get_string("InstallLocation"))
        .or_else(|| {
            cs.values_added
                .iter()
                .find(|v| is_loc(&v.key, &v.value.name))
                .and_then(|v| v.value.as_string())
        })
        .or_else(|| {
            cs.values_modified
                .iter()
                .find(|v| is_loc(&v.key, &v.after.name))
                .and_then(|v| v.after.as_string())
        })?;
    let loc = loc
        .trim()
        .trim_matches('"')
        .trim_end_matches('\\')
        .to_string();
    (!loc.is_empty() && Path::new(&loc).is_dir()).then_some(loc)
}

/// Folders programs are installed into (each program gets a folder inside).
fn program_roots(folders: &Folders) -> Vec<String> {
    let mut roots: Vec<String> = ["ProgramFiles", "ProgramFiles(x86)", "ProgramW6432"]
        .iter()
        .filter_map(|k| std::env::var(k).ok())
        .collect();
    for (real, var) in &folders.map {
        if var == "%LOCALAPPDATA%" {
            roots.push(format!("{real}\\Programs"));
            roots.push(real.clone());
        }
    }
    roots.push("C:\\Program Files".into());
    roots.push("C:\\Program Files (x86)".into());
    roots
}

/// When the program's folder already existed (an upgrade, or a program that
/// was already installed): the folder under a program root whose program
/// files changed the most.
fn changed_program_root(cs: &ChangeSet, roots: &[String]) -> Option<String> {
    let mut totals: Vec<(String, u64)> = Vec::new();
    let changed = cs
        .files_added
        .iter()
        .filter(|f| !f.is_dir)
        .chain(cs.files_modified.iter().map(|m| &m.after));
    for f in changed {
        let Some(root) = roots.iter().find(|r| inside(&f.path, r)) else {
            continue;
        };
        let rest = &f.path[root.len() + 1..];
        let Some((top, _)) = rest.split_once('\\') else {
            continue;
        };
        let dir = format!("{root}\\{top}");
        // Only folders that hold a changed program file count.
        let weight = f.size + if is_exe(&f.path) { 1 << 40 } else { 0 };
        match totals
            .iter_mut()
            .find(|(d, _)| d.eq_ignore_ascii_case(&dir))
        {
            Some((_, t)) => *t += weight,
            None => totals.push((dir, weight)),
        }
    }
    totals
        .into_iter()
        .filter(|(_, t)| *t >= 1 << 40)
        .max_by_key(|(_, t)| *t)
        .map(|(d, _)| d)
}

/// The program's main executable inside `program_dir`: the target of a new
/// Start menu or desktop shortcut when there is one, otherwise the program
/// file whose name best matches the app's name.
pub fn find_main_exe(cs: &ChangeSet, program_dir: &str, app_name: &str) -> Option<String> {
    let bad = [
        "unins",
        "uninst",
        "update",
        "crash",
        "report",
        "helper",
        "setup",
        "install",
        "elevate",
        "vc_redist",
    ];
    let looks_bad = |p: &str| {
        let l = leaf(p).to_lowercase();
        bad.iter().any(|b| l.contains(b))
    };
    for f in &cs.files_added {
        if f.is_dir || !f.path.to_lowercase().ends_with(".lnk") {
            continue;
        }
        let target = platform::resolve_shortcut(&f.path);
        if inside(&target, program_dir) && is_exe(&target) && !looks_bad(&target) {
            return Some(target);
        }
    }
    let name: String = app_name
        .to_lowercase()
        .chars()
        .filter(|c| c.is_alphanumeric())
        .collect();
    let depth = |p: &str| p.matches('\\').count();
    let mut candidates: Vec<FileEntry> = cs
        .files_added
        .iter()
        .chain(cs.files_modified.iter().map(|m| &m.after))
        .filter(|f| {
            !f.is_dir && is_exe(&f.path) && inside(&f.path, program_dir) && !looks_bad(&f.path)
        })
        .cloned()
        .collect();
    if candidates.is_empty() {
        // The folder came from InstallLocation: look at what is on disk.
        for e in crate::fswalk::WalkDir::new(program_dir)
            .max_depth(3)
            .into_iter()
            .filter_map(|e| e.ok())
        {
            let p = e.path().to_string_lossy().to_string();
            if e.file_type().is_file() && is_exe(&p) && !looks_bad(&p) {
                let size = e.metadata().map(|m| m.len()).unwrap_or(0);
                candidates.push(FileEntry {
                    path: p,
                    is_dir: false,
                    size,
                    mtime: 0,
                });
            }
        }
    }
    candidates
        .iter()
        .max_by_key(|f| {
            let stem: String = leaf(&f.path)
                .to_lowercase()
                .trim_end_matches(".exe")
                .chars()
                .filter(|c| c.is_alphanumeric())
                .collect();
            let matches = !name.is_empty()
                && !stem.is_empty()
                && (name.contains(&stem) || stem.contains(&name));
            (matches, std::cmp::Reverse(depth(&f.path)), f.size)
        })
        .map(|f| f.path.clone())
}

// ---- Writing the package ---------------------------------------------------

fn copy_tree(from: &Path, to: &Path) -> Result<(u64, u64)> {
    let (mut files, mut bytes) = (0, 0);
    for e in crate::fswalk::WalkDir::new(from).follow_links(false) {
        let e = e?;
        let rel = e.path().strip_prefix(from)?;
        let dest = to.join(rel);
        if e.file_type().is_dir() {
            std::fs::create_dir_all(&dest)?;
        } else if e.file_type().is_file() {
            if let Some(p) = dest.parent() {
                std::fs::create_dir_all(p)?;
            }
            bytes += std::fs::copy(e.path(), &dest)
                .with_context(|| format!("copy {}", e.path().display()))?;
            files += 1;
        }
    }
    Ok((files, bytes))
}

fn png(w: u32, h: u32, rgba: Vec<u8>) -> Option<Vec<u8>> {
    crate::png::encode_rgba(w, h, &rgba)
}

/// An .ico file holding PNG images (supported by every Windows since Vista).
pub fn ico_from_pngs(images: &[(u32, Vec<u8>)]) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(&[0, 0, 1, 0]);
    out.extend_from_slice(&(images.len() as u16).to_le_bytes());
    let mut offset = 6 + 16 * images.len() as u32;
    for (size, data) in images {
        let s = if *size >= 256 { 0 } else { *size as u8 };
        out.extend_from_slice(&[s, s, 0, 0]);
        out.extend_from_slice(&1u16.to_le_bytes());
        out.extend_from_slice(&32u16.to_le_bytes());
        out.extend_from_slice(&(data.len() as u32).to_le_bytes());
        out.extend_from_slice(&offset.to_le_bytes());
        offset += data.len() as u32;
    }
    for (_, data) in images {
        out.extend_from_slice(data);
    }
    out
}

/// Write appicon.ico and the PNG sizes the format asks for.
fn write_icons(exe: &str, appinfo: &Path) -> bool {
    let mut ico = Vec::new();
    let mut any = false;
    for size in [16u32, 32, 48, 75, 128, 256] {
        let Some((w, h, rgba)) = platform::icon_rgba(exe, 0, size) else {
            continue;
        };
        let Some(data) = png(w, h, rgba) else {
            continue;
        };
        if size != 48 {
            let _ = std::fs::write(appinfo.join(format!("appicon_{size}.png")), &data);
        }
        if matches!(size, 16 | 32 | 48 | 256) {
            ico.push((size, data));
        }
        any = true;
    }
    if !ico.is_empty() {
        let _ = std::fs::write(appinfo.join("appicon.ico"), ico_from_pngs(&ico));
    }
    any
}

/// A .reg file (UTF-16 with BOM, as the Launcher expects) for keys under `root`.
fn reg_file(keys: &[KeyData], root: &str) -> Vec<u8> {
    let mut s = String::from("Windows Registry Editor Version 5.00\r\n\r\n");
    let full = |p: &str| {
        p.replacen("HKCU\\", "HKEY_CURRENT_USER\\", 1)
            .replacen("HKLM\\", "HKEY_LOCAL_MACHINE\\", 1)
    };
    for k in keys
        .iter()
        .filter(|k| k.path.eq_ignore_ascii_case(root) || inside(&k.path, root))
    {
        s.push_str(&format!("[{}]\r\n", full(&k.path)));
        for v in k.values.values() {
            s.push_str(&format!(
                "{}={}\r\n",
                crate::report::reg_name(&v.name),
                crate::report::reg_data(v)
            ));
        }
        s.push_str("\r\n");
    }
    let mut bytes = vec![0xFF, 0xFE];
    for u in s.encode_utf16() {
        bytes.extend_from_slice(&u.to_le_bytes());
    }
    bytes
}

fn help_html(d: &AppDetails) -> String {
    let h = |s: &str| {
        s.replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
            .replace('"', "&quot;")
            .replace('\'', "&#39;")
    };
    let homepage = d.homepage.trim();
    let safe_homepage = if (homepage
        .get(..8)
        .is_some_and(|scheme| scheme.eq_ignore_ascii_case("https://"))
        || homepage
            .get(..7)
            .is_some_and(|scheme| scheme.eq_ignore_ascii_case("http://")))
        && homepage
            .chars()
            .all(|c| !c.is_control() && !c.is_whitespace())
    {
        Some(homepage)
    } else {
        None
    };
    format!(
        "<!DOCTYPE html>\n<html lang=\"en\"><head><meta charset=\"utf-8\"><title>{name}</title>\
         <style>body{{font-family:Segoe UI,Arial,sans-serif;max-width:720px;margin:2em auto;line-height:1.5;padding:0 1em}}</style></head>\n\
         <body><h1>{name}</h1>\n<p>{desc}</p>\n\
         <p>{name} is {base} packaged in the PortableApps.com Format, so it can run from a USB drive, \
         cloud folder or any other location and keeps its settings with it.</p>\n\
         <h2>Where things are</h2><ul><li><b>App</b> holds the program. Don't change it.</li>\
         <li><b>Data</b> holds your settings. Back it up.</li><li><b>Other</b> holds this help and the source.</li></ul>\n\
         {home}</body></html>\n",
        name = h(&d.display_name()),
        base = h(d.name.trim()),
        desc = h(d.description.trim()),
        home = safe_homepage
            .map(|url| format!("<p><a href=\"{0}\">{0}</a></p>\n", h(url)))
            .unwrap_or_default(),
    )
}

/// What `build_package` made.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Package {
    pub dir: String,
    pub app_id: String,
    pub program_dir: String,
    pub main_exe: String,
    pub files: u64,
    pub bytes: u64,
    pub launcher_ini: String,
    /// Things the user should check before publishing.
    pub warnings: Vec<String>,
    /// What could stop the program from working portably.
    pub portability: crate::portability::Report,
}
crate::json_struct!(Package, { dir, app_id, program_dir, main_exe, files, bytes, launcher_ini, warnings, portability [default] });

/// Build a PortableApps.com Format folder in `out_dir\<AppID>` from a
/// captured installation. The program's files must still be on disk.
pub fn build_package(
    cs: &ChangeSet,
    d: &AppDetails,
    out_dir: &Path,
    include_defaults: bool,
    folders: &Folders,
    progress: &dyn Fn(&str),
) -> Result<Package> {
    let id = d.app_id();
    let pkg = out_dir.join(&id);
    if pkg.exists() {
        return Err(anyhow!(
            "{} already exists; choose another folder or remove it first",
            pkg.display()
        ));
    }
    let program_dir = find_program_dir(cs, folders)
        .ok_or_else(|| {
            let st = cs.stats();
            anyhow!(
                "no program files were found in the captured changes ({} files and {} folders added, {} files changed). Did the installer finish, and was it installed for this user or all users?",
                st.files_added,
                st.dirs_added,
                st.files_modified
            )
        })?;
    let exe = find_main_exe(cs, &program_dir, &d.name)
        .ok_or_else(|| anyhow!("no program file (.exe) was found in {program_dir}"))?;
    let mut warnings = Vec::new();

    progress("Copying the program files");
    let app = pkg.join("App");
    let appinfo = app.join("AppInfo");
    std::fs::create_dir_all(appinfo.join("Launcher"))?;
    std::fs::create_dir_all(pkg.join("Other").join("Source"))?;
    let app_dir = d.app_dir();
    let (files, bytes) = copy_tree(Path::new(&program_dir), &app.join(&app_dir))?;

    progress("Checking how well the program works portably");
    let report = crate::portability::analyze(cs, Path::new(&program_dir), Path::new(&exe));
    // Visual C++ runtimes go next to the program (app-local deployment).
    let exe_dir_in_app = app.join(&app_dir).join(
        Path::new(exe[program_dir.len()..].trim_start_matches('\\'))
            .parent()
            .unwrap_or(Path::new("")),
    );
    for dll in &report.needs.runtime_dlls {
        let to = exe_dir_in_app.join(leaf(dll));
        if !to.exists() {
            if let Err(e) = std::fs::copy(dll, &to) {
                warnings.push(format!("Could not copy {dll}: {e}"));
            }
        }
    }

    progress("Writing the PortableApps.com files");
    std::fs::write(
        appinfo.join("appinfo.ini"),
        appinfo_ini_with_changes(d, &report.needs, cs),
    )?;
    if !write_icons(&exe, &appinfo) {
        warnings.push("No icon could be read from the program; add appicon.ico and the PNG icons to App\\AppInfo.".into());
    }
    let rel_exe = format!(
        "{app_dir}\\{}",
        exe[program_dir.len()..].trim_start_matches('\\')
    );
    let settings = launcher_ini_for(
        cs,
        &d.display_name(),
        folders,
        std::slice::from_ref(&program_dir),
    );
    let launcher = format!(
        "; Made by InstallScope PortableApps Forge. Review the sections below before publishing.\r\n\r\n\
         {java}[Launch]\r\nProgramExecutable={rel_exe}\r\nDirectoryMoveOK=yes\r\nSupportsUNC=yes\r\n\r\n{}{}{issues}",
        settings.sections.replace("\r\n", "\n").replace('\n', "\r\n"),
        settings.notes.replace("\r\n", "\n").replace('\n', "\r\n"),
        java = if report.needs.java { "[Activate]\r\nJava=require\r\n\r\n" } else { "" },
        issues = report
            .issues
            .iter()
            .filter(|i| i.severity > crate::portability::Severity::Info)
            .map(|i| format!("; {}: {}. {}\r\n", i.severity.label(), i.title, i.detail))
            .collect::<String>(),
    );
    std::fs::write(
        appinfo.join("Launcher").join(format!("{id}.ini")),
        &launcher,
    )?;
    std::fs::write(pkg.join("help.html"), help_html(d))?;
    std::fs::write(
        pkg.join("Other").join("Source").join("Readme.txt"),
        format!(
            "{} was captured and packaged by InstallScope (https://github.com/SeanShaffer88/InstallScope).\r\n\r\n\
             The Launcher settings are in App\\AppInfo\\Launcher\\{id}.ini.\r\n\r\n\
             Portability check:\r\n{}",
            d.display_name(),
            report
                .issues
                .iter()
                .map(|i| format!("- {}: {}. {}\r\n", i.severity.label(), i.title, i.detail))
                .collect::<String>()
        ),
    )?;

    if include_defaults {
        progress("Copying the first-run settings");
        let dd = app.join("DefaultData");
        for (key, real) in &settings.dir_moves {
            if let Err(e) = copy_tree(Path::new(real), &dd.join(key)) {
                warnings.push(format!("Could not copy {real}: {e}"));
            }
        }
        if !settings.file_moves.is_empty() || !settings.reg_moves.is_empty() {
            std::fs::create_dir_all(dd.join("settings"))?;
        }
        for f in &settings.file_moves {
            if let Err(e) = std::fs::copy(f, dd.join("settings").join(leaf(f))) {
                warnings.push(format!("Could not copy {f}: {e}"));
            }
        }
        for (name, key) in &settings.reg_moves {
            std::fs::write(
                dd.join("settings").join(format!("{name}.reg")),
                reg_file(&cs.keys_added, key),
            )?;
        }
        if !settings.dir_moves.is_empty() || !settings.reg_moves.is_empty() {
            warnings.push("The default settings were captured on this PC; check them for user names or paths.".into());
        }
    }
    if settings.text.contains("HKLM\\") {
        warnings.push("The program writes machine-wide registry keys; see the review notes in the Launcher settings.".into());
    }
    let other_dirs: Vec<String> = cs
        .files_added
        .iter()
        .filter(|f| {
            f.is_dir && !f.path.eq_ignore_ascii_case(&program_dir) && !inside(&f.path, &program_dir)
        })
        .filter(|f| {
            folders.to_var(&f.path).is_none()
                && parent(&f.path).is_some_and(|p| {
                    !cs.files_added
                        .iter()
                        .any(|x| x.is_dir && x.path.eq_ignore_ascii_case(p))
                })
        })
        .map(|f| f.path.clone())
        .collect();
    if !other_dirs.is_empty() {
        warnings.push(format!(
            "The installer also created {}; check whether the program needs it.",
            other_dirs.join(", ")
        ));
    }
    Ok(Package {
        dir: pkg.to_string_lossy().to_string(),
        app_id: id,
        program_dir,
        main_exe: exe,
        files,
        bytes,
        launcher_ini: launcher,
        warnings,
        portability: report,
    })
}

// ---- Capture --------------------------------------------------------------

/// Everything a capture needs, saved as job.json (so the same run works
/// inside Windows Sandbox, where it is started by Windows, not by InstallScope).
#[derive(Clone, Debug, Default)]
pub struct Job {
    pub details: AppDetails,
    pub installer: String,
    pub silent: bool,
    pub out_dir: String,
    pub include_defaults: bool,
    /// Seconds to watch for background activity before installing.
    pub learn_seconds: u64,
    /// Running inside Windows Sandbox: ask with message boxes, shut down at the end.
    pub sandbox: bool,
    /// Only record what the installer does (a preview); build no package.
    pub preview_only: bool,
    /// The host's capture settings (roots and exclusions).
    pub settings: Settings,
}
crate::json_struct!(Job, default, { details, installer, silent, out_dir, include_defaults, learn_seconds, sandbox, preview_only, settings });

/// Progress of a capture, written to status.json next to job.json.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Status {
    pub phase: String,
    pub done: bool,
    pub error: Option<String>,
    pub package: Option<Package>,
}
crate::json_struct!(Status, default, { phase, done, error, package });

/// Snapshot, install, snapshot, build. `ask_done` blocks until the user has
/// finished installing and has run and closed the program once.
pub fn capture(
    job: &Job,
    reg: &dyn crate::platform::Registry,
    report: &dyn Fn(&str),
    ask_done: &dyn Fn(),
    progress: &Progress,
) -> Result<(InstallLog, Package)> {
    let settings = &job.settings;
    report("Taking the first snapshot");
    let a = take_snapshot(reg, settings, progress)?;
    let mut learned = crate::noise::Learned::default();
    let before = if job.learn_seconds > 0 {
        report("Learning what Windows changes on its own");
        std::thread::sleep(std::time::Duration::from_secs(job.learn_seconds));
        let b = take_snapshot(reg, settings, progress)?;
        let roots: Vec<String> = settings.file_roots.iter().map(|r| r.path.clone()).collect();
        learned = crate::noise::learn(
            &diff(&a, &b),
            &roots,
            &settings.registry_roots,
            &settings.effective_file_exclusions(),
            &settings.effective_registry_exclusions(),
        );
        b
    } else {
        a
    };
    let installer = PathBuf::from(&job.installer);
    let cmd = installer_command(&installer, job.silent);
    report("Installing");
    let child = platform::spawn_command_line(&cmd)?;
    let code = child.wait();
    if job.silent && code != 0 && code != 3010 && code != 1641 {
        return Err(anyhow!("the installer stopped with error code {code}"));
    }
    report("Waiting for you to run the program once and close it");
    ask_done();
    report("Taking the second snapshot");
    let after = take_snapshot(reg, settings, progress)?;
    let mut cs = diff(&before, &after);
    filter_changes(&mut cs, &learned.registry, &learned.files);
    let mut log = InstallLog {
        id: crate::util::new_id(),
        name: job.details.name.clone(),
        publisher: job.details.publisher.clone(),
        version: job.details.version.clone(),
        created: crate::util::now_string(),
        installer: job.installer.clone(),
        tags: vec!["portable-build".into()],
        ..Default::default()
    };
    if let Some(d) = crate::snapshot::detect_program(&cs) {
        log.uninstall_key = d.key;
        if log.version.is_empty() {
            log.version = d.version;
        }
        if log.publisher.is_empty() {
            log.publisher = d.publisher;
        }
    }
    log.changes = cs;
    if job.preview_only {
        return Ok((log, Package::default()));
    }
    let mut details = job.details.clone();
    if details.version.is_empty() {
        details.version = log.version.clone();
    }
    if details.publisher.is_empty() {
        details.publisher = log.publisher.clone();
    }
    let pkg = build_package(
        &log.changes,
        &details,
        Path::new(&job.out_dir),
        job.include_defaults,
        &Folders::current(),
        report,
    )?;
    Ok((log, pkg))
}

/// `installscope pa-capture <job.json>`: the capture as run inside the sandbox
/// (or on this PC from the command line). Writes status.json and
/// install-log.json next to job.json.
pub fn run_job_file(job_path: &Path) -> Result<()> {
    let job: Job = crate::json::compat::from_slice(&std::fs::read(job_path)?)?;
    let dir = job_path.parent().unwrap_or(Path::new(".")).to_path_buf();
    let status_path = dir.join("status.json");
    let write = |s: &Status| {
        let _ = std::fs::write(
            &status_path,
            crate::json::compat::to_vec_pretty(s).unwrap_or_default(),
        );
    };
    let report = |phase: &str| {
        eprintln!("{phase}…");
        write(&Status {
            phase: phase.to_string(),
            ..Default::default()
        });
    };
    let ask = || {
        let msg = "When the installation has finished, start the program once, set it up the way you want it to start, and close it.\n\nThen click OK.";
        platform::ask_ok("InstallScope PortableApps Forge", msg);
    };
    let reg = platform::system_registry();
    let progress = Progress::default();
    let result = capture(&job, reg.as_ref(), &report, &ask, &progress);
    match result {
        Ok((log, pkg)) => {
            let _ = std::fs::write(
                dir.join("install-log.json"),
                crate::json::compat::to_vec(&log)?,
            );
            write(&Status {
                phase: "Done".into(),
                done: true,
                error: None,
                package: Some(pkg),
            });
        }
        Err(e) => {
            write(&Status {
                phase: "Failed".into(),
                done: true,
                error: Some(e.to_string()),
                package: None,
            });
            if job.sandbox {
                platform::ask_ok(
                    "InstallScope PortableApps Forge",
                    &format!("The capture failed: {e}"),
                );
            }
        }
    }
    if job.sandbox {
        // Closing the sandbox throws away everything the installer did.
        let _ = platform::spawn_command_line("shutdown.exe /s /t 3");
    }
    Ok(())
}

// ---- Windows Sandbox ------------------------------------------------------

/// Windows Sandbox is installed (Windows 11 Pro, Enterprise or Education,
/// with the "Windows Sandbox" optional feature turned on).
pub fn sandbox_available() -> bool {
    let windir = std::env::var("windir").unwrap_or_else(|_| "C:\\Windows".into());
    Path::new(&windir)
        .join("System32")
        .join("WindowsSandbox.exe")
        .exists()
}

/// Where things appear inside the sandbox.
pub const SANDBOX_INSTALLSCOPE: &str = "C:\\InstallScope";
pub const SANDBOX_INSTALLER: &str = "C:\\Installer";
pub const SANDBOX_WORK: &str = "C:\\InstallScopeWork";

fn xml(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

pub fn wsb_config(
    installscope_dir: &Path,
    installer_dir: &Path,
    work_dir: &Path,
    exe_name: &str,
) -> String {
    let map = |host: &Path, sandbox: &str, ro: bool| {
        format!(
            "    <MappedFolder>\n      <HostFolder>{}</HostFolder>\n      <SandboxFolder>{sandbox}</SandboxFolder>\n      <ReadOnly>{ro}</ReadOnly>\n    </MappedFolder>\n",
            xml(&host.to_string_lossy())
        )
    };
    format!(
        "<Configuration>\n  <Networking>Default</Networking>\n  <ClipboardRedirection>Enable</ClipboardRedirection>\n  <MemoryInMB>4096</MemoryInMB>\n  <MappedFolders>\n{}{}{}  </MappedFolders>\n  <LogonCommand>\n    <Command>{SANDBOX_INSTALLSCOPE}\\{} pa-capture {SANDBOX_WORK}\\job.json</Command>\n  </LogonCommand>\n</Configuration>\n",
        map(installscope_dir, SANDBOX_INSTALLSCOPE, true),
        map(installer_dir, SANDBOX_INSTALLER, true),
        map(work_dir, SANDBOX_WORK, false),
        xml(exe_name),
    )
}

/// Prepare a work folder for a sandbox capture and start the sandbox.
/// Progress then appears in `work_dir\status.json`.
pub fn start_sandbox(job: &Job, work_dir: &Path) -> Result<()> {
    if !sandbox_available() {
        return Err(anyhow!("Windows Sandbox isn't turned on"));
    }
    std::fs::create_dir_all(work_dir.join("out"))?;
    let exe = std::env::current_exe()?;
    let exe_dir = exe.parent().ok_or_else(|| anyhow!("no program folder"))?;
    let exe_name = exe
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "InstallScope.exe".into());
    let installer = PathBuf::from(&job.installer);
    let installer_dir = installer
        .parent()
        .ok_or_else(|| anyhow!("no installer folder"))?;
    let installer_name = installer
        .file_name()
        .ok_or_else(|| anyhow!("no installer file"))?;
    let mut inside_job = job.clone();
    inside_job.installer = format!("{SANDBOX_INSTALLER}\\{}", installer_name.to_string_lossy());
    inside_job.out_dir = format!("{SANDBOX_WORK}\\out");
    inside_job.sandbox = true;
    // Don't capture InstallScope's own folders inside the sandbox.
    for d in [SANDBOX_INSTALLSCOPE, SANDBOX_INSTALLER, SANDBOX_WORK] {
        inside_job.settings.file_exclusions.push(d.to_string());
    }
    std::fs::write(
        work_dir.join("job.json"),
        crate::json::compat::to_vec_pretty(&inside_job)?,
    )?;
    let _ = std::fs::remove_file(work_dir.join("status.json"));
    let wsb = work_dir.join("InstallScope.wsb");
    std::fs::write(
        &wsb,
        wsb_config(exe_dir, installer_dir, work_dir, &exe_name),
    )?;
    platform::shell_open(&wsb.to_string_lossy());
    Ok(())
}

pub fn read_status(work_dir: &Path) -> Option<Status> {
    crate::json::compat::from_slice(&std::fs::read(work_dir.join("status.json")).ok()?).ok()
}

/// After a sandbox capture: move the package from the work folder to the
/// chosen output folder. Returns its new location.
pub fn collect_sandbox_package(work_dir: &Path, pkg: &Package, out_dir: &Path) -> Result<PathBuf> {
    let from = work_dir.join("out").join(&pkg.app_id);
    let mut to = out_dir.join(&pkg.app_id);
    let mut n = 2;
    while to.exists() {
        to = out_dir.join(format!("{} ({n})", pkg.app_id));
        n += 1;
    }
    std::fs::create_dir_all(out_dir)?;
    if std::fs::rename(&from, &to).is_err() {
        copy_tree(&from, &to)?;
        let _ = std::fs::remove_dir_all(&from);
    }
    Ok(to)
}

// ---- Compiling with the PortableApps.com tools ------------------------------

#[derive(Clone, Debug, Default, PartialEq)]
pub struct PaTools {
    pub launcher_generator: Option<PathBuf>,
    pub installer: Option<PathBuf>,
    pub app_compactor: Option<PathBuf>,
}

pub const LAUNCHER_URL: &str =
    "https://portableapps.com/apps/development/portableapps.com_launcher";
pub const INSTALLER_URL: &str =
    "https://portableapps.com/apps/development/portableapps.com_installer";
pub const APP_COMPACTOR_URL: &str =
    "https://portableapps.com/apps/utilities/portableapps.com_appcompactor";

/// Look for the PortableApps.com Launcher and Installer in `portableapps_dir`
/// (a PortableApps folder), then in X:\PortableApps on every drive and in the
/// user's profile.
pub fn find_tools(portableapps_dir: &str) -> PaTools {
    let mut bases: Vec<PathBuf> = Vec::new();
    if !portableapps_dir.trim().is_empty() {
        bases.push(PathBuf::from(portableapps_dir.trim()));
    }
    for l in b'A'..=b'Z' {
        bases.push(PathBuf::from(format!("{}:\\PortableApps", l as char)));
    }
    if let Ok(p) = std::env::var("USERPROFILE") {
        bases.push(Path::new(&p).join("PortableApps"));
    }
    find_tools_in(bases)
}

fn find_tools_in(bases: impl IntoIterator<Item = PathBuf>) -> PaTools {
    let mut t = PaTools::default();
    for b in bases {
        let g = b
            .join("PortableApps.comLauncher")
            .join("PortableApps.comLauncherGenerator.exe");
        if t.launcher_generator.is_none() && g.is_file() {
            t.launcher_generator = Some(g);
        }
        let i = b
            .join("PortableApps.comInstaller")
            .join("PortableApps.comInstaller.exe");
        if t.installer.is_none() && i.is_file() {
            t.installer = Some(i);
        }
        let c = b
            .join("PortableApps.comAppCompactor")
            .join("PortableApps.comAppCompactor.exe");
        if t.app_compactor.is_none() && c.is_file() {
            t.app_compactor = Some(c);
        }
        if t.launcher_generator.is_some() && t.installer.is_some() {
            break;
        }
    }
    t
}

#[derive(Clone, Debug, Default)]
pub struct Compiled {
    pub launcher: Option<PathBuf>,
    pub paf: Option<PathBuf>,
}

/// Legacy source-build compatibility API; the desktop uses
/// [`compile_official_with_cancel`] instead. This path can acquire the pinned
/// development toolchain and use the InstallScope installer fallback.
///
/// Run the Launcher Generator (makes `AppID.exe`) and then the Installer
/// (makes `AppID_version.paf.exe` next to the package folder). Official frontends
/// receive the package directory; tool-specific UI may still appear.
pub fn compile(
    pkg_dir: &Path,
    app_id: &str,
    tools: &PaTools,
    data_root: &Path,
    report: &dyn Fn(&str),
) -> Result<Compiled> {
    compile_with_cancel(pkg_dir, app_id, tools, data_root, report, &|| false)
}

/// Compile with cooperative cancellation, including the running compiler tree on Windows.
pub fn compile_with_cancel(
    pkg_dir: &Path,
    app_id: &str,
    tools: &PaTools,
    data_root: &Path,
    report: &dyn Fn(&str),
    cancel: &dyn Fn() -> bool,
) -> Result<Compiled> {
    if cancel() {
        return Err(anyhow!("Build cancelled"));
    }
    portable_builder_kit::validate_app_id(app_id)?;
    let metadata_id = portable_builder_kit::package_app_id(pkg_dir)?;
    if metadata_id != app_id {
        return Err(anyhow!(
            "package AppID does not match the requested launcher"
        ));
    }
    let mut out = Compiled::default();
    // The launcher: compiled from the official PortableApps.com Launcher
    // source, which InstallScope carries (or fetches once) with its compiler.
    let toolchain = crate::patools::ensure_with_cancel(data_root, report, cancel);
    if cancel() {
        return Err(anyhow!("Build cancelled"));
    }
    match (&toolchain, &tools.launcher_generator) {
        (Ok(tc), _) => {
            report("Making the launcher");
            out.launcher = Some(crate::patools::build_launcher_with_cancel(
                tc, pkg_dir, cancel,
            )?);
        }
        (Err(_), Some(g)) => {
            report("Making the launcher");
            let exe = pkg_dir.join(format!("{app_id}.exe"));
            if exe.try_exists()? {
                std::fs::remove_file(&exe).context("close the old launcher and try again")?;
            }
            run_package_tool(g, pkg_dir, cancel)?;
            let exe = pkg_dir.join(format!("{app_id}.exe"));
            if !exe.is_file() {
                return Err(anyhow!("the Launcher Generator didn't make {app_id}.exe"));
            }
            portable_builder_kit::validate_executable(&exe)?;
            out.launcher = Some(exe);
        }
        (Err(e), None) => return Err(anyhow!("{e}")),
    }
    // The installer: the official PortableApps.com Installer when it is
    // installed (needed only to publish on portableapps.com), otherwise
    // InstallScope's own.
    report("Making the installer");
    if let Some(i) = &tools.installer {
        let parent = pkg_dir.parent().unwrap_or(pkg_dir);
        let hash = |path: &Path| -> std::io::Result<[u8; 32]> {
            crate::util::sha256_reader(std::fs::File::open(path)?)
        };
        let watch = portable_builder_kit::ArtifactWatch::capture(
            &[parent.to_path_buf(), pkg_dir.to_path_buf()],
            app_id,
            &hash,
        )?;
        let outcome = run_package_tool(i, pkg_dir, cancel)?;
        out.paf = Some(watch.finish(outcome.exit_code, &hash)?.path);
    } else {
        let tc = toolchain.map_err(|e| anyhow!("{e}"))?;
        let work = data_root.join("tools").join("work");
        out.paf = Some(crate::patools::build_installer_with_cancel(
            &tc, pkg_dir, &work, cancel,
        )?);
    }
    Ok(out)
}

/// Custom entry points recognized by the official-tool documentation, including
/// the legacy Launcher path. Presence is a consent gate, not a complete script audit.
pub fn official_custom_code(pkg_dir: &Path) -> [PathBuf; 3] {
    [
        "App/AppInfo/Launcher/Custom.nsh",
        "Other/Source/PortableApps.comInstallerCustom.nsh",
        "Other/Source/PortableApps.comLauncherCustom.nsh",
    ]
    .map(|p| pkg_dir.join(p))
}

/// Official frontends only. Never downloads tools or substitutes a compiler.
/// The caller supplies a prepared working package, not original application files.
/// Tool interfaces may display their own UI. No official approval is implied.
pub fn compile_official_with_cancel(
    pkg_dir: &Path,
    app_id: &str,
    tools: &PaTools,
    allow_custom_code: bool,
    report: &dyn Fn(&str),
    cancel: &dyn Fn() -> bool,
) -> Result<Compiled> {
    if cancel() {
        return Err(anyhow!("Build cancelled"));
    }
    if !allow_custom_code {
        for path in official_custom_code(pkg_dir) {
            if path.try_exists()? {
                return Err(anyhow!("Custom NSIS code was found. Review it and explicitly allow it for this build before executing the official tools."));
            }
        }
    }
    let generator=tools.launcher_generator.as_ref().ok_or_else(||anyhow!("Locate the official PortableApps.com Launcher Generator before building. No fallback tool was used."))?;
    let installer=tools.installer.as_ref().ok_or_else(||anyhow!("Locate the official PortableApps.com Installer before building. No fallback tool was used."))?;
    if !generator.is_file() || !installer.is_file() {
        return Err(anyhow!(
            "An official tool path is unavailable. Locate the installed tools and try again."
        ));
    }
    portable_builder_kit::validate_app_id(app_id)?;
    if portable_builder_kit::package_app_id(pkg_dir)? != app_id {
        return Err(anyhow!(
            "Package AppID does not match the requested launcher"
        ));
    }
    let hash = |path: &Path| -> std::io::Result<[u8; 32]> {
        crate::util::sha256_reader(std::fs::File::open(path)?)
    };
    let launcher = pkg_dir.join(format!("{app_id}.exe"));
    let stamp = |path: &Path| -> Result<Option<(std::time::SystemTime, [u8; 32])>> {
        if !path.try_exists()? {
            return Ok(None);
        }
        portable_builder_kit::validate_executable(path)?;
        Ok(Some((std::fs::metadata(path)?.modified()?, hash(path)?)))
    };
    let before = stamp(&launcher)?;
    report("Running the official Launcher Generator");
    run_package_tool(generator, pkg_dir, cancel)?;
    let after = stamp(&launcher)?;
    if after.is_none() || after == before {
        return Err(anyhow!("Could not verify a newly created or updated launcher. Review the official Generator's messages; the previous output was not accepted as a new build."));
    }
    let parent = pkg_dir.parent().unwrap_or(pkg_dir);
    let watch = portable_builder_kit::ArtifactWatch::capture(
        &[parent.to_path_buf(), pkg_dir.to_path_buf()],
        app_id,
        &hash,
    )?;
    report("Running the official PortableApps.com Installer");
    let outcome = run_package_tool(installer, pkg_dir, cancel)?;
    let artifact = watch.finish(outcome.exit_code, &hash)?;
    Ok(Compiled {
        launcher: Some(launcher),
        paf: Some(artifact.path),
    })
}

/// Exact shell-free requests used by the official frontend adapter. Read-only.
/// Tool selection is user supplied; PE checks do not authenticate the publisher.
pub fn official_build_requests(
    package: &Path,
    tools: &PaTools,
) -> Result<[portable_builder_kit::process::ToolRequest; 2]> {
    let generator = tools
        .launcher_generator
        .as_ref()
        .context("Locate the official Launcher Generator")?;
    let installer = tools
        .installer
        .as_ref()
        .context("Locate the official Installer")?;
    Ok([
        package_tool_request(generator, package)?,
        package_tool_request(installer, package)?,
    ])
}
fn package_tool_request(
    tool: &Path,
    package: &Path,
) -> Result<portable_builder_kit::process::ToolRequest> {
    let tool = std::fs::canonicalize(tool)?;
    portable_builder_kit::validate_executable(&tool)?;
    let package = std::fs::canonicalize(package)?;
    if !package.is_dir() {
        return Err(anyhow!("Package must be a directory"));
    }
    Ok(portable_builder_kit::process::ToolRequest {
        arguments: vec![package.into_os_string()],
        working_directory: tool.parent().map(Path::to_path_buf),
        executable: tool,
        timeout: std::time::Duration::from_secs(600),
    })
}
fn run_package_tool(
    tool: &Path,
    package: &Path,
    cancel: &dyn Fn() -> bool,
) -> Result<portable_builder_kit::process::ToolOutcome> {
    let request = package_tool_request(tool, package)?;
    Ok(portable_builder_kit::process::run_tool(&request, cancel)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn official_requests_keep_package_as_one_argument() {
        let temp = crate::util::tempdir().unwrap();
        let package = temp.path().join("Package 工具 with spaces");
        std::fs::create_dir(&package).unwrap();
        let tool = temp.path().join("official tool.exe");
        let mut bytes = vec![0u8; 88];
        bytes[..2].copy_from_slice(b"MZ");
        bytes[60..64].copy_from_slice(&64u32.to_le_bytes());
        bytes[64..68].copy_from_slice(b"PE\0\0");
        std::fs::write(&tool, bytes).unwrap();
        let tools = PaTools {
            launcher_generator: Some(tool.clone()),
            installer: Some(tool.clone()),
            ..Default::default()
        };
        for request in official_build_requests(&package, &tools).unwrap() {
            assert_eq!(request.executable, std::fs::canonicalize(&tool).unwrap());
            assert_eq!(
                request.arguments,
                vec![std::fs::canonicalize(&package).unwrap().into_os_string()]
            );
            assert_eq!(
                request.working_directory,
                request.executable.parent().map(Path::to_path_buf)
            );
            assert_eq!(request.timeout.as_secs(), 600);
        }
    }

    #[test]
    fn official_build_rejects_custom_code_without_consent() {
        let temp = crate::util::tempdir().unwrap();
        let path = temp.path().join("App/AppInfo/Launcher/Custom.nsh");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, b"!error must-not-run").unwrap();
        let error = compile_official_with_cancel(
            temp.path(),
            "DemoPortable",
            &PaTools::default(),
            false,
            &|_| panic!("must not run"),
            &|| false,
        )
        .unwrap_err();
        assert!(error.to_string().contains("Custom NSIS"));
        assert_eq!(std::fs::read(path).unwrap(), b"!error must-not-run");
    }
    #[test]
    fn official_build_requires_tools_before_preparation() {
        let temp = crate::util::tempdir().unwrap();
        let missing = temp.path().join("must-not-exist");
        let error = compile_official_with_cancel(
            &missing,
            "DemoPortable",
            &PaTools::default(),
            false,
            &|_| panic!("must not run a tool"),
            &|| false,
        )
        .unwrap_err();
        assert!(error.to_string().contains("official"));
        assert!(!missing.exists());
        let error = compile_official_with_cancel(
            &missing,
            "DemoPortable",
            &PaTools::default(),
            false,
            &|_| panic!("must not run a tool"),
            &|| true,
        )
        .unwrap_err();
        assert!(error.to_string().contains("cancelled"));
    }
    #[test]
    fn cancellation_prevents_preparation_or_tool_acquisition() {
        let temp = crate::util::tempdir().unwrap();
        let missing = temp.path().join("must-not-be-created");
        let error = compile_with_cancel(
            &missing,
            "DemoPortable",
            &PaTools::default(),
            &missing,
            &|_| panic!("must not start work"),
            &|| true,
        )
        .unwrap_err();
        assert!(error.to_string().contains("cancelled"));
        assert!(!missing.exists());
    }

    #[test]
    fn compilation_rejects_mismatched_metadata_before_tool_acquisition() {
        let temp = crate::util::tempdir().unwrap();
        let root = temp.path();
        std::fs::create_dir_all(root.join("App/AppInfo")).unwrap();
        let details = AppDetails {
            name: "Demo".into(),
            ..Default::default()
        };
        std::fs::write(root.join("App/AppInfo/appinfo.ini"), appinfo_ini(&details)).unwrap();
        let data = root.join("unused-data-root");
        let result = compile(root, "OtherPortable", &PaTools::default(), &data, &|_| {
            panic!("must not acquire tools")
        });
        assert!(result.unwrap_err().to_string().contains("does not match"));
        assert!(!data.exists());
    }

    fn dir(p: &str) -> FileEntry {
        FileEntry {
            path: p.into(),
            is_dir: true,
            size: 0,
            mtime: 0,
        }
    }
    fn file(p: &str, size: u64) -> FileEntry {
        FileEntry {
            path: p.into(),
            is_dir: false,
            size,
            mtime: 0,
        }
    }
    fn folders() -> Folders {
        Folders::new(
            "C:\\Users\\Sean",
            "C:\\Users\\Sean\\AppData\\Roaming",
            "C:\\Users\\Sean\\AppData\\Local",
            "C:\\ProgramData",
            "C:\\Users\\Sean\\AppData\\Local\\Temp",
        )
    }

    #[test]
    fn ids_and_versions() {
        let d = AppDetails {
            name: "Notepad++".into(),
            ..Default::default()
        };
        assert_eq!(d.app_id(), "NotepadPlusPlusPortable");
        assert_eq!(d.app_dir(), "NotepadPlusPlus");
        assert_eq!(d.display_name(), "Notepad++ Portable");
        let d = AppDetails {
            name: "GIMP Portable".into(),
            ..Default::default()
        };
        assert_eq!(d.app_id(), "GIMPPortable");
        assert_eq!(d.display_name(), "GIMP Portable");
        let d = AppDetails {
            name: "Portable".into(),
            ..Default::default()
        };
        assert_eq!(d.app_id(), "Portable");
        assert_eq!(d.app_dir(), "App");
        assert_eq!(package_version("8.6.5"), "8.6.5.0");
        assert_eq!(package_version("v24.08 beta 2"), "24.8.2.0");
        assert_eq!(package_version("1.2.3.4.5"), "1.2.3.4");
        assert_eq!(package_version(""), "0.0.0.0");
    }

    #[test]
    fn appinfo_has_required_fields() {
        let d = AppDetails {
            name: "Foo".into(),
            publisher: "Foo Inc.".into(),
            packager: "Sean".into(),
            version: "2.1".into(),
            category: "Nonsense".into(),
            ..Default::default()
        };
        let ini = appinfo_ini(&d);
        for want in [
            "Type=PortableApps.comFormat",
            "Name=Foo Portable",
            "AppID=FooPortable",
            "Publisher=Foo Inc. & Sean",
            "Category=Utilities",
            "PackageVersion=2.1.0.0",
            "DisplayVersion=2.1",
            "Start=FooPortable.exe",
        ] {
            assert!(ini.contains(want), "{want} missing:\n{ini}");
        }
    }

    #[test]
    fn appinfo_metadata_cannot_inject_ini_sections() {
        let d = AppDetails {
            name: "Foo\r\n[Dependencies]\r\nRequiresAdmin=yes".into(),
            publisher: "Acme\r\n[Control]\r\nIcons=99".into(),
            packager: "Builder\"quoted".into(),
            version: "1.2\r\n[Associations]\r\nProtocols=evil".into(),
            homepage: "https://example.test/\r\n[License]\r\nShareable=false".into(),
            language: "English\r\n[Dependencies]\r\nUsesJava=yes".into(),
            description: "Useful\r\n[Control]\r\nIcons=0".into(),
            ..Default::default()
        };
        let ini = appinfo_ini(&d);
        let section_count = |section: &str| ini.lines().filter(|line| *line == section).count();
        assert_eq!(section_count("[Dependencies]"), 0);
        assert_eq!(section_count("[Control]"), 1);
        assert_eq!(section_count("[Associations]"), 0);
        assert_eq!(section_count("[License]"), 1);
        assert!(
            ini.contains("Publisher=Acme [Control] Icons=99 & Builder'quoted"),
            "{ini}"
        );
        assert!(ini.contains("Homepage=https://example.test/ [License] Shareable=false"));
        assert!(ini.contains("Language=English [Dependencies] UsesJava=yes"));
        assert!(ini.contains("Description=Useful [Control] Icons=0"));
    }

    #[test]
    fn help_html_escapes_metadata_and_only_links_http_urls() {
        let d = AppDetails {
            name: "Foo <script>alert(1)</script>".into(),
            description: "A \"quoted\" & useful app".into(),
            homepage: "javascript:alert(1)".into(),
            ..Default::default()
        };
        let html = help_html(&d);
        assert!(!html.contains("<script>"));
        assert!(html.contains("&lt;script&gt;"));
        assert!(html.contains("&quot;quoted&quot; &amp; useful app"));
        assert!(!html.contains("href="));

        let d = AppDetails {
            homepage: "https://example.test/\"onmouseover=\"alert(1)".into(),
            ..Default::default()
        };
        let html = help_html(&d);
        assert!(html.contains("href=\"https://example.test/&quot;onmouseover=&quot;alert(1)\""));
        assert!(!html.contains("\" onmouseover=\""));
    }

    #[test]
    fn appinfo_includes_captured_file_associations_safely() {
        use crate::model::{RegValue, ValueRef};

        let changes = ChangeSet {
            keys_added: vec![
                KeyData::new("HKCU\\Software\\Classes\\.foo"),
                KeyData::new("HKLM\\Software\\Classes\\.BAR"),
                KeyData::new("HKCU\\Software\\Classes\\.foo\\OpenWithProgids"),
                KeyData::new("HKCU\\Software\\Classes\\.bad,entry"),
                KeyData::new("HKCU\\Software\\Other\\.ignored"),
                KeyData::new("HKCU\\Software\\Classes\\myapp")
                    .with(RegValue::string("URL Protocol", "")),
                KeyData::new("HKCU\\Software\\Classes\\bad:scheme")
                    .with(RegValue::string("URL Protocol", "")),
            ],
            values_added: vec![ValueRef {
                key: "HKCU\\Software\\Classes\\.baz".into(),
                value: RegValue::string("", "Baz.Program"),
            }],
            values_modified: vec![crate::model::ValueChange {
                key: "HKLM\\Software\\Classes\\otherapp".into(),
                before: RegValue::string("URL Protocol", "old"),
                after: RegValue::string("URL Protocol", ""),
            }],
            ..Default::default()
        };
        let ini = appinfo_ini_with_changes(
            &AppDetails::default(),
            &crate::portability::Needs::default(),
            &changes,
        );
        assert!(
            ini.contains("[Associations]\r\nFileTypes=bar,baz,foo\r\nProtocols=myapp,otherapp\r\n")
        );
        assert!(!ini.contains("bad,entry"));
        assert!(!ini.contains("bad:scheme"));
        assert!(!ini.contains("ignored"));
        assert_eq!(ini.matches("[Associations]").count(), 1);
    }

    #[test]
    fn discovers_optional_app_compactor_in_portableapps_folder() {
        let root =
            std::env::temp_dir().join(format!("installscope-tools-{}", crate::util::new_id()));
        let launcher = root
            .join("PortableApps.comLauncher")
            .join("PortableApps.comLauncherGenerator.exe");
        let installer = root
            .join("PortableApps.comInstaller")
            .join("PortableApps.comInstaller.exe");
        let compactor = root
            .join("PortableApps.comAppCompactor")
            .join("PortableApps.comAppCompactor.exe");
        for path in [&launcher, &installer, &compactor] {
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, b"test").unwrap();
        }
        let tools = find_tools_in([root.clone()]);
        assert_eq!(tools.launcher_generator, Some(launcher));
        assert_eq!(tools.installer, Some(installer));
        assert_eq!(tools.app_compactor, Some(compactor));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn finds_program_folder_and_main_exe() {
        let cs = ChangeSet {
            files_added: vec![
                dir("C:\\Program Files\\Foo"),
                file("C:\\Program Files\\Foo\\foo.exe", 5_000_000),
                file("C:\\Program Files\\Foo\\uninstall.exe", 9_000_000),
                dir("C:\\Program Files\\Foo\\plugins"),
                file("C:\\Program Files\\Foo\\plugins\\big.exe", 50_000_000),
                dir("C:\\Users\\Sean\\AppData\\Roaming\\Foo"),
                file(
                    "C:\\Users\\Sean\\AppData\\Roaming\\Foo\\settings.exe",
                    99_000_000,
                ),
                dir("C:\\Users\\Sean\\AppData\\Local\\Temp\\x"),
                file(
                    "C:\\Users\\Sean\\AppData\\Local\\Temp\\x\\setup.exe",
                    999_000_000,
                ),
            ],
            ..Default::default()
        };
        // Some programs install into the user's folders (e.g. %APPDATA%\Foo);
        // the largest new tree with a program file wins.
        let pd = find_program_dir(&cs, &folders()).unwrap();
        assert_eq!(pd, "C:\\Users\\Sean\\AppData\\Roaming\\Foo");
        let mut cs2 = cs.clone();
        cs2.files_added.retain(|f| !f.path.contains("Roaming"));
        let pd = find_program_dir(&cs2, &folders()).unwrap();
        assert_eq!(pd, "C:\\Program Files\\Foo");
        assert_eq!(
            find_main_exe(&cs2, &pd, "Foo").unwrap(),
            "C:\\Program Files\\Foo\\foo.exe"
        );
    }

    #[test]
    fn finds_an_existing_program_folder_that_changed() {
        let changed = |p: &str, size| crate::model::FileChange {
            before: file(p, 1),
            after: file(p, size),
        };
        let cs = ChangeSet {
            files_modified: vec![
                changed("C:\\Program Files\\7-Zip\\7zFM.exe", 900_000),
                changed("C:\\Program Files\\7-Zip\\7z.dll", 1_800_000),
                changed("C:\\Program Files\\Big\\data.bin", 900_000_000),
            ],
            ..Default::default()
        };
        let roots = vec!["C:\\Program Files".to_string()];
        assert_eq!(
            changed_program_root(&cs, &roots).unwrap(),
            "C:\\Program Files\\7-Zip"
        );
    }

    #[test]
    fn ico_header() {
        let ico = ico_from_pngs(&[(16, vec![1, 2, 3]), (256, vec![4, 5])]);
        assert_eq!(&ico[..6], &[0, 0, 1, 0, 2, 0]);
        assert_eq!(ico[6], 16);
        assert_eq!(ico[22], 0); // 256 is written as 0
        assert_eq!(u32::from_le_bytes(ico[18..22].try_into().unwrap()), 38);
        assert_eq!(&ico[38..], &[1, 2, 3, 4, 5]);
    }

    #[test]
    fn sandbox_config() {
        let c = wsb_config(
            Path::new("D:\\Tools\\InstallScope"),
            Path::new("D:\\Downloads"),
            Path::new("C:\\Work & Co"),
            "InstallScope.exe",
        );
        assert!(c.contains("<HostFolder>D:\\Tools\\InstallScope</HostFolder>\n      <SandboxFolder>C:\\InstallScope</SandboxFolder>\n      <ReadOnly>true</ReadOnly>"));
        assert!(c.contains("<HostFolder>C:\\Work &amp; Co</HostFolder>\n      <SandboxFolder>C:\\InstallScopeWork</SandboxFolder>\n      <ReadOnly>false</ReadOnly>"));
        assert!(c.contains(
            "<Command>C:\\InstallScope\\InstallScope.exe pa-capture C:\\InstallScopeWork\\job.json</Command>"
        ));
    }

    #[test]
    fn installer_commands() {
        assert_eq!(
            installer_command(Path::new("C:\\dl\\foo.msi"), true),
            "msiexec.exe /i \"C:\\dl\\foo.msi\" /qn /norestart"
        );
        assert_eq!(
            installer_command(Path::new("C:\\dl\\missing.exe"), true),
            "\"C:\\dl\\missing.exe\""
        );
    }
}
