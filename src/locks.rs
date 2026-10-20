//! Files that can't be deleted because something has them open: finding out
//! who (a program, a service, Windows Explorer with a shell extension
//! loaded) and, where it is safe, getting them to let go.
//!
//! Detection uses the Windows Restart Manager, which reports every process
//! with a file open or a DLL loaded, including services hosted in shared
//! processes. Off Windows every query is empty, so the core logic stays
//! testable anywhere.

use crate::revert::Action;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// What kind of process is holding a file.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum HolderKind {
    /// Windows Explorer, usually because a shell extension DLL is loaded.
    Explorer,
    /// A Windows service (`service` names it).
    Service,
    /// A process Windows can't run without; never closed by InstallScope.
    System,
    /// Any other program.
    Program,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Holder {
    pub pid: u32,
    /// Friendly name, as Windows reports it ("Windows Explorer", "Notepad").
    pub name: String,
    /// Short service name when the holder is a service.
    pub service: Option<String>,
    pub kind: HolderKind,
}

/// Every process holding any of these files open (or with them loaded).
/// Empty when none do, when the query fails, and off Windows.
pub fn file_holders(paths: &[&Path]) -> Vec<Holder> {
    if paths.is_empty() {
        return Vec::new();
    }
    #[cfg(windows)]
    {
        crate::platform::windows::file_holders(paths)
    }
    #[cfg(not(windows))]
    {
        Vec::new()
    }
}

/// "held by Windows Explorer (a shell extension it has loaded) and the
/// FooUpdate service", or `None` if nothing is known to hold the files.
pub fn describe(holders: &[Holder]) -> Option<String> {
    let mut sorted: Vec<&Holder> = holders.iter().collect();
    sorted.sort_by(|a, b| {
        a.kind
            .cmp(&b.kind)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
            .then(a.pid.cmp(&b.pid))
    });
    let mut parts: Vec<String> = Vec::new();
    for h in sorted {
        let part = match h.kind {
            HolderKind::Explorer => {
                "Windows Explorer (a shell extension it has loaded)".to_string()
            }
            HolderKind::Service => match &h.service {
                Some(s) if !s.is_empty() => format!("the {s} service"),
                _ => format!("the {} service", h.name),
            },
            HolderKind::System => format!("{} (a Windows component)", h.name),
            HolderKind::Program => format!("{} (process {})", h.name, h.pid),
        };
        // Explorer and services can be reported once per file; say them once.
        if !parts.contains(&part) {
            parts.push(part);
        }
    }
    const SHOWN: usize = 3;
    let list = match parts.len() {
        0 => return None,
        1 => parts.remove(0),
        n if n <= SHOWN => {
            let last = parts.pop().unwrap();
            format!("{} and {last}", parts.join(", "))
        }
        n => format!("{} and {} more", parts[..SHOWN].join(", "), n - SHOWN),
    };
    Some(format!("held by {list}"))
}

/// File types Explorer and other hosts load as code (shell extensions,
/// ActiveX controls, Control Panel items, DirectShow filters).
pub fn is_module(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| {
            matches!(
                e.to_ascii_lowercase().as_str(),
                "dll" | "ocx" | "ax" | "cpl"
            )
        })
        .unwrap_or(false)
}

/// Program files (`.exe`).
pub fn is_program(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("exe"))
}

/// Most files examined when looking for code in folders being removed;
/// keeps a huge folder from stalling the start of a removal.
pub const MODULE_SCAN_LIMIT: usize = 20_000;

/// Code modules among the files these actions will delete: the files
/// themselves, and modules inside folders removed with their contents.
pub fn module_paths<'a>(actions: impl IntoIterator<Item = &'a Action>) -> Vec<PathBuf> {
    files_matching(actions, is_module)
}

/// Program files among the files these actions will delete.
pub fn program_paths<'a>(actions: impl IntoIterator<Item = &'a Action>) -> Vec<PathBuf> {
    files_matching(actions, is_program)
}

