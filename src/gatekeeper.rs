//! Autostart Gatekeeper: an opt-in review queue for new autostart entries.
//!
//! When turned on (Settings, or `installscope gatekeeper enable`), each scan
//! compares the programs that start with Windows — Run and RunOnce values and
//! Startup-folder shortcuts, the set [`crate::startup::list_startup`] reports —
//! against a stored baseline. Entries that are new or changed since the
//! baseline land here as *pending* items, each attributed to the install log
//! that explains it when the evidence supports that, and judged by the
//! Installer Decrapifier rules. The user then decides: **allow** (keep the
//! entry; the baseline advances to include it) or **remove** (delete it,
//! backed up first like every other removal in InstallScope).
//!
//! The Gatekeeper never blocks an installer while it runs and never removes
//! anything on its own: with no decision from the user, an item stays pending
//! and its entry stays exactly as it is. Detection runs only when something
//! asks for a scan (the app, or `installscope gatekeeper scan`); there is no
//! background watcher in this slice.

use crate::backup::BackupWriter;
use crate::decrap;
use crate::model::{fold, ChangeSet, InstallLog};
use crate::platform::Registry;
use crate::startup::{self, StartupEntry, StartupSource};
use crate::store::Store;
use anyhow::{Context as _, Result};
use std::collections::HashMap;
use std::path::PathBuf;

/// Version of the on-disk queue format.
pub const QUEUE_VERSION: u32 = 1;

/// `WatchedEntry.kind` for a value in a Run/RunOnce key.
pub const KIND_RUN: &str = "run";
/// `WatchedEntry.kind` for a file in a Startup folder.
pub const KIND_FOLDER: &str = "startup-folder";

/// One autostart entry in the canonical form the baseline is stored in.
/// Identity is `kind` + `location` + `name` (all case-folded); any other
/// field changing makes the entry *changed*.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct WatchedEntry {
    /// [`KIND_RUN`] or [`KIND_FOLDER`].
    pub kind: String,
    pub name: String,
    /// The command line (registry) or the shortcut path (folder).
    pub command: String,
    /// The registry key, or the Startup folder.
    pub location: String,
    pub enabled: bool,
    pub all_users: bool,
}
crate::json_struct!(WatchedEntry, default, { kind, name, command, location, enabled, all_users });

impl From<&StartupEntry> for WatchedEntry {
    fn from(e: &StartupEntry) -> Self {
        let (kind, location) = match &e.source {
            StartupSource::Registry { key } => (KIND_RUN, key.clone()),
            StartupSource::Folder { dir } => (KIND_FOLDER, dir.clone()),
        };
        WatchedEntry {
            kind: kind.to_string(),
            name: e.name.clone(),
            command: e.command.clone(),
            location,
            enabled: e.enabled,
            all_users: e.all_users,
        }
    }
}

/// The identity key two snapshots of the same entry share.
pub fn entry_key(e: &WatchedEntry) -> String {
    format!("{}|{}|{}", e.kind, fold(&e.location), fold(&e.name))
}

/// Where a queue item stands.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ItemState {
    /// Waiting for the user to decide.
    #[default]
    Pending,
    /// The user chose to keep the entry.
    Allowed,
    /// The user chose to remove the entry, and it was removed.
    Removed,
}
crate::json_enum!(ItemState {
    Pending,
    Allowed,
    Removed
});

impl ItemState {
    pub fn label(self) -> &'static str {
        match self {
            ItemState::Pending => "waiting for your decision",
            ItemState::Allowed => "allowed",
            ItemState::Removed => "removed",
        }
    }
}

/// Who added an entry, as far as the evidence honestly goes.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum Attribution {
    /// An install log explains the entry; `evidence` says how strongly.
    Log {
        log_id: String,
        /// The program the log belongs to.
        program: String,
        /// Plain-language statement of the evidence level.
        evidence: String,
    },
    /// No install log explains it.
    #[default]
    Unknown,
}
crate::json_enum!(Attribution { Log { log_id, program, evidence }, Unknown });

impl Attribution {
    pub fn label(&self) -> String {
        match self {
            Attribution::Log { program, .. } => format!("added by {program}"),
            Attribution::Unknown => "unknown origin".to_string(),
        }
    }
}

