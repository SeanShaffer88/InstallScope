//! Background auto-monitoring: a lightweight `installscope watch` process,
//! started at sign-in by Task Scheduler, keeps a baseline snapshot and checks
//! every half minute whether a new program has registered itself. When one
//! has (and the installation has settled), it takes a second snapshot and
//! saves the difference as an install log, exactly as manual monitoring
//! would, so even forgotten installations can be removed precisely.

use crate::platform::{run_hidden, Registry};
use anyhow::{anyhow, Result};
use std::collections::BTreeSet;

pub const TASK: &str = "Expunge\\Auto-monitoring";
/// Tag given to logs recorded in the background.
pub const TAG: &str = "auto";

/// Every program's uninstall key, to notice new installations cheaply.
pub fn uninstall_keys(reg: &dyn Registry) -> BTreeSet<String> {
    crate::programs::list_programs(reg, true)
        .into_iter()
        .map(|p| p.key.to_lowercase())
        .collect()
}

/// Keys present now that weren't before.
pub fn added(before: &BTreeSet<String>, now: &BTreeSet<String>) -> Vec<String> {
    now.difference(before).cloned().collect()
}

/// The schtasks arguments that start `installscope watch` at sign-in.
pub fn create_args(exe: &str) -> Vec<String> {
    [
        "/Create",
        "/TN",
        TASK,
        "/TR",
        &format!("\"{exe}\" watch"),
        "/SC",
        "ONLOGON",
        "/RL",
        "HIGHEST",
        "/F",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect()
}

pub fn enable() -> Result<()> {
    let exe = std::env::current_exe()?.to_string_lossy().to_string();
    let args = create_args(&exe);
    let refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
    let (code, out) = run_hidden("schtasks", &refs)?;
    if code != 0 {
        return Err(anyhow!("Task Scheduler refused the task: {out}"));
    }
    // Start watching now, not only after the next sign-in.
    let _ = run_hidden("schtasks", &["/Run", "/TN", TASK]);
    Ok(())
}

pub fn disable() -> Result<()> {
    let _ = run_hidden("schtasks", &["/End", "/TN", TASK]);
    let (code, out) = run_hidden("schtasks", &["/Delete", "/TN", TASK, "/F"])?;
    if code != 0 && is_enabled() {
        return Err(anyhow!("could not remove the task: {out}"));
    }
    Ok(())
}

pub fn is_enabled() -> bool {
    matches!(run_hidden("schtasks", &["/Query", "/TN", TASK]), Ok((0, _)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn notices_new_programs() {
        let r = crate::platform::MemRegistry::new();
        let root = "HKLM\\SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\Uninstall";
        r.put(
            crate::model::KeyData::new(&format!("{root}\\A"))
                .with(crate::model::RegValue::string("DisplayName", "A")),
        );
        let before = uninstall_keys(&r);
        r.put(
            crate::model::KeyData::new(&format!("{root}\\B"))
                .with(crate::model::RegValue::string("DisplayName", "B")),
        );
        let now = uninstall_keys(&r);
        assert_eq!(added(&before, &now).len(), 1);
        assert!(added(&before, &now)[0].ends_with("\\b"));
        assert!(added(&now, &before).is_empty());
        let a = create_args("C:\\InstallScope.exe");
        assert!(
            a.contains(&"ONLOGON".to_string())
                && a.contains(&"\"C:\\InstallScope.exe\" watch".to_string())
        );
    }
}
