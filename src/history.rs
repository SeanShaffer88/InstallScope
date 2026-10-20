//! Slowdown Detective — history capture.
//!
//! The Detective answers "why is my PC slower than it was?" by diffing what
//! changed between two dates. That answer is only as good as the history
//! behind it, so this module's job is to *record* the raw material durably:
//! dated events for program installs, updates (version changes) and removals,
//! and for autostart entries added, removed or changed.
//!
//! ## Where events come from
//!
//! Nothing here monitors anything new. [`HistoryStore::capture`] is handed
//! three things InstallScope already sees — the saved install logs, the
//! installed-program list and the startup list — and derives events from
//! them:
//!
//! - **Install logs** carry exact timestamps and versions, so a first log for
//!   a program is an install and a later log with a different version is an
//!   update. A program with logs that has vanished from the installed list
//!   (and from the stored baseline) is a removal.
//! - **Diffs against the stored baseline** (the program and autostart state
//!   as of the previous capture) catch everything monitoring missed:
//!   unmonitored installs, updates and removals and autostart changes. Their
//!   timestamp is when the change was *observed*, which is the honest claim.
//!
//! The very first capture only establishes the baseline and records no
//! events: InstallScope cannot know when the programs already present were
//! installed, and inventing dates would poison every later report. The store
//! remembers when capture began (`started`) so reports can say how far back
//! the history honestly goes.
//!
//! ## On-disk format (`history.json`, beside `settings.json`)
//!
//! ```json
//! { "version": 1, "started": 1760000000, "events": [...],
//!   "programs": [...], "autostart": [...], "seen_logs": [...] }
//! ```
//!
//! `version` is the schema version ([`SCHEMA_VERSION`]). Loading is tolerant
//! in the same way as the usage database: a missing or unreadable file means
//! an empty history, unknown fields are ignored and absent fields take
//! defaults, so additive changes need no migration; `migrate` is the hook
//! for the day a change is not additive. A file written by a *newer* version
//! still loads the fields this build understands, but [`HistoryStore::save`]
//! refuses to overwrite it, so newer data is never downgraded away. Writes
//! are staged through a `.tmp` file and renamed, like install logs.
//!
//! Event times: `ts` is Unix seconds and `date` the local `YYYY-MM-DD` the
//! event belongs to; reports group by `date`. For log-derived events `ts` is
//! the log's local wall-clock time read as UTC — an ordering key only, never
//! displayed as an instant — because a log's `created` string carries no time
//! zone. For observed events `ts` is the real capture time.

use crate::model::{fold, InstallLog};
use crate::programs::InstalledProgram;
use crate::startup::StartupEntry;
use crate::time::{Date, DateTime};
use anyhow::{Context, Result};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

/// The on-disk schema version written by this build.
pub const SCHEMA_VERSION: u32 = 1;

/// Oldest events are dropped beyond this many, so the file stays bounded no
/// matter how long the history runs. A busy PC produces dozens of events a
/// year; this is centuries of headroom.
pub const MAX_EVENTS: usize = 10_000;

/// What happened, in the vocabulary of the report.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum HistoryEventKind {
    ProgramInstalled,
    ProgramUpdated,
    ProgramRemoved,
    AutostartAdded,
    AutostartRemoved,
    AutostartChanged,
}
crate::json_enum!(HistoryEventKind {
    ProgramInstalled,
    ProgramUpdated,
    ProgramRemoved,
    AutostartAdded,
    AutostartRemoved,
    AutostartChanged
});

impl Default for HistoryEventKind {
    fn default() -> Self {
        HistoryEventKind::ProgramInstalled
    }
}

impl HistoryEventKind {
    /// The plain-language label used in listings and reports.
    pub fn label(self) -> &'static str {
        match self {
            HistoryEventKind::ProgramInstalled => "Installed",
            HistoryEventKind::ProgramUpdated => "Updated",
            HistoryEventKind::ProgramRemoved => "Removed",
            HistoryEventKind::AutostartAdded => "Autostart added",
            HistoryEventKind::AutostartRemoved => "Autostart removed",
            HistoryEventKind::AutostartChanged => "Autostart changed",
        }
    }

    /// Whether the event is about a program (vs. an autostart entry).
    pub fn is_program(self) -> bool {
        matches!(
            self,
            HistoryEventKind::ProgramInstalled
                | HistoryEventKind::ProgramUpdated
                | HistoryEventKind::ProgramRemoved
        )
    }
}

