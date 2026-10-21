//! Turning a recorded change set into a removal plan, and carrying it out.
//!
//! This is the step free tools stop short of: given what an installation
//! changed, undo exactly that, with backups, safety rails, and a preview the
//! user can edit before anything is touched.

use crate::backup::{ActionResult, BackupWriter};
use crate::model::*;
use crate::platform::{self, expand_env, Registry};
use crate::util::{is_protected_dir, is_protected_key, protected_dirs};
use anyhow::Result;
use std::collections::BTreeSet;
use std::path::Path;

#[derive(Clone, Debug, PartialEq)]
pub enum Action {
    /// Stop and delete a Windows service the installation registered.
    StopDeleteService {
        name: String,
    },
    /// Delete a scheduled task (task path like `\Vendor\Updater`).
    DeleteTask {
        name: String,
    },
    DeleteFile {
        path: String,
    },
    /// Remove a folder the installation created (deepest first).
    DeleteDir {
        path: String,
    },
    /// Remove a folder and everything in it (leftover scans).
    DeleteTree {
        path: String,
    },
    DeleteKey {
        path: String,
    },
    DeleteValue {
        key: String,
        name: String,
    },
    /// Put back a value's pre-installation data.
    SetValue {
        key: String,
        value: RegValue,
    },
    /// Recreate a key the installation deleted.
    RestoreKey {
        key: KeyData,
    },
    /// Surgically undo a PATH-style list edit without clobbering later edits.
    EditList {
        key: String,
        name: String,
        remove: Vec<String>,
        add_back: Vec<String>,
    },
}
crate::json_enum!(Action { StopDeleteService { name }, DeleteTask { name }, DeleteFile { path }, DeleteDir { path }, DeleteTree { path }, DeleteKey { path }, DeleteValue { key, name }, SetValue { key, value }, RestoreKey { key }, EditList { key, name, remove, add_back } });

