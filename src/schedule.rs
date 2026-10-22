//! Scheduled junk cleaning through Windows Task Scheduler.
//!
//! The task runs `SetupScope.exe clean --scheduled --categories …` with the
//! categories chosen when it was set up, so the schedule needs no SetupScope
//! process running in the background.

use crate::platform::run_hidden;
use anyhow::{anyhow, Result};

/// Task name in Task Scheduler (inside an "SetupScope" folder).
pub const TASK: &str = "SetupSweep\\Junk cleaning";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Frequency {
    Daily,
    Weekly,
    Monthly,
}

impl Frequency {
    pub fn label(self) -> &'static str {
        match self {
            Frequency::Daily => "Every day",
            Frequency::Weekly => "Every week",
            Frequency::Monthly => "Every month",
        }
    }
    pub const ALL: [Frequency; 3] = [Frequency::Daily, Frequency::Weekly, Frequency::Monthly];
}

/// The schtasks arguments that create the task.
pub fn create_args(exe: &str, categories: &[&str], freq: Frequency, elevated: bool) -> Vec<String> {
    let run = format!(
        "\"{exe}\" clean --scheduled --categories {}",
        categories.join(",")
    );
    let mut a: Vec<String> = vec![
        "/Create".into(),
        "/TN".into(),
        TASK.into(),
        "/TR".into(),
        run,
        "/F".into(),
    ];
    match freq {
        Frequency::Daily => a.extend(["/SC", "DAILY", "/ST", "12:30"].map(String::from)),
        Frequency::Weekly => {
            a.extend(["/SC", "WEEKLY", "/D", "SUN", "/ST", "12:30"].map(String::from))
        }
        Frequency::Monthly => {
            a.extend(["/SC", "MONTHLY", "/D", "1", "/ST", "12:30"].map(String::from))
        }
    }
    if elevated {
        // Lets the task clean machine-wide locations too.
        a.extend(["/RL", "HIGHEST"].map(String::from));
    }
    a
}

/// Create or replace the scheduled cleaning task.
pub fn set(categories: &[&str], freq: Frequency, elevated: bool) -> Result<()> {
    if categories.is_empty() {
        return Err(anyhow!("choose at least one kind of junk to clean"));
    }
    let exe = std::env::current_exe()?.to_string_lossy().to_string();
    let args = create_args(&exe, categories, freq, elevated);
    let refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
    let (code, out) = run_hidden("schtasks", &refs)?;
    if code != 0 {
        return Err(anyhow!("Task Scheduler refused the task: {out}"));
    }
    Ok(())
}

/// Remove the scheduled task (no error when there is none).
pub fn remove() -> Result<()> {
    let (code, out) = run_hidden("schtasks", &["/Delete", "/TN", TASK, "/F"])?;
    if code != 0 && current().is_some() {
        return Err(anyhow!("could not remove the task: {out}"));
    }
    Ok(())
}

/// The current schedule, if a task exists: its frequency and categories.
pub fn current() -> Option<(Frequency, Vec<String>)> {
    let (code, xml) = run_hidden("schtasks", &["/Query", "/TN", TASK, "/XML"]).ok()?;
    if code != 0 {
        return None;
    }
    parse_task_xml(&xml)
}

pub fn parse_task_xml(xml: &str) -> Option<(Frequency, Vec<String>)> {
    let freq = if xml.contains("<ScheduleByMonth>") {
        Frequency::Monthly
    } else if xml.contains("<ScheduleByWeek>") {
        Frequency::Weekly
    } else if xml.contains("<ScheduleByDay>") {
        Frequency::Daily
    } else {
        return None;
    };
    let args = xml
        .split("<Arguments>")
        .nth(1)?
        .split("</Arguments>")
        .next()?;
    let cats = args
        .split("--categories")
        .nth(1)
        .map(|s| {
            s.trim()
                .split(|c: char| c == ',' || c.is_whitespace())
                .filter(|c| !c.is_empty())
                .map(String::from)
                .collect()
        })
        .unwrap_or_default();
    Some((freq, cats))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn task_arguments() {
        let a = create_args(
            "C:\\Tools\\SetupScope.exe",
            &["user_temp", "crash_dumps"],
            Frequency::Weekly,
            false,
        );
        assert!(a.contains(&"WEEKLY".to_string()));
        assert!(a.iter().any(|x| {
            x
            == "\"C:\\Tools\\SetupScope.exe\" clean --scheduled --categories user_temp,crash_dumps"
        }));
        assert!(!a.contains(&"HIGHEST".to_string()));
        let a = create_args("x.exe", &["user_temp"], Frequency::Monthly, true);
        assert!(a.contains(&"MONTHLY".to_string()) && a.contains(&"HIGHEST".to_string()));
    }

    #[test]
    fn reads_task_xml() {
        let xml = r#"<Task><Triggers><CalendarTrigger><ScheduleByWeek><WeeksInterval>1</WeeksInterval></ScheduleByWeek></CalendarTrigger></Triggers>
<Actions><Exec><Command>"C:\SetupScope.exe"</Command><Arguments>clean --scheduled --categories user_temp,shader_cache</Arguments></Exec></Actions></Task>"#;
        let (f, c) = parse_task_xml(xml).unwrap();
        assert_eq!(f, Frequency::Weekly);
        assert_eq!(c, vec!["user_temp", "shader_cache"]);
        assert!(parse_task_xml("<Task/>").is_none());
    }
}