/// One dated change. See the module docs for the meaning of `ts` and `date`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct HistoryEvent {
    /// Unix seconds (an ordering key; see the module docs).
    pub ts: i64,
    /// Local `YYYY-MM-DD` — the day reports group and filter by.
    pub date: String,
    pub kind: HistoryEventKind,
    /// Program name, or autostart entry name.
    pub subject: String,
    /// Version before the change (updates and removals).
    pub from_version: String,
    /// Version after the change (installs and updates).
    pub to_version: String,
    /// The autostart entry's command line, where the event has one.
    pub target: String,
    /// Free-text detail, e.g. how an autostart entry changed.
    pub detail: String,
}
crate::json_struct!(HistoryEvent, default, {
    ts, date, kind, subject,
    from_version [default], to_version [default], target [default], detail [default]
});

impl HistoryEvent {
    fn new(ts: i64, kind: HistoryEventKind, subject: &str) -> HistoryEvent {
        HistoryEvent {
            ts,
            date: DateTime::from_unix_local(ts).date.to_string(),
            kind,
            subject: subject.to_string(),
            ..Default::default()
        }
    }
}

/// One installed program as the baseline remembers it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ProgramState {
    /// The program's uninstall key; empty for programs known only by name.
    pub key: String,
    pub name: String,
    pub version: String,
}
crate::json_struct!(ProgramState, default, { key, name, version });

impl ProgramState {
    pub fn of(p: &InstalledProgram) -> ProgramState {
        ProgramState {
            key: p.key.clone(),
            name: p.name.clone(),
            version: p.version.clone(),
        }
    }

    /// The identity two states are compared by: the uninstall key when there
    /// is one (stable across renames), else the folded name.
    fn id(&self) -> String {
        if self.key.trim().is_empty() {
            fold(&self.name)
        } else {
            fold(&self.key)
        }
    }
}

/// One autostart entry as the baseline remembers it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AutostartState {
    pub name: String,
    pub command: String,
    /// Where the entry lives (a registry key path or a Startup folder).
    pub location: String,
    pub enabled: bool,
}
crate::json_struct!(AutostartState, default, { name, command, location, enabled });

impl AutostartState {
    pub fn of(e: &StartupEntry) -> AutostartState {
        AutostartState {
            name: e.name.clone(),
            command: e.command.clone(),
            location: e.location.clone(),
            enabled: e.enabled,
        }
    }

    /// Entries are identified by where they live plus their name: the same
    /// name in a Run key and in the Startup folder is two entries.
    fn id(&self) -> String {
        format!("{}\n{}", fold(&self.location), fold(&self.name))
    }
}

/// The versioned history file: the events plus the baselines the next
/// capture diffs against. Serialized as `history.json`; see the module docs.
#[derive(Clone, Debug, PartialEq)]
pub struct HistoryStore {
    /// Schema version of the file this was loaded from (or will be saved as).
    pub version: u32,
    /// When capture began (Unix seconds); 0 until the first capture.
    pub started: i64,
    /// Every recorded event, oldest first.
    pub events: Vec<HistoryEvent>,
    /// Program state as of the last capture.
    pub programs: Vec<ProgramState>,
    /// Autostart state as of the last capture.
    pub autostart: Vec<AutostartState>,
    /// Install-log ids already turned into events.
    pub seen_logs: Vec<String>,
}
crate::json_struct!(HistoryStore, default, {
    version, started, events, programs, autostart, seen_logs
});

impl Default for HistoryStore {
    fn default() -> Self {
        HistoryStore {
            version: SCHEMA_VERSION,
            started: 0,
            events: Vec::new(),
            programs: Vec::new(),
            autostart: Vec::new(),
            seen_logs: Vec::new(),
        }
    }
}

impl HistoryStore {
    pub fn path(root: &Path) -> PathBuf {
        root.join("history.json")
    }

    /// Load the history from the data folder. A missing or unreadable file
    /// is an empty history, never an error — capture can always start over.
    pub fn load(root: &Path) -> HistoryStore {
        let mut s: HistoryStore = std::fs::read(Self::path(root))
            .ok()
            .and_then(|b| crate::json::compat::from_slice(&b).ok())
            .unwrap_or_default();
        s.migrate();
        s
    }