impl Action {
    pub fn category(&self) -> &'static str {
        match self {
            Action::StopDeleteService { .. } => "Services",
            Action::DeleteTask { .. } => "Scheduled tasks",
            Action::DeleteFile { .. } | Action::DeleteDir { .. } | Action::DeleteTree { .. } => {
                "Files"
            }
            Action::DeleteKey { .. } | Action::DeleteValue { .. } => "Registry",
            Action::SetValue { .. } | Action::RestoreKey { .. } | Action::EditList { .. } => {
                "Registry restore"
            }
        }
    }

    pub fn describe(&self) -> String {
        match self {
            Action::StopDeleteService { name } => format!("Stop and delete service {name}"),
            Action::DeleteTask { name } => format!("Delete scheduled task {name}"),
            Action::DeleteFile { path } => format!("Delete file {path}"),
            Action::DeleteDir { path } => format!("Delete folder {path}"),
            Action::DeleteTree { path } => format!("Delete folder and contents {path}"),
            Action::DeleteKey { path } => format!("Delete key {path}"),
            Action::DeleteValue { key, name } => {
                format!(
                    "Delete value {key}\\{}",
                    if name.is_empty() { "(Default)" } else { name }
                )
            }
            Action::SetValue { key, value } => {
                format!(
                    "Restore value {key}\\{} = {}",
                    value.display_name(),
                    value.display_data()
                )
            }
            Action::RestoreKey { key } => format!("Recreate key {}", key.path),
            Action::EditList {
                key,
                name,
                remove,
                add_back,
            } => {
                let mut s = format!("Edit {key}\\{name}:");
                if !remove.is_empty() {
                    s.push_str(&format!(" remove {}", remove.join("; ")));
                }
                if !add_back.is_empty() {
                    s.push_str(&format!(" add back {}", add_back.join("; ")));
                }
                s
            }
        }
    }

    /// Execution order: services and tasks first (they hold files open),
    /// then files, folders, registry deletions, and finally restores.
    fn order(&self) -> u8 {
        match self {
            Action::StopDeleteService { .. } => 0,
            Action::DeleteTask { .. } => 1,
            Action::DeleteFile { .. } => 2,
            Action::DeleteDir { .. } | Action::DeleteTree { .. } => 3,
            Action::DeleteValue { .. } => 4,
            Action::DeleteKey { .. } => 5,
            Action::RestoreKey { .. } => 6,
            Action::SetValue { .. } | Action::EditList { .. } => 7,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct PlanItem {
    pub action: Action,
    pub selected: bool,
    /// Whether the target is still present right now.
    pub exists: bool,
    /// Why an item is unselected or needs attention.
    pub note: String,
}
crate::json_struct!(PlanItem, { action, selected, exists, note });

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Plan {
    pub items: Vec<PlanItem>,
    /// Things that cannot be undone automatically (shown to the user).
    pub info: Vec<String>,
}
crate::json_struct!(Plan, { items, info });

impl Plan {
    pub fn selected(&self) -> usize {
        self.items.iter().filter(|i| i.selected).count()
    }
}

#[derive(Clone, Debug, Default)]
pub struct ExecOptions {
    /// Delete folder contents that weren't part of the installation.
    pub remove_unlogged: bool,
    /// Report what would happen without changing anything.
    pub dry_run: bool,
    /// Send deleted files and folders to the Recycle Bin (in addition to the
    /// backup) instead of deleting them permanently.
    pub use_recycle_bin: bool,
    /// Close Windows Explorer while the removal runs if it has any of the
    /// program's DLLs loaded (shell extensions), and start it again after,
    /// so those files are deleted now instead of at the next restart.
    pub restart_explorer: bool,
    /// Close running copies of the program being removed (only processes
    /// whose own .exe is among the files to delete) before deleting.
    pub close_programs: bool,
}

/// How long a service gets to stop before its removal goes ahead anyway.
const SERVICE_STOP_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

const SERVICES_PREFIX: &str = "hklm\\system\\currentcontrolset\\services\\";

pub(crate) fn service_name(key_path: &str) -> Option<String> {
    let f = fold(key_path);
    let rest = f.strip_prefix(SERVICES_PREFIX)?;
    if rest.is_empty() || rest.contains('\\') {
        return None;
    }
    Some(platform::leaf_name(key_path).to_string())
}

pub(crate) fn tasks_dir() -> String {
    fold(&expand_env("%SystemRoot%\\System32\\Tasks\\"))
}

fn is_path_list(key: &str, name: &str) -> bool {
    let n = fold(name);
    (n == "path" || n == "pathext" || n == "psmodulepath") && fold(key).ends_with("environment")
}

fn split_list(s: &str) -> Vec<String> {
    s.split(';')
        .map(|p| p.trim().to_string())
        .filter(|p| !p.is_empty())
        .collect()
}

/// Build the removal plan for an installation's change set.
pub fn build_plan(cs: &ChangeSet, reg: &dyn Registry) -> Plan {
    let mut plan = Plan::default();
    let protected = protected_dirs();
    let tasks = tasks_dir();
    let mut push = |action: Action, selected: bool, exists: bool, note: &str| {
        plan.items.push(PlanItem {
            action,
            selected: selected && exists,
            exists,
            note: note.to_string(),
        });
    };

    // Services and scheduled tasks.
    for k in &cs.keys_added {
        if let Some(name) = service_name(&k.path) {
            let exists = reg.exists(&k.path);
            push(Action::StopDeleteService { name }, true, exists, "");
        }
    }
    for f in &cs.files_added {
        if !f.is_dir && fold(&f.path).starts_with(&tasks) {
            let rel = &f.path[tasks.len()..];
            let exists = Path::new(&f.path).exists();
            push(
                Action::DeleteTask {
                    name: format!("\\{rel}"),
                },
                true,
                exists,
                "",
            );
        }
    }

    // Files and folders.
    for f in cs.files_added.iter().filter(|f| !f.is_dir) {
        let exists = Path::new(&f.path).exists();
        push(
            Action::DeleteFile {
                path: f.path.clone(),
            },
            true,
            exists,
            "",
        );
    }
    for f in cs.files_added.iter().filter(|f| f.is_dir) {
        let exists = Path::new(&f.path).exists();
        if is_protected_dir(&f.path, &protected) {
            push(
                Action::DeleteDir {
                    path: f.path.clone(),
                },
                false,
                exists,
                "Protected system folder",
            );
        } else {
            push(
                Action::DeleteDir {
                    path: f.path.clone(),
                },
                true,
                exists,
                "",
            );
        }
    }

    // Registry: delete only the top-most added keys (subtrees go with them).
    let added: BTreeSet<String> = cs.keys_added.iter().map(|k| fold(&k.path)).collect();
    for k in &cs.keys_added {
        let mut has_added_parent = false;
        let mut p = k.path.as_str();
        while let Some(parent) = platform::parent_path(p) {
            // A protected parent won't be deleted, so its children need their own items.
            if added.contains(&fold(parent)) && !is_protected_key(parent) {
                has_added_parent = true;
                break;
            }
            p = parent;
        }
        if has_added_parent {
            continue;
        }
        let exists = reg.exists(&k.path);
        if is_protected_key(&k.path) {
            push(
                Action::DeleteKey {
                    path: k.path.clone(),
                },
                false,
                exists,
                "Protected system key",
            );
        } else {
            push(
                Action::DeleteKey {
                    path: k.path.clone(),
                },
                true,
                exists,
                "",
            );
        }
    }
    let current_value =
        |key: &str, name: &str| reg.read_key(key).and_then(|k| k.get(name).cloned());
    for v in &cs.values_added {
        let cur = current_value(&v.key, &v.value.name);
        let changed = cur.as_ref().is_some_and(|c| c != &v.value);
        push(
            Action::DeleteValue {
                key: v.key.clone(),
                name: v.value.name.clone(),
            },
            !changed,
            cur.is_some(),
            if changed {
                "Changed since installation"
            } else {
                ""
            },
        );
    }
    for v in &cs.values_modified {
        let cur = current_value(&v.key, &v.after.name);
        if is_path_list(&v.key, &v.after.name) {
            let (Some(b), Some(a)) = (v.before.as_string(), v.after.as_string()) else {
                continue;
            };
            let (bl, al) = (split_list(&b), split_list(&a));
            let bset: BTreeSet<String> = bl.iter().map(|s| fold(s)).collect();
            let aset: BTreeSet<String> = al.iter().map(|s| fold(s)).collect();
            let remove: Vec<String> = al
                .iter()
                .filter(|s| !bset.contains(&fold(s)))
                .cloned()
                .collect();
            let add_back: Vec<String> = bl
                .iter()
                .filter(|s| !aset.contains(&fold(s)))
                .cloned()
                .collect();
            if !remove.is_empty() || !add_back.is_empty() {
                push(
                    Action::EditList {
                        key: v.key.clone(),
                        name: v.after.name.clone(),
                        remove,
                        add_back,
                    },
                    true,
                    cur.is_some(),
                    "Only the entries the installer changed",
                );
            }
            continue;
        }
        let changed = cur.as_ref().is_some_and(|c| c != &v.after);
        push(
            Action::SetValue {
                key: v.key.clone(),
                value: v.before.clone(),
            },
            !changed,
            true,
            if changed {
                "Changed again since installation"
            } else {
                ""
            },
        );
    }
    for v in &cs.values_removed {
        let cur = current_value(&v.key, &v.value.name);
        push(
            Action::SetValue {
                key: v.key.clone(),
                value: v.value.clone(),
            },
            cur.is_none(),
            true,
            if cur.is_some() {
                "Value exists again"
            } else {
                ""
            },
        );
    }
    // Recreate removed keys, top-most first, with their values.
    let mut removed = cs.keys_removed.clone();
    removed.sort_by_key(|a| a.path.len());
    for k in removed {
        let exists = reg.exists(&k.path);
        push(
            Action::RestoreKey { key: k },
            !exists,
            true,
            if exists { "Key exists again" } else { "" },
        );
    }

    for f in &cs.files_modified {
        plan.info.push(format!(
            "Modified during installation (left as is): {}",
            f.after.path
        ));
    }
    for f in &cs.files_removed {
        plan.info.push(format!(
            "Deleted during installation (cannot be restored): {}",
            f.path
        ));
    }
    mark_windows_components(&mut plan, cs, &fold(&expand_env("%SystemRoot%")));
    plan.items.sort_by_key(|i| i.action.order());
    plan
}

/// The part of a registry path that identifies its owner: the COM class or
/// AppID for `...\CLSID\{guid}\...` paths, otherwise the key itself.
fn owner_root(path: &str) -> String {
    let f = fold(path);
    for marker in ["\\clsid\\{", "\\appid\\{", "\\typelib\\{", "\\interface\\{"] {
        if let Some(i) = f.find(marker) {
            let rest = &f[i + marker.len()..];
            if let Some(end) = rest.find('}') {
                return f[..i + marker.len() + end + 1].to_string();
            }
        }
    }
    f
}

/// Registry areas Windows maintains itself (its security components and
/// app packages). Windows can change them in the background during an
/// installation, so they are never treated as the program's.
fn windows_owned_key(path: &str) -> bool {
    let f = fold(path);
    [
        "\\windows defender",
        "securityhealth",
        "sechealth",
        "defenderagent",
        "\\appx\\packagestate\\",
        "\\appmodel\\",
        // Task Scheduler's own bookkeeping (run times, triggers); a program's
        // tasks are removed together with the task itself.
        "\\schedule\\taskcache\\",
    ]
    .iter()
    .any(|m| f.contains(m))
}

/// Unselect registry items that belong to Windows rather than the program:
/// entries whose data points into the Windows folder (and everything under
/// the same COM class or AppID), and areas Windows maintains itself.
fn mark_windows_components(plan: &mut Plan, cs: &ChangeSet, sysroot: &str) {
    let sysroot = sysroot.to_string();
    let check_paths = !sysroot.contains('%') && !sysroot.is_empty();
    // Folders of Windows' own security components, which update themselves.
    let mut windows_folders: Vec<String> = vec![
        "%programdata%\\microsoft\\windows defender".into(),
        "%programfiles%\\windows defender".into(),
    ];
    for v in [
        "%ProgramData%\\Microsoft\\Windows Defender",
        "%ProgramData%\\Microsoft\\Windows Security Health",
        "%ProgramFiles%\\Windows Defender",
        "%ProgramFiles(x86)%\\Windows Defender",
        "%ProgramFiles%\\Windows Security",
    ] {
        let e = fold(&expand_env(v));
        if !e.contains('%') {
            windows_folders.push(e);
        }
    }
    let points_into_windows = |v: &RegValue| {
        v.as_string().is_some_and(|s| {
            let f = fold(&s);
            let f = f.trim_start_matches('@').trim_start_matches('"');
            let f = f.strip_prefix("\\\\?\\").unwrap_or(f);
            (check_paths && f.starts_with(&sysroot))
                || f.starts_with("%systemroot%")
                || f.starts_with("%windir%")
                || windows_folders.iter().any(|w| f.starts_with(w.as_str()))
        })
    };
    let mut owned: BTreeSet<String> = BTreeSet::new();
    for k in &cs.keys_added {
        if k.values.values().any(&points_into_windows) {
            owned.insert(owner_root(&k.path));
        }
    }
    for v in &cs.values_added {
        if points_into_windows(&v.value) {
            owned.insert(owner_root(&v.key));
        }
    }
    for v in &cs.values_modified {
        if points_into_windows(&v.before) || points_into_windows(&v.after) {
            owned.insert(owner_root(&v.key));
        }
    }
    for v in &cs.values_removed {
        if points_into_windows(&v.value) {
            owned.insert(owner_root(&v.key));
        }
    }
    for i in &mut plan.items {
        let key = match &i.action {
            Action::DeleteKey { path } => path.as_str(),
            Action::DeleteValue { key, .. } | Action::SetValue { key, .. } => key.as_str(),
            Action::RestoreKey { key } => key.path.as_str(),
            _ => continue,
        };
        if windows_owned_key(key) || owned.contains(&owner_root(key)) {
            i.selected = false;
            i.note = "Windows component".to_string();
        }
    }
}

/// Why an action must never run: deleting a protected folder or registry
/// key (Windows, Program Files, profiles, drive roots, hive roots…). Checked
/// for every item, whether or not the target exists.
pub fn refusal(action: &Action, protected: &[String]) -> Option<String> {
    match action {
        Action::DeleteDir { path } | Action::DeleteTree { path }
            if is_protected_dir(path, protected) =>
        {
            Some(format!("refusing to delete protected folder {path}"))
        }
        Action::DeleteFile { path } if is_protected_dir(path, protected) => {
            Some(format!("refusing to delete protected location {path}"))
        }
        Action::DeleteKey { path } if is_protected_key(path) => {
            Some(format!("refusing to delete protected key {path}"))
        }
        _ => None,
    }
}

/// Carry out the selected plan items, backing everything up first.
pub fn execute_plan(
    plan: &Plan,
    reg: &dyn Registry,
    backup: Option<&mut BackupWriter>,
    opts: &ExecOptions,
) -> Vec<ActionResult> {
    execute_plan_with(plan, reg, backup, opts, &protected_dirs())
}

/// [`execute_plan`] with a chosen list of protected folders (for tests).
pub fn execute_plan_with(
    plan: &Plan,
    reg: &dyn Registry,
    mut backup: Option<&mut BackupWriter>,
    opts: &ExecOptions,
    protected: &[String],
) -> Vec<ActionResult> {
    let mut results = Vec::new();
    let mut items: Vec<&PlanItem> = plan.items.iter().filter(|i| i.selected).collect();
    items.sort_by(|a, b| {
        a.action
            .order()
            .cmp(&b.action.order())
            .then_with(|| match (&a.action, &b.action) {
                // Deepest folders first.
                (
                    Action::DeleteDir { path: x } | Action::DeleteTree { path: x },
                    Action::DeleteDir { path: y } | Action::DeleteTree { path: y },
                ) => y.len().cmp(&x.len()),
                _ => std::cmp::Ordering::Equal,
            })
    });
    let allowed: Vec<&Action> = items
        .iter()
        .filter(|i| refusal(&i.action, protected).is_none())
        .map(|i| &i.action)
        .collect();
    if opts.close_programs && !opts.dry_run {
        let exes = crate::locks::program_paths(allowed.iter().copied());
        let refs: Vec<&Path> = exes.iter().map(|p| p.as_path()).collect();
        let c = crate::locks::close_programs(&refs);
        if !c.closed.is_empty() {
            results.push(ActionResult::ok(
                "Close the running program",
                format!("closed {}", c.closed.join(", ")),
            ));
        }
        if !c.still_running.is_empty() {
            results.push(ActionResult::err(
                "Close the running program",
                format!(
                    "{} kept running; files it uses will be deleted at the next restart",
                    c.still_running.join(", ")
                ),
            ));
        }
    }
    // Explorer is closed once for the whole removal, not per file.
    let explorer = if opts.restart_explorer && !opts.dry_run {
        let modules = crate::locks::module_paths(allowed.iter().copied());
        let refs: Vec<&Path> = modules.iter().map(|p| p.as_path()).collect();
        let pause = crate::locks::pause_explorer(&refs);
        if pause.is_some() {
            results.push(ActionResult::ok(
                "Close Windows Explorer",
                "it had the program's shell extensions loaded; it starts again when the removal is done",
            ));
        }
        pause
    } else {
        None
    };
    for item in items {
        let desc = item.action.describe();
        // Refused whatever the item's origin, before anything is touched.
        if let Some(why) = refusal(&item.action, protected) {
            results.push(ActionResult::err(desc, why));
            continue;
        }
        if opts.dry_run {
            results.push(ActionResult::ok(desc, "would run"));
            continue;
        }
        let r = run_action(&item.action, reg, backup.as_deref_mut(), opts, protected);
        results.push(match r {
            Ok(msg) => ActionResult::ok(desc, msg),
            Err(e) => ActionResult::err(desc, format!("{e:#}")),
        });
    }
    drop(explorer); // starts Explorer again
    results
}

/// ERROR_CHILD_MUST_BE_VOLATILE: a lasting key can't be created under a
/// volatile (memory-only) one.
fn is_volatile_parent_error(e: &anyhow::Error) -> bool {
    e.chain().any(|c| {
        c.downcast_ref::<std::io::Error>()
            .and_then(|io| io.raw_os_error())
            == Some(1021)
    })
}

fn run_action(
    action: &Action,
    reg: &dyn Registry,
    mut backup: Option<&mut BackupWriter>,
    opts: &ExecOptions,
    protected: &[String],
) -> Result<String> {
    match action {
        Action::StopDeleteService { name } => {
            let key = format!("HKLM\\SYSTEM\\CurrentControlSet\\Services\\{name}");
            if let Some(b) = backup.as_deref_mut() {
                b.add_tree(reg, &key)?;
            }
            // Wait until it has really stopped, so its files are released
            // before the file deletions that follow.
            let stop = crate::locks::stop_service(name, SERVICE_STOP_TIMEOUT);
            let (code, out) = platform::run_hidden("sc.exe", &["delete", name])?;
            if code == 0 {
                Ok(match stop {
                    Ok(crate::locks::ServiceStop::Stopped) => "stopped and deleted".into(),
                    Ok(_) => "deleted (it was not running)".into(),
                    Err(e) => format!(
                        "deleted, but it did not stop ({e:#}); Windows removes it at the next restart"
                    ),
                })
            } else if code == 1060 {
                Ok("already gone".into())
            } else {
                anyhow::bail!("sc delete failed ({code}): {out}")
            }
        }
        Action::DeleteTask { name } => {
            let file = format!("{}{}", expand_env("%SystemRoot%\\System32\\Tasks"), name);
            if let Some(b) = backup.as_deref_mut() {
                if Path::new(&file).is_file() {
                    b.add_file(Path::new(&file))?;
                }
            }
            let (code, out) =
                platform::run_hidden("schtasks.exe", &["/Delete", "/TN", name, "/F"])?;
            if code == 0 {
                Ok("deleted".into())
            } else if !Path::new(&file).exists() {
                Ok("already gone".into())
            } else {
                anyhow::bail!("schtasks failed ({code}): {out}")
            }
        }
        Action::DeleteFile { path } => {
            let p = Path::new(path);
            if !p.exists() {
                return Ok("already gone".into());
            }
            if let Some(b) = backup.as_deref_mut() {
                b.add_file(p)?;
            }
            if opts.use_recycle_bin && platform::recycle(p).is_ok() {
                return Ok("moved to the Recycle Bin".into());
            }
            match platform::delete_file(p) {
                Ok(()) => Ok("deleted".into()),
                Err(e) => {
                    let why = match crate::locks::describe(&crate::locks::file_holders(&[p])) {
                        Some(who) => format!("in use, {who}"),
                        None => format!("in use ({e})"),
                    };
                    match platform::delete_on_reboot(p) {
                        Ok(()) => Ok(format!("{why}; will be deleted at next restart")),
                        Err(_) => Err(anyhow::anyhow!("{why}: {e}")),
                    }
                }
            }
        }
        Action::DeleteDir { path } => {
            let p = Path::new(path);
            if !p.exists() {
                return Ok("already gone".into());
            }
            crate::util::guard_dir_delete(p, protected)?;
            if crate::util::is_link(p) {
                // A junction or symbolic link: remove the link, never what it points to.
                std::fs::remove_dir(p).or_else(|_| std::fs::remove_file(p))?;
                return Ok("link removed".into());
            }
            let empty = std::fs::read_dir(p)
                .map(|mut d| d.next().is_none())
                .unwrap_or(false);
            if empty {
                if let Some(b) = backup.as_deref_mut() {
                    b.add_dir(p)?;
                }
                std::fs::remove_dir(p)?;
                Ok("deleted".into())
            } else if opts.remove_unlogged {
                if let Some(b) = backup.as_deref_mut() {
                    b.add_dir_tree(p)?;
                }
                if opts.use_recycle_bin && platform::recycle(p).is_ok() {
                    return Ok(
                        "moved to the Recycle Bin with contents created after installation".into(),
                    );
                }
                std::fs::remove_dir_all(p)?;
                Ok("deleted with contents created after installation".into())
            } else {
                Ok("kept: contains files created after installation".into())
            }
        }
        Action::DeleteTree { path } => {
            let p = Path::new(path);
            if !p.exists() {
                return Ok("already gone".into());
            }
            crate::util::guard_dir_delete(p, protected)?;
            if crate::util::is_link(p) {
                // A junction or symbolic link: remove the link, never what it points to.
                std::fs::remove_dir(p).or_else(|_| std::fs::remove_file(p))?;
                return Ok("link removed".into());
            }
            if let Some(b) = backup.as_deref_mut() {
                b.add_dir_tree(p)?;
            }
            if opts.use_recycle_bin && platform::recycle(p).is_ok() {
                return Ok("moved to the Recycle Bin".into());
            }
            match std::fs::remove_dir_all(p) {
                Ok(()) => Ok("deleted".into()),
                Err(e) => {
                    // Something is locked: remove what we can, schedule the rest.
                    let mut scheduled: Vec<std::path::PathBuf> = Vec::new();
                    for f in crate::fswalk::WalkDir::new(p)
                        .contents_first(true)
                        .into_iter()
                        .flatten()
                    {
                        let fp = f.path();
                        let gone = if f.file_type().is_dir() {
                            std::fs::remove_dir(fp).is_ok()
                        } else {
                            platform::delete_file(fp).is_ok()
                        };
                        if !gone && platform::delete_on_reboot(fp).is_ok() {
                            scheduled.push(fp.to_path_buf());
                        }
                    }
                    if !scheduled.is_empty() {
                        let files: Vec<&Path> = scheduled
                            .iter()
                            .filter(|p| p.is_file())
                            .map(|p| p.as_path())
                            .collect();
                        let who = crate::locks::describe(&crate::locks::file_holders(&files))
                            .map(|w| format!(" ({w})"))
                            .unwrap_or_default();
                        Ok(format!(
                            "{} locked items{who} will be deleted at next restart",
                            scheduled.len()
                        ))
                    } else if p.exists() {
                        Err(e.into())
                    } else {
                        Ok("deleted".into())
                    }
                }
            }
        }
        Action::DeleteKey { path } => {
            if !reg.exists(path) {
                return Ok("already gone".into());
            }
            if is_protected_key(path) {
                anyhow::bail!("refusing to delete protected key");
            }
            if let Some(b) = backup.as_deref_mut() {
                b.add_tree(reg, path)?;
            }
            reg.delete_tree(path)?;
            Ok("deleted".into())
        }
        Action::DeleteValue { key, name } => {
            if reg
                .read_key(key)
                .and_then(|k| k.get(name).cloned())
                .is_none()
            {
                return Ok("already gone".into());
            }
            if let Some(b) = backup.as_deref_mut() {
                b.add_value(reg, key, name)?;
            }
            reg.delete_value(key, name)?;
            Ok("deleted".into())
        }
        Action::SetValue { key, value } => {
            if let Some(b) = backup.as_deref_mut() {
                if !reg.exists(key) {
                    b.add_key_absent(key)?;
                }
                b.add_value(reg, key, &value.name)?;
            }
            reg.set_value(key, value)?;
            Ok("restored".into())
        }
        Action::RestoreKey { key } => {
            if let Some(b) = backup.as_deref_mut() {
                if !reg.exists(&key.path) {
                    b.add_key_absent(&key.path)?;
                }
            }
            match platform::write_tree(reg, std::slice::from_ref(key)) {
                Ok(()) => Ok("recreated".into()),
                // Under a key Windows keeps only in memory: it recreates
                // such keys itself, and a lasting copy can't be made there.
                Err(e) if is_volatile_parent_error(&e) => {
                    Ok("skipped: Windows keeps this key in memory and recreates it itself".into())
                }
                Err(e) => Err(e),
            }
        }
        Action::EditList {
            key,
            name,
            remove,
            add_back,
        } => {
            let cur = reg.read_key(key).and_then(|k| k.get(name).cloned());
            let Some(cur) = cur else {
                return Ok("value no longer exists".into());
            };
            let text = cur.as_string().unwrap_or_default();
            let rm: BTreeSet<String> = remove.iter().map(|s| fold(s)).collect();
            let mut list: Vec<String> = split_list(&text)
                .into_iter()
                .filter(|s| !rm.contains(&fold(s)))
                .collect();
            for a in add_back {
                if !list.iter().any(|s| fold(s) == fold(a)) {
                    list.push(a.clone());
                }
            }
            let new_text = list.join(";");
            if new_text == text {
                return Ok("nothing to change".into());
            }
            if let Some(b) = backup {
                b.add_value(reg, key, name)?;
            }
            let nv = RegValue {
                name: cur.name.clone(),
                kind: cur.kind,
                data: encode_utf16z(&new_text),
            };
            reg.set_value(key, &nv)?;
            Ok("updated".into())
        }
    }
}

/// How careful the user should be with a plan item (shown as a label).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Risk {
    Safe,
    Review,
    Careful,
}
crate::json_enum!(Risk {
    Safe,
    Review,
    Careful
});

impl Risk {
    pub fn label(&self) -> &'static str {
        match self {
            Risk::Safe => "Safe",
            Risk::Review => "Review",
            Risk::Careful => "Careful",
        }
    }

    pub fn explanation(&self) -> &'static str {
        match self {
            Risk::Safe => "Belongs to the program; safe to remove.",
            Risk::Review => "Probably belongs to the program, but check that you don't need it.",
            Risk::Careful => "Affects Windows or other programs. Only remove it if you are sure.",
        }
    }
}

