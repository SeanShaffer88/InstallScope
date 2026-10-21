//! Right-click menu manager: every context-menu handler (shell extension)
//! and menu command on files, folders, drives and the desktop, with who
//! provides it, whether its file still exists, and a reversible on/off
//! switch. Entries whose program is gone can be removed with the usual
//! review and backup.

use crate::backup::BackupWriter;
use crate::model::{fold, reg_type, RegValue};
use crate::platform::{expand_env, Registry};
use crate::revert::{Action, Plan, PlanItem};
use anyhow::Result;

/// Where a menu entry appears.
pub const SCOPES: [(&str, &str); 8] = [
    ("*", "Files"),
    ("AllFilesystemObjects", "Files and folders"),
    ("Directory", "Folders"),
    ("Folder", "Folders and libraries"),
    ("Directory\\Background", "Folder background"),
    ("DesktopBackground", "Desktop"),
    ("Drive", "Drives"),
    ("LibraryFolder\\background", "Library background"),
];

const BASES: [&str; 2] = ["HKLM\\SOFTWARE\\Classes", "HKCU\\Software\\Classes"];
pub const BLOCKED_HKLM: &str =
    "HKLM\\SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\Shell Extensions\\Blocked";
pub const BLOCKED_HKCU: &str =
    "HKCU\\Software\\Microsoft\\Windows\\CurrentVersion\\Shell Extensions\\Blocked";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// A shell extension (COM class) that adds its own items.
    Handler,
    /// A plain command (verb) that runs a program.
    Command,
}
crate::json_enum!(Kind { Handler, Command });

#[derive(Clone, Debug, PartialEq)]
pub struct MenuEntry {
    pub kind: Kind,
    /// Display name of the scope, e.g. "Files".
    pub scope: &'static str,
    pub name: String,
    /// Registry key of the entry.
    pub key: String,
    pub clsid: Option<String>,
    /// The DLL a handler loads or the program a command runs.
    pub target: String,
    pub enabled: bool,
    /// Its file (or COM class) is gone: left behind by a removed program.
    pub orphaned: bool,
    /// Part of Windows.
    pub windows: bool,
}
crate::json_struct!(MenuEntry, write_only, { kind, scope, name, key, clsid, target, enabled, orphaned, windows });

fn is_guid(s: &str) -> bool {
    s.len() == 38 && s.starts_with('{') && s.ends_with('}')
}

/// The DLL a COM class loads (64-bit, per-user, then 32-bit registration).
fn class_dll(reg: &dyn Registry, clsid: &str) -> Option<String> {
    for base in [
        "HKLM\\SOFTWARE\\Classes",
        "HKCU\\Software\\Classes",
        "HKLM\\SOFTWARE\\WOW6432Node\\Classes",
    ] {
        if let Some(v) = reg
            .read_key(&format!("{base}\\CLSID\\{clsid}\\InprocServer32"))
            .and_then(|k| k.get_string(""))
        {
            return Some(v);
        }
    }
    None
}

fn class_name(reg: &dyn Registry, clsid: &str) -> Option<String> {
    for base in ["HKLM\\SOFTWARE\\Classes", "HKCU\\Software\\Classes"] {
        if let Some(v) = reg
            .read_key(&format!("{base}\\CLSID\\{clsid}"))
            .and_then(|k| k.get_string(""))
        {
            return Some(v);
        }
    }
    None
}

/// The file a command or DLL path points at, without arguments.
pub fn file_of(command: &str) -> String {
    crate::orphans::target_path(command)
        .unwrap_or_else(|| expand_env(command.trim().trim_matches('"')))
}

fn in_windows(path: &str) -> bool {
    let f = fold(path);
    let sys = fold(&expand_env("%SystemRoot%"));
    (!sys.contains('%') && f.starts_with(&sys))
        || f.starts_with("c:\\windows\\")
        || !f.contains('\\')
}

fn blocked(reg: &dyn Registry, clsid: &str) -> bool {
    [BLOCKED_HKLM, BLOCKED_HKCU]
        .iter()
        .any(|k| reg.read_key(k).is_some_and(|k| k.get(clsid).is_some()))
}