    /// Upgrade an older file in memory. Version 1 is the first format, so
    /// there is nothing to upgrade yet; additive changes (new fields with
    /// defaults) are absorbed by the reader and need no arm here.
    fn migrate(&mut self) {
        if self.version < SCHEMA_VERSION {
            self.version = SCHEMA_VERSION;
        }
    }

    /// Save atomically (stage, then rename), like install logs. A file from
    /// a newer version of InstallScope is never overwritten.
    pub fn save(&self, root: &Path) -> Result<()> {
        if self.version > SCHEMA_VERSION {
            anyhow::bail!(
                "the history file is format v{} but this build only knows v{SCHEMA_VERSION}; \
                 leaving it untouched",
                self.version
            );
        }
        let bytes = crate::json::compat::to_vec(self).context("encode history")?;
        let tmp = root.join("history.json.tmp");
        std::fs::write(&tmp, bytes).with_context(|| format!("write {}", tmp.display()))?;
        std::fs::rename(&tmp, Self::path(root)).context("replace history file")?;
        Ok(())
    }

    /// The local date capture began, if it has.
    pub fn started_date(&self) -> Option<String> {
        (self.started > 0).then(|| DateTime::from_unix_local(self.started).ymd())
    }

    /// Fold one observation of the system into the history and return the
    /// events it produced. The first capture only sets the baseline (see the
    /// module docs) and returns nothing.
    pub fn capture(
        &mut self,
        logs: &[InstallLog],
        programs_now: &[InstalledProgram],
        autostart_now: &[StartupEntry],
        now: i64,
    ) -> Vec<HistoryEvent> {
        let programs: Vec<ProgramState> = programs_now.iter().map(ProgramState::of).collect();
        let autostart: Vec<AutostartState> = autostart_now.iter().map(AutostartState::of).collect();
        if self.started == 0 {
            self.started = now;
            self.programs = programs;
            self.autostart = autostart;
            self.seen_logs = logs.iter().map(|l| l.id.clone()).collect();
            return Vec::new();
        }

        let seen: HashSet<String> = self.seen_logs.iter().cloned().collect();
        let mut fresh = events_from_logs(logs, &seen, &programs, &self.programs, now);
        fresh.extend(diff_programs(&self.programs, &programs, now));
        fresh.extend(diff_autostart(&self.autostart, &autostart, now));
        dedupe(&mut fresh);
        sort_events(&mut fresh);

        self.events.extend(fresh.iter().cloned());
        if self.events.len() > MAX_EVENTS {
            let excess = self.events.len() - MAX_EVENTS;
            self.events.drain(..excess);
        }
        for l in logs {
            if !seen.contains(l.id.as_str()) {
                self.seen_logs.push(l.id.clone());
            }
        }
        self.programs = programs;
        self.autostart = autostart;
        fresh
    }

    /// Events whose day falls in `from..=to` (ISO dates compare
    /// chronologically as strings), oldest first.
    pub fn events_between(&self, from: Date, to: Date) -> Vec<&HistoryEvent> {
        let (from, to) = (from.to_string(), to.to_string());
        self.events
            .iter()
            .filter(|e| e.date >= from && e.date <= to)
            .collect()
    }
}

/// Deterministic order for events: by day, then time, then kind and name.
fn sort_events(events: &mut [HistoryEvent]) {
    events.sort_by(|a, b| {
        a.date
            .cmp(&b.date)
            .then(a.ts.cmp(&b.ts))
            .then(a.kind.cmp(&b.kind))
            .then_with(|| a.subject.to_lowercase().cmp(&b.subject.to_lowercase()))
    });
}

/// Collapse exact duplicates that two sources reported for the same change
/// (e.g. an update seen both in a new install log and in the program diff).
fn dedupe(events: &mut Vec<HistoryEvent>) {
    let mut seen: HashSet<(HistoryEventKind, String, String, String, String)> = HashSet::new();
    events.retain(|e| {
        seen.insert((
            e.kind,
            fold(&e.subject),
            e.date.clone(),
            e.from_version.clone(),
            e.to_version.clone(),
        ))
    });
}