/// Remove what Simple mode should not offer: items marked Careful (services,
/// scheduled tasks, Windows folders, shared registry areas) and uncertain
/// matches the scanner itself left unticked (for example a name-only match
/// when an install log already covers the program, or protected locations).
/// Returns how many were removed.
pub fn simplify(plan: &mut Plan) -> usize {
    let before = plan.items.len();
    plan.items
        .retain(|i| i.selected && risk(i) != Risk::Careful);
    before - plan.items.len()
}

/// Classify a plan item for beginners.
pub fn risk(item: &PlanItem) -> Risk {
    let sysroot = fold(&expand_env("%SystemRoot%"));
    let in_windows = |p: &str| !sysroot.contains('%') && fold(p).starts_with(&sysroot);
    let shared_key = |p: &str| {
        let f = fold(p);
        f.contains("\\classes\\")
            || f.starts_with("hkcr\\")
            || f.contains("\\currentcontrolset\\")
            || f.contains("\\policies\\")
    };
    let base = match &item.action {
        Action::StopDeleteService { .. } | Action::DeleteTask { .. } => Risk::Careful,
        Action::DeleteFile { path } | Action::DeleteDir { path } | Action::DeleteTree { path } => {
            if in_windows(path) {
                Risk::Careful
            } else {
                Risk::Safe
            }
        }
        Action::DeleteKey { path } => {
            if shared_key(path) {
                Risk::Careful
            } else {
                Risk::Safe
            }
        }
        Action::DeleteValue { key, .. } => {
            if shared_key(key) {
                Risk::Careful
            } else {
                Risk::Review
            }
        }
        Action::SetValue { key, .. } | Action::EditList { key, .. } => {
            if shared_key(key) {
                Risk::Careful
            } else {
                Risk::Review
            }
        }
        Action::RestoreKey { .. } => Risk::Review,
    };
    if !item.note.is_empty() && base == Risk::Safe {
        Risk::Review
    } else {
        base
    }
}

