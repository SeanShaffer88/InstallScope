//! Live activity: file changes stream in as they happen (Windows change
//! notifications), and registry areas report when something under them
//! changes. The exact registry changes are in the final comparison.

use std::collections::VecDeque;
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

#[derive(Default)]
pub struct Counts {
    pub added: u64,
    pub changed: u64,
    pub removed: u64,
    pub registry: u64,
}

struct Shared {
    events: VecDeque<LiveEvent>,
    counts: Counts,
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
    match kind {
        LiveKind::Added => s.counts.added += 1,
        LiveKind::Changed | LiveKind::Renamed => s.counts.changed += 1,
        LiveKind::Removed => s.counts.removed += 1,
        LiveKind::Registry => s.counts.registry += 1,
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

impl LiveWatcher {
    /// Watch folders (with whether to include subfolders) and registry
    /// roots. Paths matching `exclusions` are ignored.
    pub fn start(
        folders: &[(PathBuf, bool)],
        registry: &[String],
        exclusions: crate::util::Exclusions,
    ) -> Self {
        let shared = Arc::new(Mutex::new(Shared {
            events: VecDeque::new(),
            counts: Counts::default(),
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
                    let _ = crate::platform::windows::watch_folder(
                        &dir,
                        recursive,
                        &st,
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
                });
            }
            for root in registry {
                let (sh, st, root) = (shared.clone(), stop.clone(), root.clone());
                std::thread::spawn(move || {
                    let mut last = std::time::Instant::now() - std::time::Duration::from_secs(5);
                    let _ = crate::platform::windows::watch_registry(&root, &st, || {
                        // At most one notice per second per area.
                        if last.elapsed() >= std::time::Duration::from_secs(1) {
                            last = std::time::Instant::now();
                            push(&sh, LiveKind::Registry, root.clone());
                        }
                    });
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
        (
            s.counts.added,
            s.counts.changed,
            s.counts.removed,
            s.counts.registry,
        )
    }
}

impl Drop for LiveWatcher {
    fn drop(&mut self) {
        // Wakes every watcher thread at once.
        self.stop.stop();
    }
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;

    #[test]
    fn streams_file_changes() {
        let d = std::env::temp_dir().join(format!("setupscope-live-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join("sub")).unwrap();
        let w = LiveWatcher::start(
            &[(d.clone(), true)],
            &[],
            crate::util::Exclusions::new(&["*.tmp".to_string()]),
        );
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
        drop(w);
        let _ = std::fs::remove_dir_all(&d);
    }
}
