//! System-wide orphan scan: registrations left behind by programs that were
//! removed long ago (or removed badly), found because they point to files or
//! folders that no longer exist.
//!
//! Unlike the per-program leftover scan, this doesn't need to know which
//! program a registration belonged to. To stay safe it never reports
//! anything that points inside the Windows folder (Windows manages those
//! itself, and optional features legitimately register missing files), nor
//! targets it can't judge: bare file names resolved through the search path,
//! network paths, and paths with unexpanded variables.

use crate::leftovers::DeepIndex;
use crate::model::fold;
use crate::platform::{expand_env, Registry};
use crate::revert::{Action, Plan, PlanItem};
use std::collections::BTreeSet;

/// Extract the file a registration value points to: the program of a
/// command line, or a quoted/unquoted path; `None` when it can't be judged.
pub fn target_path(value: &str) -> Option<String> {
    let v = expand_env(value.trim());
    let prog = if let Some(rest) = v.strip_prefix('"') {
        rest.split('"').next().unwrap_or_default().to_string()
    } else {
        // Unquoted: cut after a known file extension (arguments may follow);
        // otherwise the whole value is a path (a folder, as in PATH).
        let lower = v.to_lowercase();
        const EXTS: [&str; 13] = [
            ".exe", ".dll", ".ocx", ".cpl", ".com", ".bat", ".cmd", ".msi", ".sys", ".scr", ".ax",
            ".tlb", ".olb",
        ];
        let mut cut = None;
        for ext in EXTS {
            let mut from = 0;
            while let Some(i) = lower[from..].find(ext) {
                let end = from + i + ext.len();
                let next = lower.as_bytes().get(end).copied();
                if matches!(next, None | Some(b' ') | Some(b',')) {
                    cut = Some(cut.map_or(end, |c: usize| c.min(end)));
                    break;
                }
                from = end;
            }
        }
        match cut {
            Some(end) => v[..end].to_string(),
            None => v.clone(),
        }
    };
    // A path to a DLL entry or icon index: "C:\x\y.dll,0" / "C:\x\y.exe,-101".
    let prog = match prog.rsplit_once(',') {
        Some((p, idx))
            if idx
                .trim()
                .trim_start_matches('-')
                .chars()
                .all(|c| c.is_ascii_digit()) =>
        {
            p.to_string()
        }
        _ => prog,
    };
    let p = prog.trim().trim_end_matches('\\').to_string();
    if p.contains('%') {
        return None;
    }
    let b = p.as_bytes();
    // Only absolute local paths: X:\…
    if b.len() < 4 || !b[0].is_ascii_alphabetic() || b[1] != b':' || (b[2] != b'\\' && b[2] != b'/')
    {
        return None;
    }
    Some(p)
}

/// The same path as seen by 32-bit programs (Program Files (x86), SysWOW64),
/// used so 32-bit registrations aren't mistaken for missing.
fn alternatives(path: &str) -> Vec<String> {
    let f = fold(path);
    let mut out = vec![path.to_string()];
    for (a, b) in [
        ("\\program files\\", "\\program files (x86)\\"),
        ("\\program files (x86)\\", "\\program files\\"),
    ] {
        if let Some(i) = f.find(a) {
            out.push(format!("{}{}{}", &path[..i], b, &path[i + a.len()..]));
        }
    }
    out
}

/// Context for judging whether a path is gone.
pub struct Checker<'a> {
    exists: &'a dyn Fn(&str) -> bool,
    windows: String,
}

impl<'a> Checker<'a> {
    pub fn new(exists: &'a dyn Fn(&str) -> bool, windows_dir: &str) -> Self {
        Checker {
            exists,
            windows: fold(windows_dir.trim_end_matches('\\')),
        }
    }

    /// True if `value` clearly points to a file or folder that doesn't exist
    /// and isn't Windows' own.
    pub fn missing(&self, value: &str) -> bool {
        let Some(p) = target_path(value) else {
            return false;
        };
        let f = fold(&p);
        if !self.windows.is_empty()
            && (f == self.windows || f.starts_with(&format!("{}\\", self.windows)))
        {
            return false;
        }
        !alternatives(&p).iter().any(|a| (self.exists)(a))
    }
}

fn item(action: Action, note: &str) -> PlanItem {
    PlanItem {
        action,
        selected: true,
        exists: true,
        note: note.into(),
    }
}