/// How sure SetupSweep is that removing an item is harmless, 0–100, with the
/// reasons. Green (80+) is safe, yellow (45–79) is shared or uncertain, red
/// (below 45) touches Windows or other programs.
#[derive(Clone, Debug, PartialEq)]
pub struct Confidence {
    pub score: u8,
    pub reasons: Vec<&'static str>,
}

impl Confidence {
    pub fn band(&self) -> Risk {
        match self.score {
            80.. => Risk::Safe,
            45.. => Risk::Review,
            _ => Risk::Careful,
        }
    }
}

/// Software that ships with Windows or with hardware: entries mentioning it
/// are rarely a removed program's own.
const SYSTEM_VENDORS: &[&str] = &[
    "\\microsoft\\",
    "\\windows\\",
    "\\nvidia",
    "\\intel",
    "\\amd\\",
    "\\realtek",
    "\\dolby",
    "\\synaptics",
    "\\elan",
    "\\qualcomm",
    "\\broadcom",
    "\\vmware",
    "\\oracle\\virtualbox guest",
];

/// Score a plan item with local heuristics (no data leaves the PC).
pub fn confidence(item: &PlanItem) -> Confidence {
    let mut score: i32 = 70;
    let mut reasons: Vec<&'static str> = Vec::new();
    let mut add = |d: i32, why: &'static str| {
        score += d;
        reasons.push(why);
    };
    let sysroot = fold(&expand_env("%SystemRoot%"));
    let target = match &item.action {
        Action::DeleteFile { path } | Action::DeleteDir { path } | Action::DeleteTree { path } => {
            fold(path)
        }
        Action::DeleteKey { path } => fold(path),
        Action::DeleteValue { key, .. }
        | Action::SetValue { key, .. }
        | Action::EditList { key, .. } => fold(key),
        Action::RestoreKey { key } => fold(&key.path),
        Action::StopDeleteService { name } | Action::DeleteTask { name } => fold(name),
    };
    let padded = format!("{target}\\");
    match &item.action {
        Action::DeleteFile { .. } | Action::DeleteDir { .. } | Action::DeleteTree { .. } => {
            if !sysroot.contains('%') && target.starts_with(&sysroot) {
                add(-55, "Inside the Windows folder");
            } else if target.contains("\\common files\\") {
                add(-25, "In Common Files, which programs share");
            } else if target.contains("\\program files")
                || target.contains("\\appdata\\local\\programs\\")
            {
                add(20, "In a program's own folder");
            } else if target.contains("\\appdata\\") || target.contains("\\programdata\\") {
                add(12, "Program data in AppData or ProgramData");
            }
            if target.ends_with(".sys") {
                add(-30, "A driver");
            } else if target.ends_with(".lnk")
                || target.ends_with(".log")
                || target.ends_with(".tmp")
            {
                add(15, "A shortcut, log or temporary file");
            } else if target.ends_with(".dll") && !target.contains("\\program files") {
                add(-10, "A library other programs might use");
            }
        }
        Action::DeleteKey { .. } | Action::DeleteValue { .. } => {
            if target.contains("\\currentversion\\uninstall\\") {
                add(25, "The program's own uninstall entry");
            } else if target.contains("\\currentcontrolset\\") {
                add(-40, "Windows system configuration");
            } else if target.contains("\\policies\\") {
                add(-30, "A policy setting");
            } else if target.contains("\\classes\\clsid\\")
                || target.contains("\\typelib\\")
                || target.contains("\\interface\\")
            {
                add(-22, "A COM registration other programs could use");
            } else if target.contains("\\classes\\") {
                add(-12, "A file type or shell setting");
            } else if target.contains("\\currentversion\\run") {
                add(15, "A startup entry");
            } else if target.starts_with("hkcu\\software\\")
                || target.starts_with("hklm\\software\\")
            {
                add(12, "The program's own settings key");
            }
            if matches!(item.action, Action::DeleteValue { .. }) {
                add(-5, "A single value in a key that stays");
            }
        }
        Action::StopDeleteService { .. } => add(-35, "A Windows service"),
        Action::DeleteTask { .. } => add(-20, "A scheduled task"),
        Action::SetValue { .. } | Action::RestoreKey { .. } | Action::EditList { .. } => {
            add(5, "Puts back how it was before the installation")
        }
    }
    if SYSTEM_VENDORS.iter().any(|v| padded.contains(v)) {
        add(-30, "Mentions Windows or a hardware maker");
    }
    if !item.note.is_empty() {
        add(-15, "Flagged for a closer look");
    }
    // The broad Safe/Review/Careful rules set the ceiling.
    let cap = match risk(item) {
        Risk::Safe => 100,
        Risk::Review => 79,
        Risk::Careful => 44,
    };
    Confidence {
        score: score.clamp(1, cap) as u8,
        reasons,
    }
}

