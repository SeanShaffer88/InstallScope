//! Live activity: file changes stream in as they happen (Windows change
//! notifications), and registry areas report when something under them
//! changes. The exact registry changes are in the final comparison.

use std::collections::{HashSet, VecDeque};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LiveKind {
    Added,
    Changed,
    Removed,
    Renamed,
    Registry,
}

#[derive(Clone, Debug, PartialEq)]
pub struct LiveEvent {
    /// Local time, HH:MM:SS.
    pub time: String,
    pub kind: LiveKind,
    pub path: String,
}

/// Events kept in memory (oldest dropped first).
#[cfg_attr(not(windows), allow(dead_code))]
const KEEP: usize = 2000;
/// Bound session-wide distinct-path accounting for long-running installs.
const MAX_TRACKED_DISTINCT_PATHS: usize = 100_000;

struct Shared {
    events: VecDeque<LiveEvent>,
    coverage: crate::model::MonitoringCoverage,
    seen_added: HashSet<String>,
    seen_changed: HashSet<String>,
    seen_removed: HashSet<String>,
    seen_registry: HashSet<String>,
}

pub struct LiveWatcher {
    shared: Arc<Mutex<Shared>>,
    stop: Arc<crate::platform::StopSignal>,
}

#[cfg_attr(not(windows), allow(dead_code))]
fn now() -> String {
    crate::time::DateTime::now_local().hms()
}

#[cfg_attr(not(windows), allow(dead_code))]
fn push(shared: &Mutex<Shared>, kind: LiveKind, path: String) {
    let mut s = shared.lock().unwrap();
    let key = crate::model::fold(&path);
    match kind {
        LiveKind::Added => {
            s.coverage.file_notifications += 1;
            track_path(&mut s, LiveKind::Added, key);
        }
        LiveKind::Changed | LiveKind::Renamed => {
            s.coverage.file_notifications += 1;
            track_path(&mut s, LiveKind::Changed, key);
        }
        LiveKind::Removed => {
            s.coverage.file_notifications += 1;
            track_path(&mut s, LiveKind::Removed, key);
        }
        LiveKind::Registry => {
            s.coverage.registry_notifications += 1;
            track_path(&mut s, LiveKind::Registry, key);
        }
    }
    // Collapse repeated writes to the same file.
    if let Some(last) = s.events.back() {
        if last.path == path && last.kind == kind {
            return;
        }
    }
    s.events.push_back(LiveEvent {
        time: now(),
        kind,
        path,
    });
    while s.events.len() > KEEP {
        s.events.pop_front();
    }
}

fn track_path(shared: &mut Shared, kind: LiveKind, key: String) {
    let exists = match kind {
        LiveKind::Added => shared.seen_added.contains(&key),
        LiveKind::Changed | LiveKind::Renamed => shared.seen_changed.contains(&key),
        LiveKind::Removed => shared.seen_removed.contains(&key),
        LiveKind::Registry => shared.seen_registry.contains(&key),
    };
    if exists {
        return;
    }
    let total = shared.seen_added.len()
        + shared.seen_changed.len()
        + shared.seen_removed.len()
        + shared.seen_registry.len();
    if total >= MAX_TRACKED_DISTINCT_PATHS {
        shared.coverage.distinct_path_counts_limited = true;
        let warning = format!(
            "Distinct live-path counts reached the {MAX_TRACKED_DISTINCT_PATHS} item memory limit; notification totals continue."
        );
        if !shared.coverage.warnings.contains(&warning) {
            shared.coverage.warnings.push(warning);
        }
        return;
    }
    match kind {
        LiveKind::Added => {
            shared.seen_added.insert(key);
        }
        LiveKind::Changed | LiveKind::Renamed => {
            shared.seen_changed.insert(key);
        }
        LiveKind::Removed => {
            shared.seen_removed.insert(key);
        }
        LiveKind::Registry => {
            shared.seen_registry.insert(key);
        }
    }
}

#[cfg_attr(not(windows), allow(dead_code))]
fn watcher_ready(shared: &Mutex<Shared>, root: &str, file: bool) {
    let mut s = shared.lock().unwrap();
    let roots = if file {
        &mut s.coverage.active_file_roots
    } else {
        &mut s.coverage.active_registry_roots
    };
    if !roots.iter().any(|r| r.eq_ignore_ascii_case(root)) {
        roots.push(root.to_string());
    }
}

#[cfg_attr(not(windows), allow(dead_code))]
fn watcher_warning(shared: &Mutex<Shared>, warning: String) {
    let mut s = shared.lock().unwrap();
    if !s.coverage.warnings.contains(&warning) {
        s.coverage.warnings.push(warning);
    }
}

