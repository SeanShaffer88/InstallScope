//! Startup impact and delayed start.
//!
//! Impact comes from Windows' own boot-performance diagnostics
//! (Microsoft-Windows-Diagnostics-Performance/Operational): event 100 records
//! how long each boot took, and event 101 names programs that slowed it down
//! and by how much.
//!
//! Delayed start moves a Run-key program to a scheduled task that starts it a
//! few minutes after you sign in; the original entry is switched off (not
//! deleted), so undoing it is exact.

use crate::platform::run_hidden;
use crate::startup::StartupEntry;
use anyhow::{anyhow, Result};
use std::collections::HashMap;

/// How much one program slows down startup, averaged over recorded boots.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Impact {
    pub boots: u32,
    pub avg_ms: u64,
    pub max_ms: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Level {
    Low,
    Medium,
    High,
}

impl Level {
    pub fn label(self) -> &'static str {
        match self {
            Level::Low => "Low",
            Level::Medium => "Medium",
            Level::High => "High",
        }
    }
}

impl Impact {
    pub fn level(&self) -> Level {
        match self.avg_ms {
            0..=299 => Level::Low,
            300..=1499 => Level::Medium,
            _ => Level::High,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct BootInfo {
    /// Duration of the most recent recorded boot, in milliseconds.
    pub last_boot_ms: Option<u64>,
    /// Average over the recorded boots.
    pub avg_boot_ms: Option<u64>,
    /// By lower-case executable file name (e.g. "onedrive.exe").
    pub apps: HashMap<String, Impact>,
}

impl BootInfo {
    /// The impact of the program a startup command runs, if measured.
    pub fn impact_of(&self, command: &str) -> Option<&Impact> {
        let (prog, _) = crate::util::split_command_line(command);
        let file = prog
            .rsplit(['\\', '/'])
            .next()
            .unwrap_or(&prog)
            .trim_matches('"')
            .to_lowercase();
        self.apps.get(&file)
    }
}

/// Read the boot diagnostics (needs administrator rights). `None` when the
/// log can't be read.
pub fn read() -> Option<BootInfo> {
    let (code, xml) = run_hidden(
        "wevtutil",
        &[
            "qe",
            "Microsoft-Windows-Diagnostics-Performance/Operational",
            "/q:*[System[(EventID=100 or EventID=101)]]",
            "/f:xml",
            "/c:400",
            "/rd:true",
        ],
    )
    .ok()?;
    if code != 0 {
        return None;
    }
    Some(parse_events(&xml))
}

fn data<'a>(event: &'a str, name: &str) -> Option<&'a str> {
    for quote in ['\'', '"'] {
        let tag = format!("<Data Name={quote}{name}{quote}>");
        if let Some(i) = event.find(&tag) {
            let rest = &event[i + tag.len()..];
            return rest.split("</Data>").next();
        }
    }
    None
}

fn event_id(event: &str) -> Option<u32> {
    let i = event.find("<EventID")?;
    let rest = &event[i..];
    let start = rest.find('>')? + 1;
    rest[start..].split('<').next()?.trim().parse().ok()
}

/// Parse `wevtutil qe … /f:xml` output (newest first).
pub fn parse_events(xml: &str) -> BootInfo {
    let mut info = BootInfo::default();
    let mut boots: Vec<u64> = Vec::new();
    let mut sums: HashMap<String, (u32, u64, u64)> = HashMap::new();
    for ev in xml.split("<Event ").skip(1) {
        match event_id(ev) {
            Some(100) => {
                if let Some(ms) = data(ev, "BootTime").and_then(|v| v.trim().parse().ok()) {
                    boots.push(ms);
                }
            }
            Some(101) => {
                let Some(name) = data(ev, "Name") else {
                    continue;
                };
                let ms: u64 = data(ev, "DegradationTime")
                    .and_then(|v| v.trim().parse().ok())
                    .unwrap_or(0);
                let e = sums.entry(name.trim().to_lowercase()).or_default();
                e.0 += 1;
                e.1 += ms;
                e.2 = e.2.max(ms);
            }
            _ => {}
        }
    }
    info.last_boot_ms = boots.first().copied();
    if !boots.is_empty() {
        info.avg_boot_ms = Some(boots.iter().sum::<u64>() / boots.len() as u64);
    }
    info.apps = sums
        .into_iter()
        .map(|(k, (n, sum, max))| {
            (
                k,
                Impact {
                    boots: n,
                    avg_ms: sum / n.max(1) as u64,
                    max_ms: max,
                },
            )
        })
        .collect();
    info
}

/// Task Scheduler folder for delayed programs.
pub const DELAY_FOLDER: &str = "SetupSweep\\Delayed";

fn task_name(entry_name: &str) -> String {
    let safe: String = entry_name
        .chars()
        .map(|c| if "\\/:*?\"<>|".contains(c) { '_' } else { c })
        .collect();
    format!("{DELAY_FOLDER}\\{safe}")
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// Task definition: run `program args` `delay_secs` after `user` signs in.
pub fn task_xml(program: &str, args: &str, user: &str, delay_secs: u32) -> String {
    let args_el = if args.trim().is_empty() {
        String::new()
    } else {
        format!("<Arguments>{}</Arguments>", xml_escape(args.trim()))
    };
    format!(
        r#"<?xml version="1.0" encoding="UTF-16"?>
<Task version="1.2" xmlns="http://schemas.microsoft.com/windows/2004/02/mit/task">
  <RegistrationInfo><Description>Started later by SetupSweep (delayed start).</Description></RegistrationInfo>
  <Triggers><LogonTrigger><Enabled>true</Enabled><UserId>{user}</UserId><Delay>PT{delay_secs}S</Delay></LogonTrigger></Triggers>
  <Principals><Principal id="Author"><UserId>{user}</UserId><LogonType>InteractiveToken</LogonType><RunLevel>LeastPrivilege</RunLevel></Principal></Principals>
  <Settings><MultipleInstancesPolicy>IgnoreNew</MultipleInstancesPolicy><DisallowStartIfOnBatteries>false</DisallowStartIfOnBatteries><StopIfGoingOnBatteries>false</StopIfGoingOnBatteries><ExecutionTimeLimit>PT0S</ExecutionTimeLimit><Priority>7</Priority></Settings>
  <Actions Context="Author"><Exec><Command>{}</Command>{args_el}</Exec></Actions>
</Task>"#,
        xml_escape(program),
        user = xml_escape(user),
    )
}

/// Names (as in the startup list) of programs currently delayed.
pub fn delayed() -> Vec<String> {
    let Ok((0, out)) = run_hidden("schtasks", &["/Query", "/FO", "CSV", "/NH"]) else {
        return Vec::new();
    };
    parse_task_list(&out)
}

pub fn parse_task_list(csv: &str) -> Vec<String> {
    let prefix = format!("\"\\{DELAY_FOLDER}\\");
    let mut v: Vec<String> = csv
        .lines()
        .filter_map(|l| l.strip_prefix(&prefix))
        .filter_map(|rest| rest.split('"').next())
        .map(String::from)
        .collect();
    v.sort();
    v.dedup();
    v
}

/// Start a Run-key program `delay_secs` after sign-in instead of at once.
pub fn delay(
    reg: &dyn crate::platform::Registry,
    entry: &StartupEntry,
    delay_secs: u32,
    backup: Option<&mut crate::backup::BackupWriter>,
) -> Result<()> {
    if entry.approved.is_none() {
        return Err(anyhow!("only startup programs can be delayed"));
    }
    if entry.all_users {
        return Err(anyhow!(
            "programs that start for all users can't be delayed for one account"
        ));
    }
    let (program, args) = crate::util::split_command_line(&entry.command);
    let program = crate::platform::expand_env(&program);
    let user = std::env::var("USERDOMAIN")
        .map(|d| format!("{d}\\{}", crate::platform::user_name()))
        .unwrap_or_else(|_| crate::platform::user_name());
    let xml = task_xml(&program, &args, &user, delay_secs);
    let file = std::env::temp_dir().join(format!("setupsweep-delay-{}.xml", std::process::id()));
    // Task Scheduler expects UTF-16 with a byte-order mark.
    let mut bytes = vec![0xFF, 0xFE];
    for u in xml.encode_utf16() {
        bytes.extend_from_slice(&u.to_le_bytes());
    }
    std::fs::write(&file, bytes)?;
    let name = task_name(&entry.name);
    let r = run_hidden(
        "schtasks",
        &[
            "/Create",
            "/TN",
            &name,
            "/XML",
            &file.to_string_lossy(),
            "/F",
        ],
    );
    let _ = std::fs::remove_file(&file);
    let (code, out) = r?;
    if code != 0 {
        return Err(anyhow!("Task Scheduler refused the task: {out}"));
    }
    // Switch the original off; it stays in place for undoing.
    if let Err(e) = crate::startup::set_enabled(reg, entry, false, backup) {
        let _ = run_hidden("schtasks", &["/Delete", "/TN", &name, "/F"]);
        return Err(e);
    }
    Ok(())
}

/// Undo [`delay`]: remove the task and switch the original entry back on.
pub fn undelay(reg: &dyn crate::platform::Registry, entry: &StartupEntry) -> Result<()> {
    let name = task_name(&entry.name);
    let (code, out) = run_hidden("schtasks", &["/Delete", "/TN", &name, "/F"])?;
    if code != 0 {
        return Err(anyhow!("could not remove the delayed task: {out}"));
    }
    crate::startup::set_enabled(reg, entry, true, None)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"<Events><Event xmlns='http://schemas.microsoft.com/win/2004/08/events/event'><System><Provider Name='Microsoft-Windows-Diagnostics-Performance'/><EventID>101</EventID></System><EventData><Data Name='Name'>OneDrive.exe</Data><Data Name='FriendlyName'>Microsoft OneDrive</Data><Data Name='TotalTime'>4200</Data><Data Name='DegradationTime'>2100</Data></EventData></Event>
<Event xmlns='http://schemas.microsoft.com/win/2004/08/events/event'><System><EventID>100</EventID></System><EventData><Data Name='BootTime'>41000</Data><Data Name='MainPathBootTime'>12000</Data></EventData></Event>
<Event xmlns='http://schemas.microsoft.com/win/2004/08/events/event'><System><EventID>101</EventID></System><EventData><Data Name='Name'>onedrive.exe</Data><Data Name='DegradationTime'>900</Data></EventData></Event>
<Event xmlns='http://schemas.microsoft.com/win/2004/08/events/event'><System><EventID>101</EventID></System><EventData><Data Name='Name'>Spotify.exe</Data><Data Name='DegradationTime'>120</Data></EventData></Event>
<Event xmlns='http://schemas.microsoft.com/win/2004/08/events/event'><System><EventID>100</EventID></System><EventData><Data Name='BootTime'>59000</Data></EventData></Event></Events>"#;

    #[test]
    fn parses_boot_events() {
        let b = parse_events(SAMPLE);
        assert_eq!(b.last_boot_ms, Some(41000));
        assert_eq!(b.avg_boot_ms, Some(50000));
        let od = &b.apps["onedrive.exe"];
        assert_eq!((od.boots, od.avg_ms, od.max_ms), (2, 1500, 2100));
        assert_eq!(od.level(), Level::High);
        assert_eq!(b.apps["spotify.exe"].level(), Level::Low);
        let hit =
            b.impact_of("\"C:\\Program Files\\Microsoft OneDrive\\OneDrive.exe\" /background");
        assert_eq!(hit.map(|i| i.boots), Some(2));
        assert!(b.impact_of("C:\\x\\other.exe").is_none());
    }

    #[test]
    fn delayed_task_definition() {
        let x = task_xml("C:\\Apps\\a&b.exe", "--min", "PC\\alex", 120);
        assert!(x.contains("<Delay>PT120S</Delay>"));
        assert!(x.contains("<Command>C:\\Apps\\a&amp;b.exe</Command>"));
        assert!(x.contains("<Arguments>--min</Arguments>"));
        assert!(x.contains("<UserId>PC\\alex</UserId>"));
        assert!(!task_xml("a.exe", "", "u", 60).contains("<Arguments>"));
        let csv = "\"\\SetupSweep\\Delayed\\Spotify\",\"N/A\",\"Ready\"\r\n\"\\Microsoft\\Other\",\"x\",\"Ready\"\r\n\"\\SetupSweep\\Delayed\\Discord\",\"N/A\",\"Ready\"";
        assert_eq!(parse_task_list(csv), vec!["Discord", "Spotify"]);
        assert_eq!(task_name("a/b"), "SetupSweep\\Delayed\\a_b");
    }
}