/// What an uninstaller left behind: the parts of an installation that are
/// still present (or still changed) on the system now.
pub fn residue(cs: &ChangeSet, reg: &dyn Registry) -> Plan {
    let mut p = build_plan(cs, reg);
    // Protected locations are never removed, so they never count as residue.
    p.items
        .retain(|i| !i.note.starts_with("Protected") && i.note != "Windows component");
    p.items.retain(|i| match &i.action {
        Action::DeleteFile { .. }
        | Action::DeleteDir { .. }
        | Action::DeleteTree { .. }
        | Action::DeleteKey { .. }
        | Action::DeleteValue { .. }
        | Action::StopDeleteService { .. }
        | Action::DeleteTask { .. } => i.exists,
        Action::SetValue { .. } | Action::RestoreKey { .. } => i.selected,
        Action::EditList {
            key, name, remove, ..
        } => reg
            .read_key(key)
            .and_then(|k| k.get_string(name))
            .is_some_and(|cur| {
                let l = split_list(&cur);
                remove.iter().any(|r| l.iter().any(|x| fold(x) == fold(r)))
            }),
    });
    for i in &mut p.items {
        i.selected = i.note.is_empty() || matches!(i.action, Action::DeleteValue { .. });
    }
    p.info.clear();
    p
}

/// Markdown report of an uninstaller test.
pub fn residue_markdown(name: &str, plan: &Plan) -> String {
    let mut s = format!("# Uninstaller test: {name}\n\n");
    if plan.items.is_empty() {
        s.push_str("**Clean.** The uninstaller removed everything the installation added.\n");
    } else {
        s.push_str(&format!(
            "**{} items were left behind or not restored.**\n\n",
            plan.items.len()
        ));
        let mut cats: Vec<&str> = plan.items.iter().map(|i| i.action.category()).collect();
        cats.sort();
        cats.dedup();
        for c in cats {
            s.push_str(&format!("## {c}\n\n"));
            for i in plan.items.iter().filter(|i| i.action.category() == c) {
                s.push_str(&format!("- {}\n", i.action.describe()));
            }
            s.push('\n');
        }
    }
    s.push_str("\n_Generated by SetupSweep_\n");
    s
}