/// A log's identity for its program: the uninstall key's leaf when the log
/// recorded one, else the program name — the same rule as `ProgramState::id`,
/// applied to the other shape.
fn log_program_id(log: &InstallLog) -> String {
    if log.uninstall_key.trim().is_empty() {
        fold(&log.name)
    } else {
        fold(crate::platform::leaf_name(&log.uninstall_key))
    }
}

/// A program-list identity reduced to the same space as `log_program_id`:
/// the key's leaf, or the name when there is no key.
fn state_id_for_logs(p: &ProgramState) -> String {
    if p.key.trim().is_empty() {
        fold(&p.name)
    } else {
        fold(crate::platform::leaf_name(&p.key))
    }
}

/// Parse an install log's `created` string (`YYYY-MM-DD HH:MM:SS`, local)
/// into (ordering timestamp, date). The timestamp reads the wall clock as
/// UTC — an ordering key only, per the module docs. `None` when the string
/// is not in that shape; such a log contributes no events.
fn parse_created(s: &str) -> Option<(i64, String)> {
    let date = Date::parse_ymd(s)?;
    let mut secs = 0i64;
    let b = s.as_bytes();
    if s.len() >= 19 && b[10] == b' ' && b[13] == b':' && b[16] == b':' {
        let h: i64 = s[11..13].parse().ok()?;
        let m: i64 = s[14..16].parse().ok()?;
        let sec: i64 = s[17..19].parse().ok()?;
        if h > 23 || m > 59 || sec > 60 {
            return None;
        }
        secs = h * 3600 + m * 60 + sec;
    }
    let days = date.days_since(Date::from_days_since_epoch(0));
    Some((days * 86_400 + secs, date.to_string()))
}

/// Events derivable from install logs alone: installs and updates from the
/// logs not in `seen`, and removals for logged programs that are gone from
/// both the current program list and the baseline (programs in the baseline
/// get their removal from `diff_programs` instead, so nothing is reported
/// twice). Pure; `now` timestamps the removals, which are observations.
pub fn events_from_logs(
    logs: &[InstallLog],
    seen: &HashSet<String>,
    installed_now: &[ProgramState],
    baseline: &[ProgramState],
    now: i64,
) -> Vec<HistoryEvent> {
    let mut out = Vec::new();
    // Group the unseen logs by program, oldest first within a program.
    let mut groups: HashMap<String, Vec<&InstallLog>> = HashMap::new();
    for log in logs {
        if !seen.contains(log.id.as_str()) {
            groups.entry(log_program_id(log)).or_default().push(log);
        }
    }
    let now_ids: HashSet<String> = installed_now.iter().map(state_id_for_logs).collect();
    let baseline_ids: HashSet<String> = baseline.iter().map(state_id_for_logs).collect();

    let mut ids: Vec<String> = groups.keys().cloned().collect();
    ids.sort();
    for id in &ids {
        let mut group = groups.remove(id).unwrap_or_default();
        group.sort_by(|a, b| a.created.cmp(&b.created));
        let mut prev_version: Option<&str> = None;
        for log in &group {
            let Some((ts, date)) = parse_created(&log.created) else {
                continue;
            };
            let version = log.version.trim();
            match prev_version {
                None => {
                    let mut e =
                        HistoryEvent::new(ts, HistoryEventKind::ProgramInstalled, &log.name);
                    e.date = date;
                    e.to_version = version.to_string();
                    out.push(e);
                }
                Some(prev) if prev != version && !version.is_empty() => {
                    let mut e = HistoryEvent::new(ts, HistoryEventKind::ProgramUpdated, &log.name);
                    e.date = date;
                    e.from_version = prev.to_string();
                    e.to_version = version.to_string();
                    out.push(e);
                }
                _ => {}
            }
            if !version.is_empty() {
                prev_version = Some(log.version.as_str());
            }
        }
        // Removal: the program had logs, is not installed now, and is not in
        // the baseline (whose diff reports removals for everything it held).
        if !now_ids.contains(id) && !baseline_ids.contains(id) {
            if let Some(last) = group.last() {
                let mut e = HistoryEvent::new(now, HistoryEventKind::ProgramRemoved, &last.name);
                e.from_version = last.version.trim().to_string();
                e.detail = "had an install log; no longer installed".into();
                out.push(e);
            }
        }
    }
    sort_events(&mut out);
    out
}