fn files_matching<'a>(
    actions: impl IntoIterator<Item = &'a Action>,
    want: fn(&Path) -> bool,
) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut seen = 0usize;
    for a in actions {
        match a {
            Action::DeleteFile { path } => {
                let p = Path::new(path);
                if want(p) && p.is_file() {
                    out.push(p.to_path_buf());
                }
            }
            Action::DeleteTree { path } | Action::DeleteDir { path } => {
                let p = Path::new(path);
                if crate::util::is_link(p) {
                    continue; // only the link is removed
                }
                for e in crate::fswalk::WalkDir::new(p).into_iter().flatten() {
                    seen += 1;
                    if seen > MODULE_SCAN_LIMIT {
                        return out;
                    }
                    if e.file_type().is_file() && want(e.path()) {
                        out.push(e.into_path());
                    }
                }
            }
            _ => {}
        }
    }
    out
}

/// Running programs closed before a removal.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Closed {
    /// "Atom (process 4120)"
    pub closed: Vec<String>,
    /// Ones that were still running afterwards.
    pub still_running: Vec<String>,
}

/// Close every running process whose own program file is one of `exes`
/// (the program being removed). A program with a window is asked to close
/// first, so it can save its work, and ended if it hasn't after Windows'
/// 30-second grace period; console and windowless ones are ended at once.
/// Processes that only have one of the program's files loaded or open (an
/// add-in inside another application, say) are never closed; nor are
/// services, Explorer or parts of Windows.
pub fn close_programs(exes: &[&Path]) -> Closed {
    if exes.is_empty() {
        return Closed::default();
    }
    #[cfg(windows)]
    {
        crate::platform::windows::close_programs(exes)
    }
    #[cfg(not(windows))]
    {
        Closed::default()
    }
}

/// How stopping a service went.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ServiceStop {
    /// It was running and has stopped; its process (if it had its own) has exited.
    Stopped,
    /// It was already stopped.
    WasNotRunning,
    /// No such service.
    NotFound,
}

/// Stop a service and wait until it has really stopped: Windows reports it
/// stopped and, for a service with its own process, that process has
/// exited (so the files it had open are released). Fails if it is still
/// running after `timeout`.
pub fn stop_service(name: &str, timeout: Duration) -> anyhow::Result<ServiceStop> {
    #[cfg(windows)]
    {
        crate::platform::windows::stop_service(name, timeout)
    }
    #[cfg(not(windows))]
    {
        let _ = timeout;
        anyhow::bail!("services exist only on Windows: {name}")
    }
}

/// Windows Explorer closed for the duration of a removal. Dropping it starts
/// Explorer again.
pub struct ExplorerPause {
    #[cfg(windows)]
    _inner: crate::platform::windows::ExplorerPause,
}

impl Drop for ExplorerPause {
    fn drop(&mut self) {
        // The Windows half restarts Explorer when it is dropped, right after this.
    }
}