#[cfg(test)]
mod confidence_tests {
    use super::*;

    fn item(action: Action) -> PlanItem {
        PlanItem {
            action,
            selected: true,
            exists: true,
            note: String::new(),
        }
    }

    #[test]
    fn scores_by_location() {
        let own = confidence(&item(Action::DeleteDir {
            path: "C:\\Program Files\\Acme".into(),
        }));
        assert_eq!(own.band(), Risk::Safe, "{own:?}");
        let uninst = confidence(&item(Action::DeleteKey {
            path: "HKLM\\SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\Acme".into(),
        }));
        assert!(uninst.score >= 60, "{uninst:?}");
        let com = confidence(&item(Action::DeleteKey {
            path: "HKLM\\SOFTWARE\\Classes\\CLSID\\{1}".into(),
        }));
        assert_eq!(com.band(), Risk::Careful, "{com:?}");
        let svc = confidence(&item(Action::StopDeleteService {
            name: "AcmeUpdate".into(),
        }));
        assert_eq!(svc.band(), Risk::Careful);
        let nv = confidence(&item(Action::DeleteDir {
            path: "C:\\ProgramData\\NVIDIA Corporation\\x".into(),
        }));
        assert!(nv.band() != Risk::Safe, "{nv:?}");
        let mut flagged = item(Action::DeleteDir {
            path: "C:\\Program Files\\Acme".into(),
        });
        flagged.note = "Matched by name".into();
        assert!(confidence(&flagged).score < own.score);
    }
}

#[cfg(test)]
mod protected_tests {
    use super::*;
    use crate::platform::MemRegistry;
    use crate::util::protected_dirs_with;

    fn win_env(s: &str) -> String {
        let vars = [
            ("SystemDrive", "C:"),
            ("SystemRoot", "C:\\Windows"),
            ("ProgramFiles", "C:\\Program Files"),
            ("ProgramFiles(x86)", "C:\\Program Files (x86)"),
            ("ProgramW6432", "C:\\Program Files"),
            ("CommonProgramFiles", "C:\\Program Files\\Common Files"),
            (
                "CommonProgramFiles(x86)",
                "C:\\Program Files (x86)\\Common Files",
            ),
            ("CommonProgramW6432", "C:\\Program Files\\Common Files"),
            ("ProgramData", "C:\\ProgramData"),
            ("PUBLIC", "C:\\Users\\Public"),
            ("USERPROFILE", "C:\\Users\\Sean"),
            ("APPDATA", "C:\\Users\\Sean\\AppData\\Roaming"),
            ("LOCALAPPDATA", "C:\\Users\\Sean\\AppData\\Local"),
        ];
        let mut out = s.to_string();
        for (k, v) in vars {
            out = out.replace(&format!("%{k}%"), v);
        }
        out
    }