#[cfg_attr(not(windows), allow(dead_code))]
fn watcher_overflow(shared: &Mutex<Shared>, root: &str) {
    let mut s = shared.lock().unwrap();
    s.coverage.file_buffer_overflows += 1;
    let warning = format!(
        "Live file notifications overflowed under {root}; the final snapshot comparison remains authoritative."
    );
    if !s.coverage.warnings.contains(&warning) {
        s.coverage.warnings.push(warning);
    }
}

fn unique_counts(shared: &Shared) -> (u64, u64, u64, u64) {
    (
        shared.seen_added.len() as u64,
        shared.seen_changed.len() as u64,
        shared.seen_removed.len() as u64,
        shared.seen_registry.len() as u64,
    )
}

impl LiveWatcher {
    /// Watch folders (with whether to include subfolders) and registry
    /// roots. Paths matching `exclusions` are ignored.
    pub fn start(
        folders: &[(PathBuf, bool)],
        registry: &[String],
        exclusions: crate::util::Exclusions,
    ) -> Self {
        let mut coverage = crate::model::MonitoringCoverage {
            tracking_started: crate::util::now_string(),
            file_roots: folders
                .iter()
                .map(|(p, _)| p.to_string_lossy().into_owned())
                .collect(),
            registry_roots: registry.to_vec(),
            ..Default::default()
        };
        coverage.file_roots.sort_by_key(|p| p.to_lowercase());
        coverage.registry_roots.sort_by_key(|p| p.to_lowercase());
        let shared = Arc::new(Mutex::new(Shared {
            events: VecDeque::new(),
            coverage,
            seen_added: HashSet::new(),
            seen_changed: HashSet::new(),
            seen_removed: HashSet::new(),
            seen_registry: HashSet::new(),
        }));
        let stop = Arc::new(crate::platform::StopSignal::new());
        #[cfg(windows)]
        {
            use crate::platform::windows::FolderEvent;
            let exclusions = Arc::new(exclusions);
            // One thread per folder, each blocked in the kernel until
            // something changes there or the watcher is dropped.
            for (dir, recursive) in folders {
                let (sh, st, ex) = (shared.clone(), stop.clone(), exclusions.clone());
                let (dir, recursive) = (dir.clone(), *recursive);
                std::thread::spawn(move || {
                    let mut folded = String::new();
                    let mut chars = Vec::new();
                    let root = dir.to_string_lossy().into_owned();
                    let result = crate::platform::windows::watch_folder(
                        &dir,
                        recursive,
                        &st,
                        &mut || watcher_ready(&sh, &root, true),
                        &mut || watcher_overflow(&sh, &root),
                        &mut |ev, path| {
                            crate::util::fold_into(path, &mut folded);
                            if ex.matches_folded(&folded, &mut chars) {
                                return;
                            }
                            let kind = match ev {
                                FolderEvent::Added => LiveKind::Added,
                                FolderEvent::Removed => LiveKind::Removed,
                                FolderEvent::Modified => LiveKind::Changed,
                                FolderEvent::Renamed => LiveKind::Renamed,
                            };
                            push(&sh, kind, path.to_string());
                        },
                    );
                    if let Err(error) = result {
                        watcher_warning(
                            &sh,
                            format!("Could not monitor files under {root}: {error:#}"),
                        );
                    }
                });
            }
            for root in registry {
                let (sh, st, root) = (shared.clone(), stop.clone(), root.clone());
                std::thread::spawn(move || {
                    let mut last = std::time::Instant::now() - std::time::Duration::from_secs(5);
                    let watched_root = root.clone();
                    let result = crate::platform::windows::watch_registry(
                        &root,
                        &st,
                        || {
                            watcher_ready(&sh, &watched_root, false);
                        },
                        || {
                            // At most one notice per second per area.
                            if last.elapsed() >= std::time::Duration::from_secs(1) {
                                last = std::time::Instant::now();
                                push(&sh, LiveKind::Registry, root.clone());
                            }
                        },
                    );
                    if let Err(error) = result {
                        watcher_warning(
                            &sh,
                            format!("Could not monitor registry area {root}: {error:#}"),
                        );
                    }
                });
            }
        }
        // Development builds off Windows have no live view.
        #[cfg(not(windows))]
        let _ = (folders, registry, exclusions);
        LiveWatcher { shared, stop }
    }

    /// The most recent events, newest last.
    pub fn recent(&self, n: usize) -> Vec<LiveEvent> {
        let s = self.shared.lock().unwrap();
        s.events.iter().rev().take(n).rev().cloned().collect()
    }