/// Read every right-click menu entry. `exists` decides whether a file is
/// there (so tests can run without touching the disk).
pub fn scan_with(reg: &dyn Registry, exists: &dyn Fn(&str) -> bool) -> Vec<MenuEntry> {
    let mut out: Vec<MenuEntry> = Vec::new();
    for (class, scope) in SCOPES {
        for base in BASES {
            // Shell extensions.
            let hk = format!("{base}\\{class}\\shellex\\ContextMenuHandlers");
            for name in reg.subkeys(&hk) {
                let key = format!("{hk}\\{name}");
                let value = reg
                    .read_key(&key)
                    .and_then(|k| k.get_string(""))
                    .unwrap_or_default();
                let clsid = if is_guid(&value) {
                    value.clone()
                } else if is_guid(&name) {
                    name.clone()
                } else {
                    continue;
                };
                let dll = class_dll(reg, &clsid);
                let target = dll.as_deref().map(file_of).unwrap_or_default();
                let display = if is_guid(&name) {
                    class_name(reg, &clsid).unwrap_or_else(|| name.clone())
                } else {
                    name.clone()
                };
                let windows = !target.is_empty() && in_windows(&target);
                out.push(MenuEntry {
                    kind: Kind::Handler,
                    scope,
                    name: display,
                    key,
                    enabled: !blocked(reg, &clsid),
                    orphaned: dll.is_none() || (!windows && !target.is_empty() && !exists(&target)),
                    windows,
                    target,
                    clsid: Some(clsid),
                });
            }
            // Plain commands.
            let sk = format!("{base}\\{class}\\shell");
            for verb in reg.subkeys(&sk) {
                let key = format!("{sk}\\{verb}");
                let Some(cmd) = reg
                    .read_key(&format!("{key}\\command"))
                    .and_then(|k| k.get_string(""))
                else {
                    continue;
                };
                let k = reg.read_key(&key);
                let label = k
                    .as_ref()
                    .and_then(|k| k.get_string("MUIVerb").or_else(|| k.get_string("")))
                    .filter(|l| !l.starts_with('@'))
                    .map(|l| l.replace('&', ""))
                    .unwrap_or_else(|| verb.clone());
                let target = file_of(&cmd);
                let windows = in_windows(&target);
                out.push(MenuEntry {
                    kind: Kind::Command,
                    scope,
                    name: label,
                    enabled: k.as_ref().is_none_or(|k| k.get("LegacyDisable").is_none()),
                    orphaned: !windows && !target.is_empty() && !exists(&target),
                    windows,
                    target,
                    clsid: None,
                    key,
                });
            }
        }
    }
    out
}

pub fn scan(reg: &dyn Registry) -> Vec<MenuEntry> {
    // Demo builds (screenshots on Linux) pretend the demo programs' files exist.
    let demo = !cfg!(windows) && std::env::var_os("SETUPSWEEP_DEMO").is_some();
    scan_with(reg, &|p| demo || std::path::Path::new(p).exists())
}

/// Turn an entry on or off, reversibly: handlers through Windows' list of
/// blocked shell extensions, commands with the LegacyDisable flag.
pub fn set_enabled(
    reg: &dyn Registry,
    e: &MenuEntry,
    on: bool,
    machine: bool,
    mut backup: Option<&mut BackupWriter>,
) -> Result<()> {
    match (e.kind, &e.clsid) {
        (Kind::Handler, Some(clsid)) => {
            let lists: Vec<&str> = if on {
                vec![BLOCKED_HKLM, BLOCKED_HKCU]
            } else if machine {
                vec![BLOCKED_HKLM]
            } else {
                vec![BLOCKED_HKCU]
            };
            for list in lists {
                if let Some(b) = backup.as_deref_mut() {
                    if !reg.exists(list) {
                        b.add_key_absent(list)?;
                    }
                    b.add_value(reg, list, clsid)?;
                }
                if on {
                    if reg.read_key(list).is_some_and(|k| k.get(clsid).is_some()) {
                        reg.delete_value(list, clsid)?;
                    }
                } else {
                    reg.create_key(list)?;
                    reg.set_value(list, &RegValue::string(clsid, &e.name))?;
                }
            }
            Ok(())
        }
        _ => {
            if let Some(b) = backup {
                b.add_value(reg, &e.key, "LegacyDisable")?;
            }
            if on {
                reg.delete_value(&e.key, "LegacyDisable")
            } else {
                reg.set_value(
                    &e.key,
                    &RegValue {
                        name: "LegacyDisable".into(),
                        kind: reg_type::SZ,
                        data: crate::model::encode_utf16z(""),
                    },
                )
            }
        }
    }
}