    const FOLDERS: &[&str] = &[
        "C:\\",
        "C:",
        "D:\\",
        "\\\\server\\share",
        "C:\\Windows",
        "C:\\Windows\\",
        "c:\\windows",
        "C:\\Windows\\System32",
        "C:\\Windows\\System32\\drivers",
        "C:\\Windows\\SysWOW64",
        "C:\\Windows\\WinSxS",
        "C:\\Windows\\Fonts",
        "C:\\Program Files",
        "C:\\Program Files (x86)",
        "C:\\Program Files\\Common Files",
        "C:\\ProgramData",
        "C:\\ProgramData\\Microsoft",
        "C:\\Users",
        "C:\\Users\\Sean",
        "C:\\Users\\Public",
        "C:\\Users\\Sean\\AppData\\Roaming",
        "C:\\Users\\Sean\\AppData\\Local",
        "C:\\Users\\Sean\\Documents",
        "C:\\Users\\Sean\\Desktop",
        "C:\\$Recycle.Bin",
        "C:\\System Volume Information",
    ];

    const KEYS: &[&str] = &[
        "HKLM",
        "HKCU",
        "HKCR",
        "HKU",
        "HKLM\\SOFTWARE",
        "HKLM\\SYSTEM",
        "HKLM\\SOFTWARE\\Microsoft",
        "HKLM\\SOFTWARE\\Classes",
        "HKLM\\SOFTWARE\\WOW6432Node",
        "HKLM\\SYSTEM\\CurrentControlSet",
        "HKLM\\SYSTEM\\ControlSet001\\Services",
        "HKCU\\Software",
        "HKCU\\Software\\Classes",
        "HKLM\\SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\Run",
        "HKLM\\SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\Uninstall",
    ];

    fn plan_of(actions: Vec<Action>) -> Plan {
        Plan {
            items: actions
                .into_iter()
                .map(|action| PlanItem {
                    action,
                    selected: true,
                    exists: true,
                    note: String::new(),
                })
                .collect(),
            info: Vec::new(),
        }
    }

    #[test]
    fn every_removal_path_refuses_critical_locations() {
        let prot = protected_dirs_with(&win_env);
        let reg = MemRegistry::default();
        for key in KEYS {
            reg.put(crate::model::KeyData::new(key));
        }
        let mut actions = Vec::new();
        for f in FOLDERS {
            actions.push(Action::DeleteTree {
                path: f.to_string(),
            });
            actions.push(Action::DeleteDir {
                path: f.to_string(),
            });
            actions.push(Action::DeleteFile {
                path: f.to_string(),
            });
        }
        for k in KEYS {
            actions.push(Action::DeleteKey {
                path: k.to_string(),
            });
        }
        let n = actions.len();
        for dry_run in [true, false] {
            let opts = ExecOptions {
                remove_unlogged: true,
                dry_run,
                use_recycle_bin: false,
                restart_explorer: false,
                close_programs: false,
            };
            let results = execute_plan_with(&plan_of(actions.clone()), &reg, None, &opts, &prot);
            assert_eq!(results.len(), n);
            for r in &results {
                assert!(!r.ok, "not refused: {}", r.description);
            }
        }
        for k in KEYS {
            assert!(reg.exists(k), "{k} was deleted");
        }
    }

