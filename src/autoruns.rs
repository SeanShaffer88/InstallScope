//! Autostart locations beyond Run keys, Startup folders, services and
//! scheduled tasks: the places Sysinternals Autoruns covers that programs
//! (and malware) use to start or load code automatically.
//!
//! Each entry carries the [`Action`] that would remove it, so removals go
//! through the usual review, risk labels and backup.

use crate::leftovers::DeepIndex;
use crate::model::{fold, RegValue};
use crate::platform::{expand_env, Registry};
use crate::revert::Action;

#[derive(Clone, Debug, PartialEq)]
pub struct AutoEntry {
    /// What kind of autostart this is (translation key).
    pub kind: &'static str,
    /// Short name: value name, program name or extension name.
    pub name: String,
    /// The command or file it runs or loads.
    pub target: String,
    /// Registry location, for display.
    pub location: String,
    /// The target is part of Windows (under the Windows folder).
    pub windows: bool,
    /// How to remove it; None for entries SetupScope only shows.
    pub removal: Option<Action>,
}
crate::json_struct!(AutoEntry, write_only, { kind, name, target, location, windows, removal });

const WINLOGON: &str = "HKLM\\SOFTWARE\\Microsoft\\Windows NT\\CurrentVersion\\Winlogon";
const IFEO: &str =
    "HKLM\\SOFTWARE\\Microsoft\\Windows NT\\CurrentVersion\\Image File Execution Options";
const SESSION_MANAGER: &str = "HKLM\\SYSTEM\\CurrentControlSet\\Control\\Session Manager";

fn in_windows(target: &str) -> bool {
    let sys = fold(&expand_env("%SystemRoot%"));
    let t = fold(&expand_env(target));
    let t = t.trim_start_matches('"');
    (!sys.contains('%') && t.starts_with(&sys))
        || t.starts_with("c:\\windows\\")
        || t.starts_with("\\systemroot\\")
        || t.starts_with("system32\\")
        || !t.contains('\\')
}

/// Server file of a COM class (64-bit view first).
fn clsid_target(reg: &dyn Registry, clsid: &str) -> Option<String> {
    for base in [
        "HKLM\\SOFTWARE\\Classes",
        "HKCU\\Software\\Classes",
        "HKLM\\SOFTWARE\\WOW6432Node\\Classes",
    ] {
        for srv in ["InprocServer32", "LocalServer32"] {
            if let Some(t) = reg
                .read_key(&format!("{base}\\CLSID\\{clsid}\\{srv}"))
                .and_then(|k| k.get_string(""))
            {
                return Some(t);
            }
        }
    }
    None
}

fn leaf(path: &str) -> String {
    path.rsplit('\\').next().unwrap_or(path).to_string()
}