/// One entry waiting for (or past) a decision.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct QueueItem {
    pub id: String,
    pub entry: WatchedEntry,
    pub state: ItemState,
    /// First scan that saw the entry, local time (`2026-10-16 05:07:09`).
    pub first_seen: String,
    /// Most recent scan that still saw it.
    pub last_seen: String,
    pub attribution: Attribution,
    /// The Decrapifier verdict for this entry, when the rules produced one.
    pub verdict: Option<decrap::Verdict>,
    /// Why the verdict came out that way (plain language).
    pub verdict_reasons: Vec<String>,
    /// How the entry differs from the baseline, for changed entries.
    pub note: String,
}
crate::json_struct!(QueueItem, default, { id, entry, state, first_seen, last_seen, attribution, verdict, verdict_reasons, note });

/// The Gatekeeper's whole persistent state: the baseline of accepted
/// autostart entries plus the review queue.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Queue {
    pub version: u32,
    pub baseline: Vec<WatchedEntry>,
    pub items: Vec<QueueItem>,
}
crate::json_struct!(Queue, default, { version, baseline, items });

impl Queue {
    pub fn empty() -> Queue {
        Queue {
            version: QUEUE_VERSION,
            baseline: Vec::new(),
            items: Vec::new(),
        }
    }

    pub fn load(store: &Store) -> Result<Queue> {
        let path = store.gatekeeper_path();
        let Some(text) = std::fs::read_to_string(&path).ok() else {
            return Ok(Queue::empty());
        };
        let q: Queue = crate::json::compat::from_str(&text)
            .with_context(|| format!("reading {}", path.display()))?;
        if q.version != QUEUE_VERSION {
            anyhow::bail!(
                "{} was written by a different InstallScope version (format {}); \
                 it was left untouched",
                path.display(),
                q.version
            );
        }
        Ok(q)
    }

    /// Atomic save: write a temp file, then rename over the old one, the
    /// same way install logs are saved.
    pub fn save(&self, store: &Store) -> Result<PathBuf> {
        let path = store.gatekeeper_path();
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, crate::json::compat::to_string_pretty(self)?)?;
        std::fs::rename(&tmp, &path)?;
        Ok(path)
    }

    pub fn pending(&self) -> Vec<&QueueItem> {
        self.items
            .iter()
            .filter(|i| i.state == ItemState::Pending)
            .collect()
    }

    /// Find an item by id, accepting an unambiguous prefix of the id.
    pub fn find(&self, id: &str) -> Result<&QueueItem> {
        self.items
            .iter()
            .find(|i| i.id == id)
            .map_or_else(|| unique_prefix(&self.items, id), Ok)
    }

    fn find_mut(&mut self, id: &str) -> Result<&mut QueueItem> {
        if self.items.iter().any(|i| i.id == id) {
            return Ok(self.items.iter_mut().find(|i| i.id == id).unwrap());
        }
        let idx = unique_prefix(&self.items, id).map(|i| i.id.clone())?;
        Ok(self.items.iter_mut().find(|i| i.id == idx).unwrap())
    }

    /// Record the user's decision to keep an entry. The entry itself is not
    /// touched; the baseline takes it in right away so scans stop asking.
    pub fn allow(&mut self, id: &str) -> Result<QueueItem> {
        let (done, entry) = {
            let item = self.find_mut(id)?;
            if item.state == ItemState::Removed {
                anyhow::bail!("entry {} was already removed", item.id);
            }
            item.state = ItemState::Allowed;
            (item.clone(), item.entry.clone())
        };
        let key = entry_key(&entry);
        match self.baseline.iter_mut().find(|e| entry_key(e) == key) {
            Some(slot) => *slot = entry,
            None => self.baseline.push(entry),
        }
        Ok(done)
    }

    /// Record that an entry is gone (after a successful removal).
    pub fn mark_removed(&mut self, id: &str) -> Result<QueueItem> {
        let item = self.find_mut(id)?;
        item.state = ItemState::Removed;
        Ok(item.clone())
    }
}

fn unique_prefix<'a>(items: &'a [QueueItem], id: &str) -> Result<&'a QueueItem> {
    let mut hits = items.iter().filter(|i| i.id.starts_with(id));
    match (hits.next(), hits.next()) {
        (Some(one), None) => Ok(one),
        (Some(_), Some(_)) => {
            anyhow::bail!("\"{id}\" matches more than one queue item; use the full id")
        }
        _ => anyhow::bail!("no queue item with id \"{id}\""),
    }
}

/// What one scan found.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ScanSummary {
    /// Ids of items created for entries the baseline didn't know.
    pub added: Vec<String>,
    /// Ids of items created for entries that changed since the baseline.
    pub changed: Vec<String>,
}