/// A removal plan for entries left behind by removed programs (and their
/// COM classes), to go through the usual review and backup.
pub fn orphan_plan(entries: &[MenuEntry]) -> Plan {
    let mut plan = Plan::default();
    let mut seen = std::collections::BTreeSet::new();
    for e in entries.iter().filter(|e| e.orphaned && !e.windows) {
        if seen.insert(fold(&e.key)) {
            plan.items.push(PlanItem {
                action: Action::DeleteKey {
                    path: e.key.clone(),
                },
                selected: true,
                exists: true,
                note: format!(
                    "Right-click entry \"{}\" ({}) whose program is gone",
                    e.name, e.scope
                ),
            });
        }
    }
    plan
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::KeyData;
    use crate::platform::MemRegistry;

    fn reg() -> MemRegistry {
        let r = MemRegistry::new();
        let put = |k: KeyData| r.put(k);
        put(
            KeyData::new("HKLM\\SOFTWARE\\Classes\\*\\shellex\\ContextMenuHandlers\\7-Zip").with(
                RegValue::string("", "{23170F69-40C1-278A-1000-000100020000}"),
            ),
        );
        put(
            KeyData::new("HKLM\\SOFTWARE\\Classes\\CLSID\\{23170F69-40C1-278A-1000-000100020000}")
                .with(RegValue::string("", "7-Zip Shell Extension")),
        );
        put(KeyData::new("HKLM\\SOFTWARE\\Classes\\CLSID\\{23170F69-40C1-278A-1000-000100020000}\\InprocServer32").with(RegValue::string("", "C:\\Program Files\\7-Zip\\7-zip.dll")));
        put(KeyData::new(
            "HKLM\\SOFTWARE\\Classes\\Directory\\shellex\\ContextMenuHandlers\\OldTool",
        )
        .with(RegValue::string(
            "",
            "{11111111-2222-3333-4444-555555555555}",
        )));
        put(
            KeyData::new("HKCU\\Software\\Classes\\Directory\\Background\\shell\\OpenCode")
                .with(RegValue::string("", "Open with &Code")),
        );
        put(KeyData::new(
            "HKCU\\Software\\Classes\\Directory\\Background\\shell\\OpenCode\\command",
        )
        .with(RegValue::string("", "\"C:\\Gone\\Code.exe\" \"%V\"")));
        r
    }

    #[test]
    fn finds_handlers_and_commands() {
        let r = reg();
        let list = scan_with(&r, &|p| p.contains("7-Zip"));
        let zip = list.iter().find(|e| e.name == "7-Zip").unwrap();
        assert_eq!(
            (zip.kind, zip.scope, zip.enabled, zip.orphaned),
            (Kind::Handler, "Files", true, false)
        );
        assert!(zip.target.ends_with("7-zip.dll"));
        let old = list.iter().find(|e| e.name == "OldTool").unwrap();
        assert!(old.orphaned, "class isn't registered");
        let code = list.iter().find(|e| e.kind == Kind::Command).unwrap();
        assert_eq!(code.name, "Open with Code");
        assert_eq!(code.scope, "Folder background");
        assert!(code.orphaned);
        assert_eq!(orphan_plan(&list).items.len(), 2);
    }

    #[test]
    fn switches_entries_off_and_on() {
        let r = reg();
        let list = scan_with(&r, &|_| true);
        let zip = list.iter().find(|e| e.name == "7-Zip").unwrap().clone();
        set_enabled(&r, &zip, false, true, None).unwrap();
        assert!(
            !scan_with(&r, &|_| true)
                .iter()
                .find(|e| e.name == "7-Zip")
                .unwrap()
                .enabled
        );
        set_enabled(&r, &zip, true, true, None).unwrap();
        assert!(
            scan_with(&r, &|_| true)
                .iter()
                .find(|e| e.name == "7-Zip")
                .unwrap()
                .enabled
        );
        let code = list
            .iter()
            .find(|e| e.kind == Kind::Command)
            .unwrap()
            .clone();
        set_enabled(&r, &code, false, false, None).unwrap();
        assert!(
            !scan_with(&r, &|_| true)
                .iter()
                .find(|e| e.kind == Kind::Command)
                .unwrap()
                .enabled
        );
        set_enabled(&r, &code, true, false, None).unwrap();
        assert!(
            scan_with(&r, &|_| true)
                .iter()
                .find(|e| e.kind == Kind::Command)
                .unwrap()
                .enabled
        );
    }
}
