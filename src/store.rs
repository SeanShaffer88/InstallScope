//! Where InstallScope keeps its data: install logs, backups, pending monitoring
//! sessions and settings.
//!
//! Installed mode: `%LOCALAPPDATA%\InstallScope`.
//! Portable mode (a file named `portable` next to the exe): `InstallScopeData\`
//! next to the exe.

use crate::model::InstallLog;
use crate::settings::Settings;
use crate::snapshot::{load_snapshot, save_snapshot};
use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

/// A monitoring session waiting for the user to finish installing.
#[derive(Clone, Debug, Default)]
pub struct PendingSession {
    pub name: String,
    pub started: String,
    pub installer: String,
    /// A trial install ("Try it, then undo"): removed again after review.
    pub trial: bool,
}
crate::json_struct!(PendingSession, { name, started, installer [default], trial [default] });

#[derive(Clone, Debug)]
pub struct Store {
    pub root: PathBuf,
    pub portable: bool,
}

impl Store {
    /// Open the default data folder for this installation.
    pub fn open() -> Result<Store> {
        let exe_dir = std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(Path::to_path_buf));
        if let Some(dir) = &exe_dir {
            if dir.join("portable").exists() || dir.join("portable.txt").exists() {
                let mut s = Store::at(&dir.join("InstallScopeData"))?;
                s.portable = true;
                return Ok(s);
            }
        }
        let base = std::env::var("LOCALAPPDATA")
            .map(PathBuf::from)
            .or_else(|_| {
                std::env::var("HOME").map(|h| PathBuf::from(h).join(".local").join("share"))
            })
            .unwrap_or_else(|_| PathBuf::from("."));
        Store::at(&base.join("InstallScope"))
    }

    pub fn at(root: &Path) -> Result<Store> {
        for d in ["logs", "backups", "pending", "snapshots"] {
            std::fs::create_dir_all(root.join(d))
                .with_context(|| format!("create {}", root.join(d).display()))?;
        }
        Ok(Store {
            root: root.to_path_buf(),
            portable: false,
        })
    }

    pub fn logs_dir(&self) -> PathBuf {
        self.root.join("logs")
    }
    pub fn backups_dir(&self) -> PathBuf {
        self.root.join("backups")
    }
    /// Cached program sizes and signature results.
    pub fn cache_path(&self) -> PathBuf {
        self.root.join("cache.json")
    }
    /// Slowdown Detective history (`history.json` in the data root).
    pub fn history_path(&self) -> PathBuf {
        crate::history::HistoryStore::path(&self.root)
    }
    /// Autostart Gatekeeper state: the baseline and the review queue.
    pub fn gatekeeper_path(&self) -> PathBuf {
        self.root.join("gatekeeper.json")
    }

    pub fn snapshots_dir(&self) -> PathBuf {
        self.root.join("snapshots")
    }

    /// Named snapshots taken from the Installer Lab: (name, path, modified).
    pub fn list_snapshots(&self) -> Vec<(String, PathBuf, String)> {
        let mut v: Vec<(String, PathBuf, String)> = std::fs::read_dir(self.snapshots_dir())
            .into_iter()
            .flatten()
            .flatten()
            .filter(|e| e.path().extension().is_some_and(|x| x == "snap"))
            .map(|e| {
                let name = e
                    .path()
                    .file_stem()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .to_string();
                let when = e
                    .metadata()
                    .and_then(|m| m.modified())
                    .map(|t| crate::time::DateTime::from_system_time_local(t).ymd_hm())
                    .unwrap_or_default();
                (name, e.path(), when)
            })
            .collect();
        v.sort_by(|a, b| b.2.cmp(&a.2));
        v
    }

    fn pending_dir(&self) -> PathBuf {
        self.root.join("pending")
    }
    fn settings_path(&self) -> PathBuf {
        self.root.join("settings.json")
    }

    pub fn load_settings(&self) -> Settings {
        std::fs::read_to_string(self.settings_path())
            .ok()
            .and_then(|s| crate::json::compat::from_str(&s).ok())
            .unwrap_or_default()
    }

    pub fn save_settings(&self, s: &Settings) -> Result<()> {
        std::fs::write(
            self.settings_path(),
            crate::json::compat::to_string_pretty(s)?,
        )?;
        Ok(())
    }

    fn log_path(&self, id: &str) -> PathBuf {
        self.logs_dir().join(format!("{id}.json"))
    }

    pub fn save_log(&self, log: &InstallLog) -> Result<()> {
        // Compact JSON: large logs are much smaller and faster to write.
        let json = crate::json::compat::to_string(log)?;
        let tmp = self.logs_dir().join(format!("{}.tmp", log.id));
        std::fs::write(&tmp, json)?;
        std::fs::rename(&tmp, self.log_path(&log.id))?;
        Ok(())
    }

    pub fn load_log(&self, id: &str) -> Result<InstallLog> {
        let s = std::fs::read_to_string(self.log_path(id)).with_context(|| format!("log {id}"))?;
        Ok(crate::json::compat::from_str(&s)?)
    }

    pub fn delete_log(&self, id: &str) -> Result<()> {
        std::fs::remove_file(self.log_path(id))?;
        Ok(())
    }

    pub fn list_logs(&self) -> Vec<InstallLog> {
        let mut logs: Vec<InstallLog> = std::fs::read_dir(self.logs_dir())
            .into_iter()
            .flatten()
            .flatten()
            .filter(|e| e.path().extension().is_some_and(|x| x == "json"))
            .filter_map(|e| std::fs::read_to_string(e.path()).ok())
            .filter_map(|s| crate::json::compat::from_str(&s).ok())
            .collect();
        logs.sort_by(|a, b| b.created.cmp(&a.created));
        logs
    }

    /// Import a log exported from another machine or an older install.
    pub fn import_log(&self, path: &Path) -> Result<InstallLog> {
        let mut log: InstallLog = crate::json::compat::from_str(&std::fs::read_to_string(path)?)
            .context("not an InstallScope install log")?;
        if log.id.is_empty() || self.log_path(&log.id).exists() {
            log.id = crate::util::new_id();
        }
        self.save_log(&log)?;
        Ok(log)
    }

    // -- pending monitoring session ------------------------------------------

    pub fn save_pending(
        &self,
        session: &PendingSession,
        before: &crate::model::Snapshot,
    ) -> Result<()> {
        save_snapshot(&self.pending_dir().join("before.snap"), before)?;
        std::fs::write(
            self.pending_dir().join("session.json"),
            crate::json::compat::to_string_pretty(session)?,
        )?;
        Ok(())
    }

    pub fn pending(&self) -> Option<PendingSession> {
        let s = std::fs::read_to_string(self.pending_dir().join("session.json")).ok()?;
        crate::json::compat::from_str(&s).ok()
    }

    pub fn pending_snapshot(&self) -> Result<crate::model::Snapshot> {
        load_snapshot(&self.pending_dir().join("before.snap"))
    }

    pub fn clear_pending(&self) {
        let _ = std::fs::remove_file(self.pending_dir().join("session.json"));
        let _ = std::fs::remove_file(self.pending_dir().join("before.snap"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn logs_and_settings_persist() {
        let tmp = crate::util::tempdir().unwrap();
        let st = Store::at(tmp.path()).unwrap();
        let mut s = st.load_settings();
        s.language = "de".into();
        st.save_settings(&s).unwrap();
        assert_eq!(st.load_settings().language, "de");

        let log = InstallLog {
            id: "a".into(),
            name: "App".into(),
            created: "2026".into(),
            ..Default::default()
        };
        st.save_log(&log).unwrap();
        assert_eq!(st.list_logs().len(), 1);
        let exported = tmp.path().join("exp.json");
        std::fs::copy(st.logs_dir().join("a.json"), &exported).unwrap();
        let imported = st.import_log(&exported).unwrap();
        assert_ne!(imported.id, "a");
        assert_eq!(st.list_logs().len(), 2);
        st.delete_log("a").unwrap();
        assert_eq!(st.list_logs().len(), 1);

        assert!(st.pending().is_none());
        st.save_pending(
            &PendingSession {
                name: "x".into(),
                ..Default::default()
            },
            &Default::default(),
        )
        .unwrap();
        assert_eq!(st.pending().unwrap().name, "x");
        assert!(st.pending_snapshot().is_ok());
        st.clear_pending();
        assert!(st.pending().is_none());
    }
}