/// Scan the whole system for registrations pointing at missing files.
pub fn find_orphans(reg: &dyn Registry, ix: &DeepIndex, check: &Checker) -> Plan {
    let mut plan = Plan::default();
    let mut seen: BTreeSet<String> = BTreeSet::new();
    let mut push = |plan: &mut Plan, action: Action, note: &str| {
        if seen.insert(fold(&action.describe())) {
            plan.items.push(item(action, note));
        }
    };

    // A non-MSI uninstall registration is considered stale only when both
    // its declared install directory and its uninstaller are clearly gone.
    for (root, source) in crate::programs::UNINSTALL_ROOTS {
        for subkey in reg.subkeys(root) {
            let path = format!("{root}\\{subkey}");
            let Some(key) = reg.read_key(&path) else {
                continue;
            };
            let Some(program) = crate::programs::InstalledProgram::from_key(&key, source) else {
                continue;
            };
            if program.is_msi
                || program.system_component
                || program.no_remove
                || program.is_hidden(&key)
                || program.name.is_empty()
                || program.install_location.is_empty()
                || !check.missing(&program.install_location)
            {
                continue;
            }
            let uninstallers = [
                program.uninstall_string.as_str(),
                program.quiet_uninstall_string.as_str(),
            ];
            let has_uninstaller = uninstallers.iter().any(|command| !command.is_empty());
            let uninstallers_missing = uninstallers
                .iter()
                .filter(|command| !command.is_empty())
                .all(|command| check.missing(command));
            if has_uninstaller && uninstallers_missing {
                push(
                    &mut plan,
                    Action::DeleteKey { path },
                    "Uninstall registration",
                );
            }
        }
    }

    // COM classes, type libraries, file types, "Open with" and verbs.
    // A class can register both servers; it is orphaned only if every one
    // of its targets is missing.
    let mut by_key: std::collections::BTreeMap<
        String,
        (String, bool, &'static str, Option<String>),
    > = Default::default();
    for r in &ix.registrations {
        let e = by_key
            .entry(fold(&r.key))
            .or_insert_with(|| (r.key.clone(), true, r.note, r.clsid.clone()));
        e.1 &= check.missing(&r.target);
    }
    let mut dead_clsids = BTreeSet::new();
    let mut dead_progids = BTreeSet::new();
    for (key, all_missing, note, clsid) in by_key.into_values() {
        if !all_missing {
            continue;
        }
        if let Some(id) = clsid {
            dead_clsids.insert(id);
        }
        if note == "File type" {
            if let Some((_, name)) = key.rsplit_once('\\') {
                dead_progids.insert(fold(name));
            }
        }
        push(&mut plan, Action::DeleteKey { path: key }, note);
    }
    for (key, default, owp) in &ix.extensions {
        if default
            .as_ref()
            .is_some_and(|v| dead_progids.contains(&fold(v)))
        {
            push(
                &mut plan,
                Action::DeleteValue {
                    key: key.clone(),
                    name: String::new(),
                },
                "File association",
            );
        }
        for name in owp {
            if dead_progids.contains(&fold(name)) {
                push(
                    &mut plan,
                    Action::DeleteValue {
                        key: format!("{key}\\OpenWithProgids"),
                        name: name.clone(),
                    },
                    "Open with entry",
                );
            }
        }
    }
    // Shell extensions loading a class that is gone or orphaned.
    for (key, id) in &ix.handlers {
        let is_guid = id.starts_with('{') && id.ends_with('}') && id.len() == 38;
        if is_guid && (dead_clsids.contains(id) || !ix.clsids.contains(id)) {
            push(
                &mut plan,
                Action::DeleteKey { path: key.clone() },
                "Shell extension",
            );
        }
    }

    // PATH entries and environment variables pointing to missing folders.
    for key in [
        "HKCU\\Environment",
        "HKLM\\SYSTEM\\CurrentControlSet\\Control\\Session Manager\\Environment",
    ] {
        let Some(k) = reg.read_key(key) else { continue };
        for v in k.values.values() {
            let Some(data) = v.as_string() else { continue };
            if v.name.eq_ignore_ascii_case("Path") {
                let remove: Vec<String> = data
                    .split(';')
                    .map(str::trim)
                    .filter(|e| !e.is_empty() && check.missing(e))
                    .map(str::to_string)
                    .collect();
                if !remove.is_empty() {
                    push(
                        &mut plan,
                        Action::EditList {
                            key: key.into(),
                            name: v.name.clone(),
                            remove,
                            add_back: Vec::new(),
                        },
                        "PATH entry",
                    );
                }
            } else if !data.contains(';')
                && !v.name.eq_ignore_ascii_case("TEMP")
                && !v.name.eq_ignore_ascii_case("TMP")
                && check.missing(&data)
            {
                push(
                    &mut plan,
                    Action::DeleteValue {
                        key: key.into(),
                        name: v.name.clone(),
                    },
                    "Environment variable",
                );
            }
        }
    }

    // App Paths entries for programs that are gone.
    for base in [
        "HKLM\\SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\App Paths",
        "HKLM\\SOFTWARE\\WOW6432Node\\Microsoft\\Windows\\CurrentVersion\\App Paths",
        "HKCU\\Software\\Microsoft\\Windows\\CurrentVersion\\App Paths",
    ] {
        for sub in reg.subkeys(base) {
            let kp = format!("{base}\\{sub}");
            if reg
                .read_key(&kp)
                .and_then(|k| k.get_string(""))
                .is_some_and(|t| check.missing(&t))
            {
                push(&mut plan, Action::DeleteKey { path: kp }, "App Paths entry");
            }
        }
    }

    // Firewall rules for programs that are gone.
    let fw = "HKLM\\SYSTEM\\CurrentControlSet\\Services\\SharedAccess\\Parameters\\FirewallPolicy\\FirewallRules";
    if let Some(k) = reg.read_key(fw) {
        for v in k.values.values() {
            let Some(rule) = v.as_string() else { continue };
            let app = rule.split('|').find_map(|part| {
                part.strip_prefix("App=")
                    .or_else(|| part.strip_prefix("app="))
            });
            if app.is_some_and(|a| check.missing(a)) {
                push(
                    &mut plan,
                    Action::DeleteValue {
                        key: fw.into(),
                        name: v.name.clone(),
                    },
                    "Firewall rule",
                );
            }
        }
    }

    // Scheduled tasks (not Microsoft's) whose program is gone.
    for task in &ix.tasks {
        if check.missing(&task.command) {
            push(
                &mut plan,
                Action::DeleteTask {
                    name: task.name.clone(),
                },
                "Scheduled task",
            );
        }
    }

    plan.items
        .sort_by(|a, b| a.action.category().cmp(b.action.category()));
    plan
}

/// Run the scan against the real system.
pub fn scan_system(reg: &dyn Registry) -> Plan {
    let tasks = expand_env("%SystemRoot%\\System32\\Tasks");
    let ix = DeepIndex::build(reg, (!tasks.contains('%')).then_some(tasks.as_str()));
    let windows = expand_env("%SystemRoot%");
    let windows = if windows.contains('%') {
        String::new()
    } else {
        windows
    };
    let exists = |p: &str| std::path::Path::new(p).exists();
    find_orphans(reg, &ix, &Checker::new(&exists, &windows))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{KeyData, RegValue};
    use crate::platform::MemRegistry;

    #[test]
    fn target_paths() {
        assert_eq!(
            target_path("\"C:\\A B\\x.exe\" /run").unwrap(),
            "C:\\A B\\x.exe"
        );
        assert_eq!(target_path("C:\\x\\y.dll,0").unwrap(), "C:\\x\\y.dll");
        assert_eq!(target_path("C:\\x\\y.exe,-101").unwrap(), "C:\\x\\y.exe");
        assert!(target_path("ole32.dll").is_none());
        assert!(target_path("\\\\server\\share\\x.exe").is_none());
        assert!(target_path("%UNSET_VARIABLE_XYZ%\\x.dll").is_none());
        assert_eq!(
            target_path("C:\\Program Files\\A B\\x.dll").unwrap(),
            "C:\\Program Files\\A B\\x.dll"
        );
        assert_eq!(
            target_path("C:\\Program Files\\Tool\\bin\\").unwrap(),
            "C:\\Program Files\\Tool\\bin"
        );
        assert_eq!(
            target_path("C:\\Program Files\\x.exe /a /b").unwrap(),
            "C:\\Program Files\\x.exe"
        );
        assert_eq!(
            target_path("\"C:\\P\\x.exe\" \"%1\"").unwrap(),
            "C:\\P\\x.exe"
        );
    }

    #[test]
    fn finds_registrations_pointing_to_missing_files() {
        let reg = MemRegistry::new();
        let classes = "HKLM\\SOFTWARE\\Classes";
        // Orphaned COM class + a shell extension that loads it.
        reg.put(
            KeyData::new(&format!(
                "{classes}\\CLSID\\{{11111111-1111-1111-1111-111111111111}}\\InprocServer32"
            ))
            .with(RegValue::string("", "C:\\Program Files\\Gone\\gone.dll")),
        );
        reg.put(
            KeyData::new(&format!("{classes}\\*\\shellex\\ContextMenuHandlers\\Gone")).with(
                RegValue::string("", "{11111111-1111-1111-1111-111111111111}"),
            ),
        );
        // Handler for a class that isn't registered at all.
        reg.put(
            KeyData::new(&format!(
                "{classes}\\Directory\\shellex\\ContextMenuHandlers\\Ghost"
            ))
            .with(RegValue::string(
                "",
                "{22222222-2222-2222-2222-222222222222}",
            )),
        );
        // Healthy COM class.
        reg.put(
            KeyData::new(&format!(
                "{classes}\\CLSID\\{{33333333-3333-3333-3333-333333333333}}\\InprocServer32"
            ))
            .with(RegValue::string("", "C:\\Program Files\\Here\\here.dll")),
        );
        // Windows' own class with a missing file: never reported.
        reg.put(
            KeyData::new(&format!(
                "{classes}\\CLSID\\{{44444444-4444-4444-4444-444444444444}}\\InprocServer32"
            ))
            .with(RegValue::string("", "C:\\Windows\\System32\\optional.dll")),
        );
        // 32-bit program registered under Program Files: exists in (x86).
        reg.put(
            KeyData::new(&format!(
                "{classes}\\CLSID\\{{55555555-5555-5555-5555-555555555555}}\\InprocServer32"
            ))
            .with(RegValue::string("", "C:\\Program Files\\Old32\\old.dll")),
        );
        // Orphaned file type + extension pointing at it.
        reg.put(
            KeyData::new(&format!("{classes}\\Gone.Document\\shell\\open\\command")).with(
                RegValue::string("", "\"C:\\Program Files\\Gone\\gone.exe\" \"%1\""),
            ),
        );
        reg.put(
            KeyData::new(&format!("{classes}\\.gone")).with(RegValue::string("", "Gone.Document")),
        );
        // PATH with one dead entry; env var to a dead folder.
        reg.put(
            KeyData::new("HKCU\\Environment")
                .with(RegValue::string(
                    "Path",
                    "C:\\Program Files\\Here;C:\\Tools\\GoneSdk\\bin;%INSTALLSCOPE_TEST_UNSET%\\bin",
                ))
                .with(RegValue::string("GONE_HOME", "C:\\Tools\\GoneSdk")),
        );
        // Dead App Paths entry and firewall rule.
        reg.put(
            KeyData::new("HKLM\\SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\App Paths\\gone.exe")
                .with(RegValue::string("", "C:\\Program Files\\Gone\\gone.exe")),
        );
        reg.put(KeyData::new("HKLM\\SYSTEM\\CurrentControlSet\\Services\\SharedAccess\\Parameters\\FirewallPolicy\\FirewallRules")
            .with(RegValue::string("{rule1}", "v2.30|Action=Allow|Dir=In|App=C:\\Program Files\\Gone\\gone.exe|Name=Gone|"))
            .with(RegValue::string("{rule2}", "v2.30|Action=Allow|Dir=In|App=C:\\Program Files\\Here\\here.exe|Name=Here|")));

        let present = [
            "C:\\Program Files\\Here",
            "C:\\Program Files\\Here\\here.dll",
            "C:\\Program Files\\Here\\here.exe",
            "C:\\Program Files (x86)\\Old32\\old.dll",
        ];
        let exists = |p: &str| present.iter().any(|x| x.eq_ignore_ascii_case(p));
        let ix = DeepIndex::build(&reg, None);
        let plan = find_orphans(&reg, &ix, &Checker::new(&exists, "C:\\Windows"));
        let got: Vec<String> = plan.items.iter().map(|i| i.action.describe()).collect();
        let has = |s: &str| got.iter().any(|g| g.contains(s));
        assert!(has("{11111111-1111-1111-1111-111111111111}"), "{got:#?}");
        assert!(has("ContextMenuHandlers\\Gone"), "{got:#?}");
        assert!(has("ContextMenuHandlers\\Ghost"), "{got:#?}");
        assert!(has("Gone.Document"), "{got:#?}");
        assert!(has(".gone"), "{got:#?}");
        assert!(has("GoneSdk\\bin"), "{got:#?}");
        assert!(has("GONE_HOME"), "{got:#?}");
        assert!(has("App Paths\\gone.exe"), "{got:#?}");
        assert!(has("{rule1}"), "{got:#?}");
        assert!(!has("{33333333"), "{got:#?}");
        assert!(!has("{44444444"), "{got:#?}");
        assert!(!has("{55555555"), "{got:#?}");
        assert!(!has("{rule2}"), "{got:#?}");
        assert!(!has("%INSTALLSCOPE_TEST_UNSET%"), "{got:#?}");
    }

    #[test]
    fn finds_only_clearly_stale_non_msi_uninstall_entries() {
        let reg = MemRegistry::new();
        let root = crate::programs::UNINSTALL_ROOTS[2].0;
        reg.put(
            KeyData::new(&format!("{root}\\GoneApp"))
                .with(RegValue::string("DisplayName", "Gone App"))
                .with(RegValue::string(
                    "InstallLocation",
                    "C:\\Program Files\\GoneApp",
                ))
                .with(RegValue::string(
                    "UninstallString",
                    "\"C:\\Program Files\\GoneApp\\uninstall.exe\" /quiet",
                )),
        );
        reg.put(
            KeyData::new(&format!("{root}\\PresentApp"))
                .with(RegValue::string("DisplayName", "Present App"))
                .with(RegValue::string(
                    "InstallLocation",
                    "C:\\Program Files\\PresentApp",
                ))
                .with(RegValue::string(
                    "UninstallString",
                    "\"C:\\Program Files\\PresentApp\\uninstall.exe\"",
                )),
        );
        reg.put(
            KeyData::new(&format!("{root}\\AlternateUninstaller"))
                .with(RegValue::string("DisplayName", "Alternate Uninstaller"))
                .with(RegValue::string(
                    "InstallLocation",
                    "C:\\Program Files\\GoneAlternate",
                ))
                .with(RegValue::string(
                    "UninstallString",
                    "C:\\Tools\\uninstall-helper.exe /remove",
                ))
                .with(RegValue::string(
                    "QuietUninstallString",
                    "C:\\Program Files\\GoneAlternate\\quiet-uninstall.exe /quiet",
                )),
        );
        reg.put(
            KeyData::new(&format!("{root}\\SystemComponent"))
                .with(RegValue::string("DisplayName", "Windows Component"))
                .with(RegValue::string("InstallLocation", "C:\\Windows\\system32"))
                .with(RegValue::string(
                    "UninstallString",
                    "C:\\Windows\\system32\\uninstall.exe",
                ))
                .with(RegValue::dword("SystemComponent", 1)),
        );
        reg.put(
            KeyData::new(&format!("{root}\\MsiProduct"))
                .with(RegValue::string("DisplayName", "MSI Product"))
                .with(RegValue::string(
                    "InstallLocation",
                    "C:\\Program Files\\MsiProduct",
                ))
                .with(RegValue::string(
                    "UninstallString",
                    "MsiExec.exe /X{11111111-1111-1111-1111-111111111111}",
                ))
                .with(RegValue::dword("WindowsInstaller", 1)),
        );

        let exists = |path: &str| {
            path.eq_ignore_ascii_case("C:\\Program Files\\PresentApp")
                || path.eq_ignore_ascii_case("C:\\Program Files\\PresentApp\\uninstall.exe")
                || path.eq_ignore_ascii_case("C:\\Tools\\uninstall-helper.exe")
        };
        let ix = DeepIndex::build(&reg, None);
        let plan = find_orphans(&reg, &ix, &Checker::new(&exists, "C:\\Windows"));
        let stale = plan.items.iter().find(|item| {
            matches!(&item.action, Action::DeleteKey { path } if path == &format!("{root}\\GoneApp"))
        });
        assert!(stale.is_some(), "{:#?}", plan.items);
        assert!(stale.unwrap().selected);
        assert!(!plan.items.iter().any(|item| {
            matches!(&item.action, Action::DeleteKey { path } if path.ends_with("\\PresentApp"))
        }));
        assert!(!plan.items.iter().any(|item| {
            matches!(&item.action, Action::DeleteKey { path } if path.ends_with("\\AlternateUninstaller"))
        }));
        assert!(!plan.items.iter().any(|item| {
            matches!(&item.action, Action::DeleteKey { path } if path.ends_with("\\SystemComponent"))
        }));
        assert!(!plan.items.iter().any(|item| {
            matches!(&item.action, Action::DeleteKey { path } if path.ends_with("\\MsiProduct"))
        }));
    }
}