/// Installs, updates and removals between two program states. An update is
/// a version change for the same program — same key, or same name when
/// there is no key — which is what keeps an update from looking like a
/// removal plus an install.
pub fn diff_programs(
    before: &[ProgramState],
    after: &[ProgramState],
    ts: i64,
) -> Vec<HistoryEvent> {
    let before: HashMap<String, &ProgramState> = before.iter().map(|p| (p.id(), p)).collect();
    let after: HashMap<String, &ProgramState> = after.iter().map(|p| (p.id(), p)).collect();
    let mut out = Vec::new();
    for (id, now) in &after {
        match before.get(id) {
            None => {
                let mut e = HistoryEvent::new(ts, HistoryEventKind::ProgramInstalled, &now.name);
                e.to_version = now.version.clone();
                out.push(e);
            }
            Some(old) if old.version.trim() != now.version.trim() => {
                let mut e = HistoryEvent::new(ts, HistoryEventKind::ProgramUpdated, &now.name);
                e.from_version = old.version.clone();
                e.to_version = now.version.clone();
                out.push(e);
            }
            _ => {}
        }
    }
    for (id, old) in &before {
        if !after.contains_key(id) {
            let mut e = HistoryEvent::new(ts, HistoryEventKind::ProgramRemoved, &old.name);
            e.from_version = old.version.clone();
            out.push(e);
        }
    }
    sort_events(&mut out);
    out
}

/// Autostart entries added, removed or changed between two scans. A change
/// is a different command line or a different enabled state for the same
/// entry (same location and name).
pub fn diff_autostart(
    before: &[AutostartState],
    after: &[AutostartState],
    ts: i64,
) -> Vec<HistoryEvent> {
    let before: HashMap<String, &AutostartState> = before.iter().map(|e| (e.id(), e)).collect();
    let after: HashMap<String, &AutostartState> = after.iter().map(|e| (e.id(), e)).collect();
    let mut out = Vec::new();
    for (id, now) in &after {
        match before.get(id) {
            None => {
                let mut e = HistoryEvent::new(ts, HistoryEventKind::AutostartAdded, &now.name);
                e.target = now.command.clone();
                e.detail = now.location.clone();
                out.push(e);
            }
            Some(old) => {
                let mut changes = Vec::new();
                if old.command != now.command {
                    changes.push(format!(
                        "command changed: {} → {}",
                        old.command, now.command
                    ));
                }
                if old.enabled != now.enabled {
                    changes.push(if now.enabled {
                        "turned on".to_string()
                    } else {
                        "turned off".to_string()
                    });
                }
                if !changes.is_empty() {
                    let mut e =
                        HistoryEvent::new(ts, HistoryEventKind::AutostartChanged, &now.name);
                    e.target = now.command.clone();
                    e.detail = changes.join("; ");
                    out.push(e);
                }
            }
        }
    }
    for (id, old) in &before {
        if !after.contains_key(id) {
            let mut e = HistoryEvent::new(ts, HistoryEventKind::AutostartRemoved, &old.name);
            e.target = old.command.clone();
            e.detail = old.location.clone();
            out.push(e);
        }
    }
    sort_events(&mut out);
    out
}

/// The events a `slowdown` report covers, plus the context that keeps it
/// honest: when recording began and how much history exists in total.
pub struct SlowdownReport {
    pub from: Date,
    pub to: Date,
    /// Events in range, oldest first.
    pub events: Vec<HistoryEvent>,
    /// The local date capture began; `None` before the first capture.
    pub history_started: Option<String>,
    /// Events recorded across all dates.
    pub total_events: usize,
}

impl SlowdownReport {
    /// True when there is nothing to rank: no events in range at all. The
    /// caller renders the honest empty state from the other fields.
    pub fn is_empty(&self) -> bool {
        self.events.is_empty()
    }
}

/// Build a report over `from..=to`. Pure: ranking and rendering are the
/// caller's job, with whatever measurements it can read.
pub fn report(store: &HistoryStore, from: Date, to: Date) -> SlowdownReport {
    SlowdownReport {
        from,
        to,
        events: store
            .events_between(from, to)
            .into_iter()
            .cloned()
            .collect(),
        history_started: store.started_date(),
        total_events: store.events.len(),
    }
}