/// Scan the registry for extra autostart locations.
pub fn scan(reg: &dyn Registry, ix: &DeepIndex) -> Vec<AutoEntry> {
    let mut out = Vec::new();

    // Winlogon Shell / Userinit replaced or extended.
    for (name, default) in [
        ("Shell", "explorer.exe"),
        ("Userinit", "C:\\Windows\\system32\\userinit.exe,"),
    ] {
        for key in [
            WINLOGON.to_string(),
            WINLOGON.replace("HKLM\\SOFTWARE", "HKCU\\Software"),
        ] {
            let Some(v) = reg.read_key(&key).and_then(|k| k.get_string(name)) else {
                continue;
            };
            let norm = |s: &str| fold(s).trim().trim_end_matches(',').to_string();
            let default_norm = norm(&expand_env(default));
            let current = norm(&expand_env(&v));
            let is_default = current == default_norm
                || (name == "Userinit" && current.ends_with("\\system32\\userinit.exe"));
            if !is_default {
                let removal = if key.starts_with("HKCU") {
                    // The per-user value overrides the machine one; delete it.
                    Action::DeleteValue {
                        key: key.clone(),
                        name: name.into(),
                    }
                } else {
                    Action::SetValue {
                        key: key.clone(),
                        value: RegValue::string(name, default),
                    }
                };
                out.push(AutoEntry {
                    kind: "Winlogon",
                    name: name.into(),
                    target: v,
                    location: key.clone(),
                    windows: false,
                    removal: Some(removal),
                });
            }
        }
    }

    // Active Setup: runs once per user at sign-in.
    for base in [
        "HKLM\\SOFTWARE\\Microsoft\\Active Setup\\Installed Components",
        "HKLM\\SOFTWARE\\WOW6432Node\\Microsoft\\Active Setup\\Installed Components",
    ] {
        for id in reg.subkeys(base) {
            let key = format!("{base}\\{id}");
            let Some(k) = reg.read_key(&key) else {
                continue;
            };
            let Some(stub) = k.get_string("StubPath").filter(|s| !s.trim().is_empty()) else {
                continue;
            };
            let name = k
                .get_string("")
                .filter(|s| !s.is_empty())
                .unwrap_or(id.clone());
            out.push(AutoEntry {
                kind: "Active Setup",
                windows: in_windows(&stub),
                name,
                target: stub,
                location: key.clone(),
                removal: Some(Action::DeleteKey { path: key }),
            });
        }
    }

    // Image File Execution Options "Debugger": runs instead of a program.
    for exe in reg.subkeys(IFEO) {
        let key = format!("{IFEO}\\{exe}");
        if let Some(dbg) = reg.read_key(&key).and_then(|k| k.get_string("Debugger")) {
            out.push(AutoEntry {
                kind: "Program replacement (IFEO)",
                windows: in_windows(&dbg),
                name: exe,
                target: dbg,
                location: key.clone(),
                removal: Some(Action::DeleteValue {
                    key,
                    name: "Debugger".into(),
                }),
            });
        }
    }

    // AppInit DLLs: loaded into every program that uses user32.dll.
    for key in [
        "HKLM\\SOFTWARE\\Microsoft\\Windows NT\\CurrentVersion\\Windows",
        "HKLM\\SOFTWARE\\WOW6432Node\\Microsoft\\Windows NT\\CurrentVersion\\Windows",
    ] {
        if let Some(v) = reg
            .read_key(key)
            .and_then(|k| k.get_string("AppInit_DLLs"))
            .filter(|v| !v.trim().is_empty())
        {
            out.push(AutoEntry {
                kind: "AppInit DLL",
                name: "AppInit_DLLs".into(),
                windows: false,
                target: v,
                location: key.into(),
                removal: Some(Action::SetValue {
                    key: key.into(),
                    value: RegValue::string("AppInit_DLLs", ""),
                }),
            });
        }
    }

    // Boot-time programs (before Windows starts); only report additions.
    if let Some(v) = reg.read_key(SESSION_MANAGER).and_then(|k| {
        k.values
            .values()
            .find(|v| v.name.eq_ignore_ascii_case("BootExecute"))
            .cloned()
    }) {
        let lines: Vec<String> = if v.kind == crate::model::reg_type::MULTI_SZ {
            crate::model::decode_multi_sz(&v.data)
        } else {
            v.as_string().into_iter().collect()
        };
        for line in lines {
            let l = fold(line.trim());
            if l.is_empty() || l.starts_with("autocheck autochk") {
                continue;
            }
            out.push(AutoEntry {
                kind: "Boot program",
                name: leaf(line.split_whitespace().next().unwrap_or(&line)),
                target: line.clone(),
                location: SESSION_MANAGER.into(),
                windows: false,
                // Editing a multi-string list item by item isn't offered; shown only.
                removal: None,
            });
        }
    }

    // Explorer extensions: context-menu, drag-and-drop and property-sheet handlers.
    for (key, clsid) in &ix.handlers {
        let target = clsid_target(reg, clsid).unwrap_or_default();
        if target.is_empty() {
            continue; // dangling: the orphan scan reports those
        }
        out.push(AutoEntry {
            kind: "Explorer extension",
            name: leaf(key),
            windows: in_windows(&target),
            target,
            location: key.clone(),
            removal: Some(Action::DeleteKey { path: key.clone() }),
        });
    }
    // Icon overlays and browser helper objects.
    for (base, kind) in [
        (
            "HKLM\\SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\Explorer\\ShellIconOverlayIdentifiers",
            "Icon overlay",
        ),
        (
            "HKLM\\SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\Explorer\\Browser Helper Objects",
            "Browser helper object",
        ),
        (
            "HKLM\\SOFTWARE\\WOW6432Node\\Microsoft\\Windows\\CurrentVersion\\Explorer\\Browser Helper Objects",
            "Browser helper object",
        ),
    ] {
        for sub in reg.subkeys(base) {
            let key = format!("{base}\\{sub}");
            let clsid = if sub.starts_with('{') {
                sub.clone()
            } else {
                reg.read_key(&key).and_then(|k| k.get_string("")).unwrap_or_default()
            };
            let target = clsid_target(reg, clsid.trim()).unwrap_or_default();
            out.push(AutoEntry {
                kind,
                name: sub.trim().to_string(),
                windows: !target.is_empty() && in_windows(&target),
                target,
                location: key.clone(),
                removal: Some(Action::DeleteKey { path: key }),
            });
        }
    }
    out
}