/// Establish the first known-good snapshot without creating review items.
/// Returns `true` only when the queue was empty and its baseline was seeded.
/// This is used immediately before an explicitly monitored installation so
/// that its post-install scan can identify what changed during that install.
pub fn seed_baseline_if_empty(queue: &mut Queue, current: &[WatchedEntry]) -> bool {
    if queue.baseline.is_empty() && queue.items.is_empty() {
        queue.baseline = current.to_vec();
        true
    } else {
        false
    }
}

fn now_stamp() -> String {
    crate::time::DateTime::now_local().ymd_hms()
}

/// Diff `current` against the queue's baseline: entries that appeared or
/// changed become pending queue items; pending entries are refreshed;
/// decided and never-flagged entries form the new baseline. Pure over its
/// inputs so it can be tested without a registry.
pub fn run_scan(queue: &mut Queue, current: &[WatchedEntry], logs: &[InstallLog]) -> ScanSummary {
    let now = now_stamp();
    let mut summary = ScanSummary::default();
    let base: HashMap<String, &WatchedEntry> =
        queue.baseline.iter().map(|e| (entry_key(e), e)).collect();
    let mut pending: HashMap<String, usize> = queue
        .items
        .iter()
        .enumerate()
        .filter(|(_, i)| i.state == ItemState::Pending)
        .map(|(idx, i)| (entry_key(&i.entry), idx))
        .collect();

    for e in current {
        let key = entry_key(e);
        if let Some(&idx) = pending.get(&key) {
            // Still waiting for a decision: keep the item fresh, and if the
            // entry changed again, show what it looks like now.
            let item = &mut queue.items[idx];
            item.last_seen = now.clone();
            if item.entry.command != e.command || item.entry.enabled != e.enabled {
                item.entry = e.clone();
            }
            continue;
        }
        match base.get(&key) {
            None => {
                let item = judge(e.clone(), logs, &now, String::new());
                summary.added.push(item.id.clone());
                queue.items.push(item);
                pending.insert(key, queue.items.len() - 1);
            }
            Some(old) if **old != *e => {
                let mut note = String::new();
                if old.command != e.command {
                    note = format!("command changed from \"{}\"", old.command);
                }
                if old.enabled != e.enabled {
                    if !note.is_empty() {
                        note.push_str("; ");
                    }
                    note.push_str(if e.enabled {
                        "switched on"
                    } else {
                        "switched off"
                    });
                }
                let item = judge(e.clone(), logs, &now, note);
                summary.changed.push(item.id.clone());
                queue.items.push(item);
                pending.insert(key, queue.items.len() - 1);
            }
            _ => {}
        }
    }

    // The baseline advances only over entries with no pending item: decided
    // and never-flagged entries become the new known-good set; undecided
    // entries stay out of it so they remain in the queue.
    queue.baseline = current
        .iter()
        .filter(|e| !pending.contains_key(&entry_key(e)))
        .cloned()
        .collect();
    summary
}

/// One scan, if the Gatekeeper is on. When it is off this is a strict
/// no-op: nothing is read into the queue and the baseline does not move.
pub fn scan_if_enabled(
    enabled: bool,
    queue: &mut Queue,
    current: &[WatchedEntry],
    logs: &[InstallLog],
) -> Option<ScanSummary> {
    enabled.then(|| run_scan(queue, current, logs))
}

/// Build a queue item for an entry: attribution first, then a Decrapifier
/// verdict judged against the attributed program when there is one.
fn judge(entry: WatchedEntry, logs: &[InstallLog], now: &str, note: String) -> QueueItem {
    let attribution = attribute(&entry, logs);
    let ctx = match &attribution {
        Attribution::Log { log_id, .. } => logs
            .iter()
            .find(|l| &l.id == log_id)
            .map(decrap::context_for)
            .unwrap_or_default(),
        Attribution::Unknown => decrap::Context::default(),
    };
    let mut comp = decrap::Component {
        kind: if entry.kind == KIND_FOLDER {
            decrap::Kind::StartupShortcut
        } else {
            decrap::Kind::Autorun
        },
        name: entry.name.clone(),
        command: entry.command.clone(),
        location: entry.location.clone(),
        verdict: decrap::Verdict::Unknown,
        category: decrap::Category::Other,
        reasons: Vec::new(),
        decided_by: decrap::DecidedBy::Rules,
    };
    decrap::classify(&mut comp, &ctx);
    QueueItem {
        id: crate::util::new_id(),
        entry,
        state: ItemState::Pending,
        first_seen: now.to_string(),
        last_seen: now.to_string(),
        attribution,
        verdict: Some(comp.verdict),
        verdict_reasons: comp.reasons,
        note,
    }
}