/// Measured startup impact for one event, in average milliseconds, from
/// Windows' boot diagnostics, matched on the event's target command.
/// `None` means *no measurement exists* — callers must not present it as
/// zero impact. (Program events carry no command; the usage tracker's
/// per-program startup figure is the measurement for those, resolved by
/// the caller, which has the installed-program list.)
pub fn boot_impact_ms(ev: &HistoryEvent, boot: Option<&crate::boot::BootInfo>) -> Option<u64> {
    if ev.target.trim().is_empty() {
        return None;
    }
    boot?.impact_of(&ev.target).map(|i| i.avg_ms)
}

#[cfg(test)]
mod tests {
    use super::*;

    // 2026-10-16 12:00:00 UTC — a fixed "now" for capture-time events.
    const NOW: i64 = 1_792_137_600;

    fn prog(key: &str, name: &str, version: &str) -> ProgramState {
        ProgramState {
            key: key.into(),
            name: name.into(),
            version: version.into(),
        }
    }

    fn log(id: &str, name: &str, key: &str, version: &str, created: &str) -> InstallLog {
        InstallLog {
            id: id.into(),
            name: name.into(),
            version: version.into(),
            created: created.into(),
            uninstall_key: key.into(),
            ..Default::default()
        }
    }

    fn installed(p: &ProgramState) -> InstalledProgram {
        InstalledProgram {
            key: p.key.clone(),
            name: p.name.clone(),
            version: p.version.clone(),
            ..Default::default()
        }
    }

    #[test]
    fn program_diff_distinguishes_install_update_and_remove() {
        let before = vec![
            prog("HKLM\\Uninstall\\A", "App A", "1.0"),
            prog("HKLM\\Uninstall\\B", "App B", "2.0"),
            prog("HKLM\\Uninstall\\C", "App C", "3.0"),
        ];
        let after = vec![
            prog("HKLM\\Uninstall\\A", "App A", "1.0"), // unchanged
            prog("HKLM\\Uninstall\\B", "App B", "2.1"), // version change = update
            prog("HKLM\\Uninstall\\D", "App D", "4.0"), // new = install
        ];
        let ev = diff_programs(&before, &after, NOW);
        assert_eq!(ev.len(), 3, "{ev:?}");
        let upd = ev
            .iter()
            .find(|e| e.kind == HistoryEventKind::ProgramUpdated)
            .unwrap();
        assert_eq!(upd.subject, "App B");
        assert_eq!(upd.from_version, "2.0");
        assert_eq!(upd.to_version, "2.1");
        let ins = ev
            .iter()
            .find(|e| e.kind == HistoryEventKind::ProgramInstalled)
            .unwrap();
        assert_eq!(ins.subject, "App D");
        assert_eq!(ins.to_version, "4.0");
        let rem = ev
            .iter()
            .find(|e| e.kind == HistoryEventKind::ProgramRemoved)
            .unwrap();
        assert_eq!(rem.subject, "App C");
        assert_eq!(rem.from_version, "3.0");
    }

    #[test]
    fn program_diff_same_version_is_not_an_update() {
        let before = vec![prog("", "Portable Tool", "1.2")];
        let same = vec![prog("", "Portable Tool", "1.2")];
        assert!(diff_programs(&before, &same, NOW).is_empty());
        // A whitespace-only version difference is not an update either.
        let padded = vec![prog("", "Portable Tool", " 1.2 ")];
        assert!(diff_programs(&before, &padded, NOW).is_empty());
        // Keyless programs are matched by name: a rename is remove + install.
        let renamed = vec![prog("", "Portable Tool NG", "1.2")];
        let ev = diff_programs(&before, &renamed, NOW);
        assert_eq!(ev.len(), 2);
    }