/// WMI event subscriptions that run commands (a persistence trick used by
/// some software and by malware). Shown only.
pub fn wmi_consumers() -> Vec<AutoEntry> {
    let script = "Get-CimInstance -Namespace root/subscription -ClassName CommandLineEventConsumer -ErrorAction SilentlyContinue | \
ForEach-Object { \"C`t$($_.Name)`t$($_.CommandLineTemplate)\" }; \
Get-CimInstance -Namespace root/subscription -ClassName ActiveScriptEventConsumer -ErrorAction SilentlyContinue | \
ForEach-Object { \"S`t$($_.Name)`t$($_.ScriptFileName)\" }";
    let Ok((_, out)) = crate::platform::run_hidden(
        "powershell.exe",
        &["-NoProfile", "-NonInteractive", "-Command", script],
    ) else {
        return Vec::new();
    };
    parse_wmi(&out)
}

pub fn parse_wmi(out: &str) -> Vec<AutoEntry> {
    out.lines()
        .filter_map(|l| {
            let mut it = l.splitn(3, '\t');
            let (kind, name, target) = (it.next()?, it.next()?, it.next().unwrap_or(""));
            (kind == "C" || kind == "S").then(|| AutoEntry {
                kind: "WMI event subscription",
                name: name.to_string(),
                target: target.to_string(),
                location: "root\\subscription".into(),
                windows: name.eq_ignore_ascii_case("SCM Event Log Consumer"),
                removal: None,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::KeyData;
    use crate::platform::MemRegistry;

    #[test]
    fn finds_extra_autostarts() {
        let reg = MemRegistry::new();
        reg.put(
            KeyData::new(WINLOGON)
                .with(RegValue::string("Shell", "explorer.exe, C:\\Bad\\evil.exe"))
                .with(RegValue::string(
                    "Userinit",
                    "C:\\Windows\\system32\\userinit.exe,",
                )),
        );
        reg.put(
            KeyData::new("HKLM\\SOFTWARE\\Microsoft\\Active Setup\\Installed Components\\{AAAA}")
                .with(RegValue::string("", "Contoso Helper"))
                .with(RegValue::string(
                    "StubPath",
                    "\"C:\\Program Files\\Contoso\\setup.exe\" /user",
                )),
        );
        reg.put(
            KeyData::new("HKLM\\SOFTWARE\\Microsoft\\Active Setup\\Installed Components\\{BBBB}")
                .with(RegValue::string("", "No stub")),
        );
        reg.put(
            KeyData::new(&format!("{IFEO}\\notepad.exe")).with(RegValue::string(
                "Debugger",
                "C:\\Tools\\notepad++.exe -notepadStyleCmdline -z",
            )),
        );
        reg.put(KeyData::new(&format!("{IFEO}\\calc.exe")).with(RegValue::dword("GlobalFlag", 0)));
        reg.put(
            KeyData::new("HKLM\\SOFTWARE\\Microsoft\\Windows NT\\CurrentVersion\\Windows")
                .with(RegValue::string("AppInit_DLLs", "")),
        );
        reg.put(
            KeyData::new("HKLM\\SOFTWARE\\Classes\\CLSID\\{CCCC}\\InprocServer32")
                .with(RegValue::string("", "C:\\Program Files\\Zip\\zipshell.dll")),
        );
        reg.put(
            KeyData::new("HKLM\\SOFTWARE\\Classes\\*\\shellex\\ContextMenuHandlers\\Zip")
                .with(RegValue::string("", "{CCCC}")),
        );
        reg.put(KeyData::new("HKLM\\SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\Explorer\\ShellIconOverlayIdentifiers\\ OneDrive1")
            .with(RegValue::string("", "{CCCC}")));

        let ix = DeepIndex::build(&reg, None);
        let e = scan(&reg, &ix);
        let find = |kind: &str| e.iter().filter(|x| x.kind == kind).collect::<Vec<_>>();
        let wl = find("Winlogon");
        assert_eq!(wl.len(), 1, "{e:#?}");
        assert_eq!(wl[0].name, "Shell");
        assert!(
            matches!(&wl[0].removal, Some(Action::SetValue { value, .. }) if value.as_string().as_deref() == Some("explorer.exe"))
        );
        assert_eq!(find("Active Setup").len(), 1);
        let ifeo = find("Program replacement (IFEO)");
        assert_eq!(ifeo.len(), 1);
        assert_eq!(ifeo[0].name, "notepad.exe");
        assert!(find("AppInit DLL").is_empty());
        let ext = find("Explorer extension");
        assert_eq!(ext.len(), 1);
        assert!(ext[0].target.ends_with("zipshell.dll"));
        assert_eq!(find("Icon overlay").len(), 1);
    }

    #[test]
    fn parses_wmi() {
        let e = parse_wmi("C\tUpdater\tC:\\x\\up.exe /q\nS\tScripted\tC:\\x\\a.vbs\nnoise\n");
        assert_eq!(e.len(), 2);
        assert_eq!(e[0].name, "Updater");
        assert_eq!(e[1].target, "C:\\x\\a.vbs");
    }
}