/// Attribute an entry to an install log, strongest honest evidence first:
///
/// 1. The log's change set records the entry itself (the Run value, or the
///    Startup-folder file) — the entry is *in* that log.
/// 2. The entry's name or command names the program, or its target file
///    lives in a folder that program's log recorded creating.
/// 3. Otherwise the origin is unknown — never guessed.
pub fn attribute(entry: &WatchedEntry, logs: &[InstallLog]) -> Attribution {
    for log in logs {
        if covers(&log.changes, entry) {
            return Attribution::Log {
                log_id: log.id.clone(),
                program: log.name.clone(),
                evidence: format!("the install log for \"{}\" recorded this entry", log.name),
            };
        }
    }
    let (prog, _) = crate::util::split_command_line(&crate::platform::expand_env(&entry.command));
    let fprog = fold(&prog);
    let fname = fold(&entry.name);
    let fcmd = fold(&crate::platform::expand_env(&entry.command));
    for log in logs {
        let lname = fold(&log.name);
        let names_it = !lname.is_empty() && (fname.contains(&lname) || fcmd.contains(&lname));
        let in_its_folders = !fprog.is_empty()
            && decrap::context_for(log)
                .install_dirs
                .iter()
                .map(|d| fold(d.trim_end_matches('\\')))
                .any(|d| !d.is_empty() && fprog.starts_with(&format!("{d}\\")));
        if names_it || in_its_folders {
            return Attribution::Log {
                log_id: log.id.clone(),
                program: log.name.clone(),
                evidence: if in_its_folders {
                    format!(
                        "points at a file in a folder the install log for \"{}\" recorded",
                        log.name
                    )
                } else {
                    format!("the entry names \"{}\"", log.name)
                },
            };
        }
    }
    Attribution::Unknown
}

/// Whether a change set itself added this entry.
fn covers(cs: &ChangeSet, entry: &WatchedEntry) -> bool {
    if entry.kind == KIND_FOLDER {
        let path = fold(&entry.command);
        return cs.files_added.iter().any(|f| fold(&f.path) == path);
    }
    let key = fold(&entry.location);
    let name = fold(&entry.name);
    cs.values_added
        .iter()
        .any(|v| fold(&v.key) == key && fold(&v.value.name) == name)
        || cs
            .values_modified
            .iter()
            .any(|v| fold(&v.key) == key && fold(&v.after.name) == name)
        || cs
            .keys_added
            .iter()
            .any(|k| fold(&k.path) == key && k.values.values().any(|v| fold(&v.name) == name))
}