    #[test]
    fn autostart_diff_covers_add_remove_and_change() {
        let entry = |name: &str, cmd: &str, on: bool| AutostartState {
            name: name.into(),
            command: cmd.into(),
            location: "HKCU\\Run".into(),
            enabled: on,
        };
        let before = vec![
            entry("Keep", "keep.exe", true),
            entry("Gone", "gone.exe", true),
            entry("Retarget", "old.exe", true),
            entry("Mute", "mute.exe", true),
        ];
        let after = vec![
            entry("Keep", "keep.exe", true),
            entry("Retarget", "new.exe", true),
            entry("Mute", "mute.exe", false),
            entry("Fresh", "fresh.exe", true),
        ];
        let ev = diff_autostart(&before, &after, NOW);
        assert_eq!(ev.len(), 4, "{ev:?}");
        let added = ev
            .iter()
            .find(|e| e.kind == HistoryEventKind::AutostartAdded)
            .unwrap();
        assert_eq!(added.subject, "Fresh");
        assert_eq!(added.target, "fresh.exe");
        let removed = ev
            .iter()
            .find(|e| e.kind == HistoryEventKind::AutostartRemoved)
            .unwrap();
        assert_eq!(removed.subject, "Gone");
        let changes: Vec<_> = ev
            .iter()
            .filter(|e| e.kind == HistoryEventKind::AutostartChanged)
            .collect();
        assert_eq!(changes.len(), 2);
        let retarget = changes.iter().find(|e| e.subject == "Retarget").unwrap();
        assert!(retarget.detail.contains("old.exe") && retarget.detail.contains("new.exe"));
        let mute = changes.iter().find(|e| e.subject == "Mute").unwrap();
        assert!(mute.detail.contains("turned off"));
        // Same name in another location is a different entry.
        let elsewhere = vec![AutostartState {
            location: "HKLM\\Run".into(),
            ..entry("Keep", "keep.exe", true)
        }];
        let before2 = vec![entry("Keep", "keep.exe", true)];
        assert_eq!(diff_autostart(&before2, &elsewhere, NOW).len(), 2);
    }

    #[test]
    fn logs_yield_install_then_update_not_two_installs() {
        let logs = vec![
            log(
                "1",
                "App",
                "HKLM\\Uninstall\\App",
                "1.0",
                "2026-08-01 10:00:00",
            ),
            log(
                "2",
                "App",
                "HKLM\\Uninstall\\App",
                "1.1",
                "2026-09-01 10:00:00",
            ),
            // Re-monitored at the same version: no event.
            log(
                "3",
                "App",
                "HKLM\\Uninstall\\App",
                "1.1",
                "2026-09-02 10:00:00",
            ),
        ];
        let now_state = vec![prog("HKLM\\Uninstall\\App", "App", "1.1")];
        let ev = events_from_logs(&logs, &HashSet::new(), &now_state, &[], NOW);
        assert_eq!(ev.len(), 2, "{ev:?}");
        assert_eq!(ev[0].kind, HistoryEventKind::ProgramInstalled);
        assert_eq!(ev[0].date, "2026-08-01");
        assert_eq!(ev[0].to_version, "1.0");
        assert_eq!(ev[1].kind, HistoryEventKind::ProgramUpdated);
        assert_eq!(ev[1].from_version, "1.0");
        assert_eq!(ev[1].to_version, "1.1");
        assert_eq!(ev[1].date, "2026-09-01");
        // A log already seen produces nothing again.
        let seen: HashSet<String> = ["1", "2", "3"].iter().map(|s| s.to_string()).collect();
        assert!(events_from_logs(&logs, &seen, &now_state, &[], NOW).is_empty());
    }

    #[test]
    fn log_for_a_removed_program_yields_one_removal() {
        let logs = vec![log(
            "1",
            "Old App",
            "HKLM\\Uninstall\\Old",
            "5.0",
            "2026-07-01 09:30:00",
        )];
        // Not installed, not in the baseline: the log path reports it.
        let ev = events_from_logs(&logs, &HashSet::new(), &[], &[], NOW);
        let rems: Vec<_> = ev
            .iter()
            .filter(|e| e.kind == HistoryEventKind::ProgramRemoved)
            .collect();
        assert_eq!(rems.len(), 1);
        assert_eq!(rems[0].subject, "Old App");
        assert_eq!(rems[0].from_version, "5.0");
        // In the baseline: the program diff owns the removal, not the logs.
        let baseline = vec![prog("HKLM\\Uninstall\\Old", "Old App", "5.0")];
        let ev2 = events_from_logs(&logs, &HashSet::new(), &[], &baseline, NOW);
        assert!(ev2
            .iter()
            .all(|e| e.kind != HistoryEventKind::ProgramRemoved));
    }

