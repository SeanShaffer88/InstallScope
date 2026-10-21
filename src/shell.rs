//! Explorer integration: the "Install with SetupSweep" right-click command on
//! installers (.exe and .msi).
//!
//! The installer registers it for all users under HKLM; the in-app toggle
//! (used by the portable version) registers it for the current user under
//! HKCU. Both point at `SetupSweep.exe --monitor-install "<file>"`.
//!
//! On Windows 11 the command appears under "Show more options" (the classic
//! menu); the compact top-level menu only accepts packaged (MSIX) apps.

use crate::model::RegValue;
use crate::platform::Registry;
use anyhow::Result;

/// Command-line switch that starts monitoring the given installer.
pub const MONITOR_ARG: &str = "--monitor-install";

const VERB: &str = "SetupSweepInstall";
const EXTENSIONS: [&str; 2] = [".exe", ".msi"];

fn verb_key(root: &str, ext: &str) -> String {
    format!("{root}\\Software\\Classes\\SystemFileAssociations\\{ext}\\shell\\{VERB}")
}

/// The command Explorer runs for the menu entry.
pub fn command_line(exe: &str) -> String {
    format!("\"{exe}\" {MONITOR_ARG} \"%1\"")
}

/// Add the menu entry. `all_users` writes to HKLM (needs admin), otherwise HKCU.
pub fn register(reg: &dyn Registry, exe: &str, label: &str, all_users: bool) -> Result<()> {
    let root = if all_users { "HKLM" } else { "HKCU" };
    for ext in EXTENSIONS {
        let key = verb_key(root, ext);
        reg.create_key(&key)?;
        reg.set_value(&key, &RegValue::string("MUIVerb", label))?;
        reg.set_value(&key, &RegValue::string("Icon", &format!("\"{exe}\",0")))?;
        reg.create_key(&format!("{key}\\command"))?;
        reg.set_value(
            &format!("{key}\\command"),
            &RegValue::string("", &command_line(exe)),
        )?;
    }
    Ok(())
}

/// Remove the menu entry from the given scope (missing keys are fine).
pub fn unregister(reg: &dyn Registry, all_users: bool) -> Result<()> {
    let root = if all_users { "HKLM" } else { "HKCU" };
    for ext in EXTENSIONS {
        let key = verb_key(root, ext);
        if reg.exists(&key) {
            reg.delete_tree(&key)?;
        }
    }
    Ok(())
}

/// Whether the per-user or per-machine entry exists.
pub fn is_registered(reg: &dyn Registry, all_users: bool) -> bool {
    let root = if all_users { "HKLM" } else { "HKCU" };
    EXTENSIONS
        .iter()
        .all(|ext| reg.exists(&format!("{}\\command", verb_key(root, ext))))
}

/// Pull the installer path out of the program's own command-line arguments.
pub fn monitor_target(args: &[String]) -> Option<String> {
    let i = args.iter().position(|a| a == MONITOR_ARG)?;
    args.get(i + 1)
        .map(|s| s.trim_matches('"').to_string())
        .filter(|s| !s.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::platform::MemRegistry;

    #[test]
    fn register_and_unregister() {
        let reg = MemRegistry::new();
        assert!(!is_registered(&reg, false));
        register(
            &reg,
            "C:\\Tools\\SetupSweep.exe",
            "Install with SetupSweep",
            false,
        )
        .unwrap();
        assert!(is_registered(&reg, false));
        assert!(!is_registered(&reg, true));
        let cmd = reg
            .read_key("HKCU\\Software\\Classes\\SystemFileAssociations\\.msi\\shell\\SetupSweepInstall\\command")
            .unwrap()
            .get_string("")
            .unwrap();
        assert_eq!(
            cmd,
            "\"C:\\Tools\\SetupSweep.exe\" --monitor-install \"%1\""
        );
        unregister(&reg, false).unwrap();
        assert!(!is_registered(&reg, false));
        unregister(&reg, false).unwrap(); // idempotent
    }

    #[test]
    fn parses_launch_argument() {
        let args: Vec<String> = ["--monitor-install", "C:\\Downloads\\setup.exe"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert_eq!(monitor_target(&args).unwrap(), "C:\\Downloads\\setup.exe");
        assert!(monitor_target(&["--monitor-install".to_string()]).is_none());
        assert!(monitor_target(&[]).is_none());
    }
}