/// Remove a queued entry from the system, backed up first — the same
/// [`startup::delete_entry`] path the Startup page uses. Refuses entries
/// Windows, security software or hardware need, and entries that are no
/// longer there; the queue item is marked removed only on success.
pub fn remove_entry(
    queue: &mut Queue,
    id: &str,
    reg: &dyn Registry,
    backups_dir: &std::path::Path,
) -> Result<(QueueItem, String)> {
    let item = queue.find(id)?.clone();
    if item.state == ItemState::Removed {
        anyhow::bail!("\"{}\" was already removed", item.entry.name);
    }
    let current = startup::list_startup(reg);
    let key = entry_key(&item.entry);
    let Some(live) = current
        .iter()
        .find(|e| entry_key(&WatchedEntry::from(*e)) == key)
    else {
        anyhow::bail!(
            "\"{}\" is no longer on this PC, so there is nothing to remove",
            item.entry.name
        );
    };
    if startup::is_essential(live) {
        anyhow::bail!(
            "\"{}\" is needed by Windows, security software or hardware and can't be removed here",
            live.name
        );
    }
    let mut w = BackupWriter::new(
        backups_dir,
        &format!("Autostart Gatekeeper: remove {}", live.name),
    )?;
    startup::delete_entry(reg, live, Some(&mut w))?;
    let backup = w.finish()?;
    let item = queue.mark_removed(&item.id)?;
    Ok((item, backup.id))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{FileEntry, RegValue, ValueRef};

    const RUN_KEY: &str = "HKCU\\Software\\Microsoft\\Windows\\CurrentVersion\\Run";

    fn run_entry(name: &str, command: &str) -> WatchedEntry {
        WatchedEntry {
            kind: KIND_RUN.to_string(),
            name: name.to_string(),
            command: command.to_string(),
            location: RUN_KEY.to_string(),
            enabled: true,
            all_users: false,
        }
    }

    fn folder_entry(name: &str, dir: &str) -> WatchedEntry {
        WatchedEntry {
            kind: KIND_FOLDER.to_string(),
            name: name.to_string(),
            command: format!("{dir}\\{name}"),
            location: dir.to_string(),
            enabled: true,
            all_users: false,
        }
    }

    fn log_covering_run_value(name: &str, command: &str) -> InstallLog {
        let mut log = InstallLog {
            id: "log-1".into(),
            name: "Contoso App".into(),
            ..Default::default()
        };
        log.changes.values_added.push(ValueRef {
            key: RUN_KEY.to_string(),
            value: RegValue::string(name, command),
        });
        log
    }

    #[test]
    fn diff_reports_new_changed_and_unchanged() {
        let mut q = Queue::empty();
        q.baseline = vec![
            run_entry("Keep", "C:\\Apps\\keep.exe"),
            run_entry("Morph", "C:\\Apps\\old.exe"),
        ];
        let current = vec![
            run_entry("Keep", "C:\\Apps\\keep.exe"),
            run_entry("Morph", "C:\\Apps\\new.exe"),
            run_entry("Fresh", "C:\\Apps\\fresh.exe"),
        ];
        let s = run_scan(&mut q, &current, &[]);
        assert_eq!(s.added.len(), 1);
        assert_eq!(s.changed.len(), 1);
        let pending = q.pending();
        assert_eq!(pending.len(), 2);
        let changed = pending
            .iter()
            .find(|i| i.entry.name == "Morph")
            .expect("changed entry queued");
        assert!(changed.note.contains("command changed"), "{}", changed.note);
        // New and changed entries stay out of the baseline while undecided;
        // the unchanged one is in it.
        assert_eq!(q.baseline.len(), 1);
        assert_eq!(q.baseline[0].name, "Keep");
        // A second scan of the same world queues nothing new.
        let s2 = run_scan(&mut q, &current, &[]);
        assert!(s2.added.is_empty() && s2.changed.is_empty());
        assert_eq!(q.pending().len(), 2);
    }

    #[test]
    fn attribution_finds_the_covering_log() {
        let log = log_covering_run_value("ContosoTray", "\"C:\\Program Files\\Contoso\\tray.exe\"");
        let e = run_entry("ContosoTray", "\"C:\\Program Files\\Contoso\\tray.exe\"");
        match attribute(&e, std::slice::from_ref(&log)) {
            Attribution::Log {
                log_id, evidence, ..
            } => {
                assert_eq!(log_id, "log-1");
                assert!(evidence.contains("recorded this entry"), "{evidence}");
            }
            other => panic!("expected log attribution, got {other:?}"),
        }
        // A startup-folder shortcut the log's files_added recorded.
        let dir =
            "C:\\Users\\a\\AppData\\Roaming\\Microsoft\\Windows\\Start Menu\\Programs\\Startup";
        let mut log2 = InstallLog {
            id: "log-2".into(),
            name: "Folder App".into(),
            ..Default::default()
        };
        log2.changes.files_added.push(FileEntry {
            path: format!("{dir}\\Folder App.lnk"),
            is_dir: false,
            size: 10,
            mtime: 0,
        });
        let fe = folder_entry("Folder App.lnk", dir);
        assert!(matches!(
            attribute(&fe, &[log2]),
            Attribution::Log { log_id, .. } if log_id == "log-2"
        ));
        // The same evidence flows into a scan-produced queue item.
        let mut q = Queue::empty();
        run_scan(&mut q, &[e.clone()], &[log]);
        let item = &q.pending()[0];
        assert!(matches!(item.attribution, Attribution::Log { .. }));
        assert!(item.verdict.is_some());
    }

    #[test]
    fn attribution_is_unknown_without_evidence() {
        let e = run_entry("Mystery", "C:\\Where\\mystery.exe");
        assert_eq!(attribute(&e, &[]), Attribution::Unknown);
        // A log that neither recorded the entry nor matches its name/folder.
        let other = InstallLog {
            id: "log-x".into(),
            name: "Unrelated".into(),
            ..Default::default()
        };
        assert_eq!(attribute(&e, &[other]), Attribution::Unknown);
    }

    #[test]
    fn decided_items_advance_the_baseline() {
        let mut q = Queue::empty();
        let a = run_entry("Alpha", "C:\\A\\a.exe");
        let b = run_entry("Beta", "C:\\B\\b.exe");
        run_scan(&mut q, &[a.clone(), b.clone()], &[]);
        assert_eq!(q.pending().len(), 2);
        assert!(
            q.baseline.is_empty(),
            "nothing decided, baseline stays empty"
        );

        let alpha_id = q
            .items
            .iter()
            .find(|i| i.entry.name == "Alpha")
            .unwrap()
            .id
            .clone();
        q.allow(&alpha_id).unwrap();
        // Next scan: Alpha is decided, so it joins the baseline and is not
        // re-queued; Beta is still pending and stays out.
        let s = run_scan(&mut q, &[a.clone(), b.clone()], &[]);
        assert!(s.added.is_empty());
        assert_eq!(
            q.items
                .iter()
                .filter(|i| i.state == ItemState::Pending)
                .count(),
            1
        );
        assert_eq!(q.baseline.len(), 1);
        assert_eq!(q.baseline[0].name, "Alpha");

        // After removal the entry is gone from the system: later scans
        // neither duplicate the item nor lose the record.
        let beta_id = q
            .items
            .iter()
            .find(|i| i.entry.name == "Beta")
            .unwrap()
            .id
            .clone();
        q.mark_removed(&beta_id).unwrap();
        let s = run_scan(&mut q, std::slice::from_ref(&a), &[]);
        assert!(s.added.is_empty());
        assert_eq!(
            q.items.iter().filter(|i| i.entry.name == "Beta").count(),
            1,
            "a decided item is never duplicated"
        );
        assert_eq!(
            q.items
                .iter()
                .find(|i| i.entry.name == "Beta")
                .unwrap()
                .state,
            ItemState::Removed
        );
        // If the entry is created again later (a program re-adding itself),
        // that is a new sighting and earns a fresh queue item.
        let s = run_scan(&mut q, &[a, b], &[]);
        assert_eq!(s.added.len(), 1);
    }

    #[test]
    fn disabled_gatekeeper_is_a_no_op() {
        let mut q = Queue::empty();
        let current = vec![run_entry("Fresh", "C:\\Apps\\fresh.exe")];
        let out = scan_if_enabled(false, &mut q, &current, &[]);
        assert!(out.is_none());
        assert!(q.items.is_empty());
        assert!(q.baseline.is_empty());
        // And the same inputs do scan when it is on.
        let out = scan_if_enabled(true, &mut q, &current, &[]);
        assert_eq!(out.unwrap().added.len(), 1);
    }

    #[test]
    fn install_baseline_seed_is_non_reviewing_and_only_happens_once() {
        let before = run_entry("Existing", "C:\\Apps\\existing.exe");
        let mut q = Queue::empty();
        assert!(seed_baseline_if_empty(
            &mut q,
            std::slice::from_ref(&before)
        ));
        assert_eq!(q.baseline, vec![before.clone()]);
        assert!(q.items.is_empty());

        let added = run_entry("Added", "C:\\Apps\\added.exe");
        assert!(!seed_baseline_if_empty(
            &mut q,
            std::slice::from_ref(&added)
        ));
        let summary = run_scan(&mut q, &[before, added], &[]);
        assert_eq!(summary.added.len(), 1);
        assert_eq!(q.pending().len(), 1);
    }

    #[test]
    fn queue_round_trips_and_refuses_other_versions() {
        let tmp = crate::util::tempdir().unwrap();
        let st = Store::at(tmp.path()).unwrap();
        let mut q = Queue::empty();
        run_scan(&mut q, &[run_entry("Tray", "C:\\T\\tray.exe")], &[]);
        q.save(&st).unwrap();
        let back = Queue::load(&st).unwrap();
        assert_eq!(back, q);
        assert_eq!(back.version, QUEUE_VERSION);

        std::fs::write(
            st.gatekeeper_path(),
            "{\"version\": 99, \"baseline\": [], \"items\": []}",
        )
        .unwrap();
        assert!(Queue::load(&st).is_err());
    }

    #[test]
    fn kinds_and_keys_distinguish_sources() {
        let dir = "D:\\Startup";
        let r = run_entry("Same Name", "x");
        let f = folder_entry("Same Name", dir);
        assert_ne!(entry_key(&r), entry_key(&f));
        // Identity is case-insensitive, like the registry and file system.
        let r2 = run_entry("same name", "x");
        assert_eq!(entry_key(&r), entry_key(&r2));
    }
}