    #[test]
    fn first_capture_sets_baseline_without_inventing_events() {
        let mut store = HistoryStore::default();
        let programs = vec![installed(&prog("HKLM\\Uninstall\\A", "App A", "1.0"))];
        let logs = vec![log(
            "1",
            "App A",
            "HKLM\\Uninstall\\A",
            "1.0",
            "2026-01-01 00:00:00",
        )];
        let fresh = store.capture(&logs, &programs, &[], NOW);
        assert!(fresh.is_empty());
        assert_eq!(store.started, NOW);
        assert_eq!(store.programs.len(), 1);
        assert_eq!(store.seen_logs, vec!["1".to_string()]);
        // Second capture, nothing changed: still no events.
        assert!(store.capture(&logs, &programs, &[], NOW + 60).is_empty());
        // Third capture, an update: exactly one event, from the diff (the
        // log is already seen).
        let updated = vec![installed(&prog("HKLM\\Uninstall\\A", "App A", "1.1"))];
        let fresh = store.capture(&logs, &updated, &[], NOW + 120);
        assert_eq!(fresh.len(), 1);
        assert_eq!(fresh[0].kind, HistoryEventKind::ProgramUpdated);
        assert_eq!(store.events.len(), 1);
    }

    #[test]
    fn store_round_trip_and_version_tolerance() {
        let tmp = crate::util::tempdir().unwrap();
        let mut store = HistoryStore::default();
        store.started = NOW;
        store.events.push(HistoryEvent {
            ts: NOW,
            date: "2026-10-16".into(),
            kind: HistoryEventKind::AutostartAdded,
            subject: "OneDrive".into(),
            target: "\"C:\\Program Files\\OneDrive\\OneDrive.exe\" /background".into(),
            ..Default::default()
        });
        store.save(tmp.path()).unwrap();
        let back = HistoryStore::load(tmp.path());
        assert_eq!(back, store);

        // Unknown fields and absent sections are tolerated.
        std::fs::write(
            HistoryStore::path(tmp.path()),
            r#"{"version":1,"started":5,"events":[],"future_field":{"x":1}}"#,
        )
        .unwrap();
        let back = HistoryStore::load(tmp.path());
        assert_eq!(back.started, 5);
        assert!(back.events.is_empty());

        // A file from a newer version loads, but is never overwritten.
        std::fs::write(
            HistoryStore::path(tmp.path()),
            r#"{"version":99,"started":7,"events":[],"programs":[],"autostart":[],"seen_logs":[]}"#,
        )
        .unwrap();
        let newer = HistoryStore::load(tmp.path());
        assert_eq!(newer.version, 99);
        assert!(newer.save(tmp.path()).is_err());
        let still = std::fs::read_to_string(HistoryStore::path(tmp.path())).unwrap();
        assert!(still.contains("\"version\":99"));

        // Garbage is an empty history, not a crash.
        std::fs::write(HistoryStore::path(tmp.path()), "not json").unwrap();
        assert_eq!(HistoryStore::load(tmp.path()), HistoryStore::default());
    }

    #[test]
    fn report_empty_state_is_honest() {
        let store = HistoryStore::default();
        let from = Date::from_ymd(2026, 7, 1).unwrap();
        let to = Date::from_ymd(2026, 10, 1).unwrap();
        let r = report(&store, from, to);
        assert!(r.is_empty());
        assert_eq!(r.total_events, 0);
        assert_eq!(r.history_started, None);

        // Events outside the range do not leak in; the range is inclusive.
        let mut store = HistoryStore::default();
        store.started = NOW;
        for (date, ts) in [("2026-06-30", 1), ("2026-07-01", 2), ("2026-10-02", 3)] {
            store.events.push(HistoryEvent {
                ts,
                date: date.into(),
                kind: HistoryEventKind::ProgramInstalled,
                subject: "X".into(),
                ..Default::default()
            });
        }
        let r = report(&store, from, to);
        assert_eq!(r.events.len(), 1);
        assert_eq!(r.events[0].date, "2026-07-01");
        assert_eq!(r.total_events, 3);
        assert!(r.history_started.is_some());
    }

    #[test]
    fn created_parsing_validates_shape() {
        let (ts, date) = parse_created("2026-08-01 10:00:00").unwrap();
        assert_eq!(date, "2026-08-01");
        assert_eq!(ts % 86_400, 36_000);
        assert!(parse_created("2026-08-01").is_some()); // date only is fine
        assert!(parse_created("2026-13-01 10:00:00").is_none());
        assert!(parse_created("2026-08-01 25:00:00").is_none());
        assert!(parse_created("garbage").is_none());
    }
}