/// If Windows Explorer has any of these files loaded, close it (only
/// Explorer, never another program) so they can be deleted. `None` when
/// Explorer doesn't hold them, or it could not be closed.
pub fn pause_explorer(paths: &[&Path]) -> Option<ExplorerPause> {
    if paths.is_empty() {
        return None;
    }
    #[cfg(windows)]
    {
        crate::platform::windows::pause_explorer(paths).map(|p| ExplorerPause { _inner: p })
    }
    #[cfg(not(windows))]
    {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn h(kind: HolderKind, name: &str, pid: u32, service: Option<&str>) -> Holder {
        Holder {
            pid,
            name: name.into(),
            service: service.map(String::from),
            kind,
        }
    }

    #[test]
    fn describes_nothing_when_nothing_holds_the_file() {
        assert_eq!(describe(&[]), None);
    }

    #[test]
    fn describes_each_kind_of_holder() {
        assert_eq!(
            describe(&[h(HolderKind::Program, "Notepad", 42, None)]).unwrap(),
            "held by Notepad (process 42)"
        );
        assert_eq!(
            describe(&[h(HolderKind::Service, "Foo Updater", 7, Some("FooUpd"))]).unwrap(),
            "held by the FooUpd service"
        );
        assert_eq!(
            describe(&[h(HolderKind::Explorer, "Windows Explorer", 9, None)]).unwrap(),
            "held by Windows Explorer (a shell extension it has loaded)"
        );
        assert_eq!(
            describe(&[h(HolderKind::System, "Local Security Authority", 600, None)]).unwrap(),
            "held by Local Security Authority (a Windows component)"
        );
    }

    #[test]
    fn explorer_first_duplicates_merged_long_lists_shortened() {
        let hs = [
            h(HolderKind::Program, "Zed", 5, None),
            h(HolderKind::Explorer, "Windows Explorer", 9, None),
            h(HolderKind::Explorer, "Windows Explorer", 9, None),
            h(HolderKind::Service, "x", 7, Some("FooUpd")),
        ];
        assert_eq!(
            describe(&hs).unwrap(),
            "held by Windows Explorer (a shell extension it has loaded), the FooUpd service and Zed (process 5)"
        );
        let many: Vec<Holder> = (1..=5)
            .map(|i| h(HolderKind::Program, &format!("App{i}"), i, None))
            .collect();
        assert_eq!(
            describe(&many).unwrap(),
            "held by App1 (process 1), App2 (process 2), App3 (process 3) and 2 more"
        );
    }

    #[test]
    fn modules_are_found_in_files_and_folders_being_removed() {
        let tmp = crate::util::tempdir().unwrap();
        let d = tmp.path().join("App");
        std::fs::create_dir_all(d.join("shell")).unwrap();
        for f in [
            "app.exe",
            "readme.txt",
            "shell/ext64.DLL",
            "player.ocx",
            "filter.ax",
        ] {
            std::fs::write(d.join(f), b"x").unwrap();
        }
        let lone = tmp.path().join("Helper.dll");
        std::fs::write(&lone, b"x").unwrap();
        let missing = tmp.path().join("gone.dll");
        let actions = [
            Action::DeleteFile {
                path: lone.to_string_lossy().into(),
            },
            Action::DeleteFile {
                path: missing.to_string_lossy().into(),
            },
            Action::DeleteFile {
                path: d.join("readme.txt").to_string_lossy().into(),
            },
            Action::DeleteTree {
                path: d.to_string_lossy().into(),
            },
            Action::DeleteKey {
                path: "HKCU\\Software\\App".into(),
            },
        ];
        let mut got: Vec<String> = module_paths(&actions)
            .iter()
            .map(|p| {
                p.strip_prefix(tmp.path())
                    .unwrap()
                    .to_string_lossy()
                    .replace('\\', "/")
            })
            .collect();
        got.sort();
        assert_eq!(
            got,
            [
                "App/filter.ax",
                "App/player.ocx",
                "App/shell/ext64.DLL",
                "Helper.dll"
            ]
        );
    }

    #[test]
    fn programs_are_found_by_extension() {
        let tmp = crate::util::tempdir().unwrap();
        let d = tmp.path().join("App");
        std::fs::create_dir_all(d.join("bin")).unwrap();
        for f in ["App.EXE", "bin/helper.exe", "bin/core.dll", "notes.exe.txt"] {
            std::fs::write(d.join(f), b"x").unwrap();
        }
        let mut got: Vec<String> = program_paths(&[Action::DeleteTree {
            path: d.to_string_lossy().into(),
        }])
        .iter()
        .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
        .collect();
        got.sort();
        assert_eq!(got, ["App.EXE", "helper.exe"]);
    }

    #[test]
    fn queries_with_nothing_to_ask_are_empty() {
        assert_eq!(close_programs(&[]), Closed::default());
        assert!(file_holders(&[]).is_empty());
        assert!(pause_explorer(&[]).is_none());
    }
}