    pub fn counts(&self) -> (u64, u64, u64, u64) {
        let s = self.shared.lock().unwrap();
        unique_counts(&s)
    }

    /// Total notifications, which can exceed distinct paths because a file
    /// may generate several notifications while it is being written.
    pub fn notifications(&self) -> (u64, u64) {
        let s = self.shared.lock().unwrap();
        (
            s.coverage.file_notifications,
            s.coverage.registry_notifications,
        )
    }

    /// A point-in-time receipt of requested roots, active watchers, gaps and
    /// observed counts for the saved install log.
    pub fn coverage(&self) -> crate::model::MonitoringCoverage {
        let s = self.shared.lock().unwrap();
        let mut result = s.coverage.clone();
        result.tracking_ended = crate::util::now_string();
        (
            result.added_paths,
            result.changed_paths,
            result.removed_paths,
            result.registry_paths,
        ) = unique_counts(&s);
        result
    }
}

impl Drop for LiveWatcher {
    fn drop(&mut self) {
        // Wakes every watcher thread at once.
        self.stop.stop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counters_separate_notifications_from_distinct_paths() {
        let shared = Mutex::new(Shared {
            events: VecDeque::new(),
            coverage: crate::model::MonitoringCoverage::default(),
            seen_added: HashSet::new(),
            seen_changed: HashSet::new(),
            seen_removed: HashSet::new(),
            seen_registry: HashSet::new(),
        });
        push(&shared, LiveKind::Added, r"C:\Apps\Example\app.dll".into());
        push(&shared, LiveKind::Added, r"c:\apps\example\APP.DLL".into());
        push(
            &shared,
            LiveKind::Changed,
            r"C:\Apps\Example\app.dll".into(),
        );
        push(
            &shared,
            LiveKind::Changed,
            r"C:\Apps\Example\app.dll".into(),
        );
        let s = shared.lock().unwrap();
        assert_eq!(unique_counts(&s), (1, 1, 0, 0));
        assert_eq!(s.coverage.file_notifications, 4);
        assert_eq!(s.events.len(), 3);
    }

    #[test]
    fn repeated_overflows_count_each_gap_but_keep_one_warning_per_root() {
        let shared = Mutex::new(Shared {
            events: VecDeque::new(),
            coverage: crate::model::MonitoringCoverage::default(),
            seen_added: HashSet::new(),
            seen_changed: HashSet::new(),
            seen_removed: HashSet::new(),
            seen_registry: HashSet::new(),
        });
        watcher_overflow(&shared, r"C:\Apps");
        watcher_overflow(&shared, r"C:\Apps");
        let s = shared.lock().unwrap();
        assert_eq!(s.coverage.file_buffer_overflows, 2);
        assert_eq!(s.coverage.warnings.len(), 1);
    }

    #[cfg(windows)]
    #[test]
    fn streams_file_changes() {
        let d = std::env::temp_dir().join(format!("installscope-live-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join("sub")).unwrap();
        let w = LiveWatcher::start(
            &[(d.clone(), true)],
            &[],
            crate::util::Exclusions::new(&["*.tmp".to_string()]),
        );
        let mut watching = false;
        for _ in 0..40 {
            if w.coverage().active_file_roots.len() == 1 {
                watching = true;
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(25));
        }
        assert!(watching, "{:?}", w.coverage());
        std::thread::sleep(std::time::Duration::from_millis(200));
        std::fs::write(d.join("sub").join("app.dll"), b"x").unwrap();
        std::fs::write(d.join("ignored.tmp"), b"x").unwrap();
        std::fs::remove_file(d.join("sub").join("app.dll")).unwrap();
        let mut ok = false;
        for _ in 0..40 {
            std::thread::sleep(std::time::Duration::from_millis(50));
            let ev = w.recent(50);
            if ev
                .iter()
                .any(|e| e.kind == LiveKind::Added && e.path.ends_with("app.dll"))
                && ev.iter().any(|e| e.kind == LiveKind::Removed)
            {
                ok = true;
                break;
            }
        }
        assert!(ok, "{:?}", w.recent(50));
        assert!(w.recent(100).iter().all(|e| !e.path.ends_with(".tmp")));
        let (a, _, r, _) = w.counts();
        assert!(a >= 1 && r >= 1);
        let receipt = w.coverage();
        assert_eq!(receipt.file_roots.len(), 1);
        assert_eq!(receipt.active_file_roots.len(), 1);
        assert!(receipt.file_notifications >= a + r);
        drop(w);
        let _ = std::fs::remove_dir_all(&d);
    }
}