    #[test]
    fn ordinary_program_locations_are_allowed() {
        let prot = protected_dirs_with(&win_env);
        for p in [
            "C:\\Program Files\\Acme",
            "C:\\Users\\Sean\\AppData\\Roaming\\Acme",
            "C:\\ProgramData\\Acme",
        ] {
            assert_eq!(
                refusal(&Action::DeleteTree { path: p.into() }, &prot),
                None,
                "{p}"
            );
        }
        assert_eq!(
            refusal(
                &Action::DeleteKey {
                    path: "HKLM\\SOFTWARE\\Acme".into()
                },
                &prot
            ),
            None
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::platform::MemRegistry;
    use crate::settings::{FileRoot, Settings};
    use crate::snapshot::{diff, take_snapshot, Progress};

    #[test]
    fn simplify_drops_careful_and_uncertain_items() {
        let item = |action, note: &str| PlanItem {
            action,
            // Uncertain matches are left unticked by the scanners.
            selected: note != "Name match",
            exists: true,
            note: note.into(),
        };
        let mut plan = Plan::default();
        plan.items = vec![
            item(
                Action::DeleteKey {
                    path: "HKCU\\Software\\Contoso".into(),
                },
                "",
            ),
            item(
                Action::DeleteKey {
                    path: "HKLM\\SOFTWARE\\Classes\\.foo".into(),
                },
                "",
            ),
            item(
                Action::StopDeleteService {
                    name: "ContosoSvc".into(),
                },
                "",
            ),
            item(
                Action::DeleteTree {
                    path: "C:\\ProgramData\\Contoso".into(),
                },
                "Name match",
            ),
            // A labelled, ticked leftover (the normal case) stays visible.
            item(
                Action::DeleteFile {
                    path: "C:\\Users\\a\\Desktop\\Contoso.lnk".into(),
                },
                "Shortcut",
            ),
        ];
        assert_eq!(simplify(&mut plan), 3);
        assert_eq!(plan.items.len(), 2);
    }

    #[test]
    fn full_install_then_revert_restores_system() {
        let tmp = crate::util::tempdir().unwrap();
        let root = tmp.path().join("pf");
        std::fs::create_dir_all(&root).unwrap();
        let reg = MemRegistry::new();
        let env = "HKLM\\SYSTEM\\CurrentControlSet\\Control\\Session Manager\\Environment";
        reg.put(KeyData::new(env).with(RegValue::string("Path", "C:\\Windows;C:\\Tools")));
        reg.put(
            KeyData::new("HKCU\\Software\\Shared")
                .with(RegValue::dword("level", 1))
                .with(RegValue::dword("old", 7)),
        );
        reg.put(KeyData::new("HKCU\\Software\\Doomed").with(RegValue::string("x", "y")));
        let settings = Settings {
            registry_roots: vec![
                "HKLM\\SYSTEM".into(),
                "HKCU\\Software".into(),
                "HKLM\\SOFTWARE".into(),
            ],
            registry_exclusions: vec![],
            file_roots: vec![FileRoot {
                path: root.to_string_lossy().into(),
                recursive: true,
            }],
            file_exclusions: vec![],
            // The test folders live under %TEMP%, which the built-in list ignores.
            auto_exclusions: false,
            ..Settings::default()
        };
        let p = Progress::default();
        let before = take_snapshot(&reg, &settings, &p).unwrap();

        // The "installer".
        reg.put(KeyData::new("HKLM\\SOFTWARE\\Acme\\Widget\\Sub").with(RegValue::string("a", "b")));
        reg.set_value(
            env,
            &RegValue {
                kind: reg_type::EXPAND_SZ,
                ..RegValue::string("Path", "C:\\Windows;C:\\Tools;C:\\Acme")
            },
        )
        .unwrap();
        reg.set_value("HKCU\\Software\\Shared", &RegValue::dword("level", 2))
            .unwrap();
        reg.set_value("HKCU\\Software\\Shared", &RegValue::dword("acme", 1))
            .unwrap();
        reg.delete_value("HKCU\\Software\\Shared", "old").unwrap();
        reg.delete_tree("HKCU\\Software\\Doomed").unwrap();
        let app = root.join("Acme");
        std::fs::create_dir_all(app.join("bin")).unwrap();
        std::fs::write(app.join("bin").join("w.exe"), "exe").unwrap();
        std::fs::write(app.join("readme.txt"), "r").unwrap();

        let after = take_snapshot(&reg, &settings, &p).unwrap();
        let cs = diff(&before, &after);

        // The user adds their own PATH entry after installing.
        let cur = reg.read_key(env).unwrap().get_string("Path").unwrap();
        reg.set_value(env, &RegValue::string("Path", &format!("{cur};D:\\Mine")))
            .unwrap();
        // ...and the app writes a settings file later.
        std::fs::write(app.join("settings.ini"), "user").unwrap();

        let plan = build_plan(&cs, &reg);
        assert!(plan
            .items
            .iter()
            .any(|i| matches!(i.action, Action::EditList { .. })));
        let broot = tmp.path().join("backups");
        let mut bw = BackupWriter::new(&broot, "revert").unwrap();
        let res = execute_plan(&plan, &reg, Some(&mut bw), &ExecOptions::default());
        assert!(res.iter().all(|r| r.ok), "{res:#?}");

        // Registry back to pre-install state, except the user's own PATH entry.
        assert!(!reg.exists("HKLM\\SOFTWARE\\Acme"));
        assert_eq!(
            reg.read_key(env).unwrap().get_string("Path").unwrap(),
            "C:\\Windows;C:\\Tools;D:\\Mine"
        );
        let shared = reg.read_key("HKCU\\Software\\Shared").unwrap();
        assert_eq!(shared.get_u32("level"), Some(1));
        assert_eq!(shared.get_u32("old"), Some(7));
        assert!(shared.get("acme").is_none());
        assert_eq!(
            reg.read_key("HKCU\\Software\\Doomed")
                .unwrap()
                .get_string("x")
                .unwrap(),
            "y"
        );
        // Logged files gone, folder with the user's later file kept.
        assert!(!app.join("bin").exists());
        assert!(!app.join("readme.txt").exists());
        assert!(app.join("settings.ini").exists());
        // Only the folder holding the user's later file remains as residue.
        let left = residue(&cs, &reg);
        assert!(
            left.items
                .iter()
                .all(|i| matches!(i.action, Action::DeleteDir { .. })),
            "{left:#?}"
        );
        assert!(residue_markdown("Acme", &left).contains("left behind"));

        // With remove_unlogged the folder goes completely.
        let res = execute_plan(
            &plan,
            &reg,
            Some(&mut bw),
            &ExecOptions {
                remove_unlogged: true,
                ..Default::default()
            },
        );
        assert!(res.iter().all(|r| r.ok), "{res:#?}");
        assert!(!app.exists());

        // And the whole thing can be rolled back from the backup.
        let b = bw.finish().unwrap();
        let res = crate::backup::restore_backup(&broot, &b.id, &reg).unwrap();
        assert!(res.iter().all(|r| r.ok), "{res:#?}");
        assert!(app.join("bin").join("w.exe").exists());
        assert!(app.join("settings.ini").exists());
        assert!(reg.exists("HKLM\\SOFTWARE\\Acme\\Widget\\Sub"));
        assert!(reg
            .read_key(env)
            .unwrap()
            .get_string("Path")
            .unwrap()
            .contains("C:\\Acme"));
    }

    #[test]
    fn protected_items_are_never_selected() {
        let reg = MemRegistry::new();
        reg.put(KeyData::new("HKLM\\SOFTWARE\\Classes"));
        let cs = ChangeSet {
            keys_added: vec![KeyData::new("HKLM\\SOFTWARE\\Classes")],
            ..Default::default()
        };
        let plan = build_plan(&cs, &reg);
        assert_eq!(plan.selected(), 0);
        let res = execute_plan(
            &Plan {
                items: vec![PlanItem {
                    selected: true,
                    ..plan.items[0].clone()
                }],
                info: vec![],
            },
            &reg,
            None,
            &ExecOptions::default(),
        );
        assert!(!res[0].ok);
        assert!(reg.exists("HKLM\\SOFTWARE\\Classes"));
    }
}

#[cfg(test)]
mod windows_component_tests {
    use super::*;

    #[test]
    fn windows_owned_entries_are_left_alone() {
        let mut cs = ChangeSet::default();
        let clsid = "HKLM\\SOFTWARE\\Classes\\CLSID\\{10964DDD-6A53-4C60-917F-7B5723014344}";
        cs.values_modified.push(ValueChange {
            key: format!("{clsid}\\InprocServer32"),
            before: RegValue::string("", "\\\\?\\C:\\Windows\\System32\\SecurityHealthAgent.dll"),
            after: RegValue::string("", "C:\\ProgramData\\Defender\\new.dll"),
        });
        cs.keys_added
            .push(KeyData::new(&format!("{clsid}\\Elevation")).with(RegValue::dword("Enabled", 1)));
        cs.keys_added.push(KeyData::new("HKLM\\SOFTWARE\\Microsoft\\Windows NT\\CurrentVersion\\Image File Execution Options\\DefenderAgentScan.exe"));
        cs.keys_added.push(
            KeyData::new("HKLM\\SOFTWARE\\7-Zip")
                .with(RegValue::string("Path", "C:\\Program Files\\7-Zip\\")),
        );
        let mut plan = Plan::default();
        for k in &cs.keys_added {
            plan.items.push(PlanItem {
                action: Action::DeleteKey {
                    path: k.path.clone(),
                },
                selected: true,
                exists: true,
                note: String::new(),
            });
        }
        plan.items.push(PlanItem {
            action: Action::SetValue {
                key: format!("{clsid}\\InprocServer32"),
                value: RegValue::string("", "x"),
            },
            selected: true,
            exists: true,
            note: String::new(),
        });
        mark_windows_components(&mut plan, &cs, "c:\\windows");
        let sel: Vec<(String, bool)> = plan
            .items
            .iter()
            .map(|i| (i.action.describe(), i.selected))
            .collect();
        assert!(
            sel.iter().any(|(d, s)| d.contains("7-Zip") && *s),
            "{sel:?}"
        );
        assert_eq!(sel.iter().filter(|(_, s)| !*s).count(), 3, "{sel:?}");
        assert_eq!(owner_root(&format!("{clsid}\\InprocServer32")), fold(clsid));
    }
}

#[cfg(test)]
mod volatile_tests {
    use super::is_volatile_parent_error;

    #[test]
    fn a_volatile_parent_is_recognised_through_context() {
        let io = std::io::Error::from_raw_os_error(1021);
        let e = anyhow::Error::new(io).context("create HKLM\\SOFTWARE\\X");
        assert!(is_volatile_parent_error(&e));
        let other = anyhow::Error::new(std::io::Error::from_raw_os_error(5)).context("create");
        assert!(!is_volatile_parent_error(&other));
    }
}
