//! Where SetupScope keeps its data: install logs, backups, pending monitoring
//! sessions and settings.
//!
//! Installed mode: `%LOCALAPPDATA%\SetupScope`.
//! Portable mode (a file named `portable` next to the exe): `SetupScopeData\`
//! next to the exe.

pub(crate) mod durable;

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
                let mut s = Store::at(&dir.join("SetupScopeData"))?;
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
        Store::at(&base.join("SetupScope"))
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
    /// Record of the most recent deprovisioning run (data root).
    pub fn deprovision_report_path(&self) -> PathBuf {
        self.root.join("deprovision.json")
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
        let mut settings: Settings = std::fs::read_to_string(self.settings_path())
            .ok()
            .and_then(|s| crate::json::compat::from_str(&s).ok())
            .unwrap_or_default();
        settings.retire_interface_modes();
        settings.gatekeeper_enabled = false;
        settings.gatekeeper_scan_after_install = false;
        settings
    }

    pub fn save_settings(&self, s: &Settings) -> Result<()> {
        // Preserve a damaged or unreadable existing file for manual recovery.
        match std::fs::read_to_string(self.settings_path()) {
            Ok(text) => {
                let _: Settings = crate::json::compat::from_str(&text)
                .context("Existing settings are damaged; preserve settings.json and restore it from backup before saving")?;
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e).context("Read existing settings before replacement"),
        }
        durable::write(
            &self.settings_path(),
            crate::json::compat::to_string_pretty(s)?.as_bytes(),
            true,
        )
        .context("Save settings atomically")
    }

    /// Explicit reset preserves the exact previous file, including damaged JSON.
    /// Ordinary saves refuse to replace damaged settings.
    pub fn reset_settings(&self, settings: &Settings) -> Result<Option<PathBuf>> {
        use std::io::Read;
        let path = self.settings_path();
        durable::check_path(&path)?;
        let json = crate::json::compat::to_string_pretty(settings)?;
        let previous = match std::fs::File::open(&path) {
            Ok(file) => {
                anyhow::ensure!(
                    file.metadata()?.is_file(),
                    "Settings must be a regular file"
                );
                let mut bytes = Vec::new();
                file.take(2 * 1024 * 1024 + 1).read_to_end(&mut bytes)?;
                anyhow::ensure!(
                    bytes.len() <= 2 * 1024 * 1024,
                    "Settings exceed 2 MiB; preserve them for manual recovery before resetting"
                );
                Some(bytes)
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            Err(e) => return Err(e).context("Read settings before reset"),
        };
        let mut recovery = None;
        if let Some(bytes) = previous {
            for attempt in 0..128 {
                let candidate = self.root.join(format!(
                    "settings-recovery-{}-{}-{attempt}.json",
                    crate::util::new_id(),
                    std::process::id()
                ));
                match durable::write(&candidate, &bytes, false) {
                    Ok(()) => {
                        recovery = Some(candidate);
                        break;
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
                    Err(e) => return Err(e).context("Preserve previous settings before reset"),
                }
            }
            anyhow::ensure!(
                recovery.is_some(),
                "Cannot allocate a settings recovery file; reset refused"
            );
        }
        durable::write(&path, json.as_bytes(), true)
            .context("Reset settings; any recovery copy was retained")?;
        Ok(recovery)
    }

    fn log_path(&self, id: &str) -> Result<PathBuf> {
        crate::util::validate_local_id(id)?;
        Ok(self.logs_dir().join(format!("{id}.json")))
    }

    pub fn save_log(&self, log: &InstallLog) -> Result<()> {
        let destination = self.log_path(&log.id)?;
        // Compact JSON: large logs are much smaller and faster to write.
        let json = crate::json::compat::to_string(log)?;
        durable::write(&destination, json.as_bytes(), true).context("Save install log atomically")
    }

    pub fn load_log(&self, id: &str) -> Result<InstallLog> {
        let path = self.log_path(id)?;
        durable::check_path(&path)?;
        let s = std::fs::read_to_string(path).with_context(|| format!("log {id}"))?;
        let log: InstallLog = crate::json::compat::from_str(&s)?;
        anyhow::ensure!(
            log.id == id,
            "Install log ID does not match its filename; preserve the file for recovery"
        );
        Ok(log)
    }

    pub fn delete_log(&self, id: &str) -> Result<()> {
        std::fs::remove_file(self.log_path(id)?)?;
        Ok(())
    }

    /// Save the outcome of a deprovisioning run (atomic tmp+rename, like
    /// the install logs).
    pub fn save_deprovision_report(
        &self,
        report: &crate::debloat::DeprovisionReport,
    ) -> Result<()> {
        let json = crate::json::compat::to_string_pretty(report)?;
        durable::write(&self.deprovision_report_path(), json.as_bytes(), true)
            .context("Save deprovisioning report atomically")
    }

    /// The most recent deprovisioning run, if one was recorded. A corrupt
    /// or unreadable file is not an error here; it just means no report.
    pub fn load_deprovision_report(&self) -> Option<crate::debloat::DeprovisionReport> {
        std::fs::read_to_string(self.deprovision_report_path())
            .ok()
            .and_then(|s| crate::json::compat::from_str(&s).ok())
    }

    pub fn list_logs(&self) -> Vec<InstallLog> {
        let mut logs: Vec<InstallLog> = std::fs::read_dir(self.logs_dir())
            .into_iter()
            .flatten()
            .flatten()
            .filter(|e| e.path().extension().is_some_and(|x| x == "json"))
            .filter_map(|e| e.path().file_stem()?.to_str().map(str::to_owned))
            .filter_map(|id| self.load_log(&id).ok())
            .collect();
        logs.sort_by(|a, b| b.created.cmp(&a.created));
        logs
    }

    /// Import a log exported from another machine or an older install.
    pub fn import_log(&self, path: &Path) -> Result<InstallLog> {
        use std::io::Read;
        const MAX_IMPORT: u64 = 256 * 1024 * 1024;
        durable::check_path(path)?;
        let meta = std::fs::symlink_metadata(path)?;
        anyhow::ensure!(
            meta.is_file() && meta.len() <= MAX_IMPORT,
            "Install log import requires a regular file no larger than 256 MiB"
        );
        let mut bytes = Vec::new();
        std::fs::File::open(path)?
            .take(MAX_IMPORT + 1)
            .read_to_end(&mut bytes)?;
        anyhow::ensure!(
            bytes.len() as u64 <= MAX_IMPORT,
            "Install log import exceeds 256 MiB"
        );
        let mut log: InstallLog = crate::json::compat::from_str(std::str::from_utf8(&bytes)?)
            .context("Not a SetupScope install log")?;
        for attempt in 0..128 {
            if attempt > 0 || self.log_path(&log.id).is_err() {
                log.id = format!("{}-{}-{attempt}", crate::util::new_id(), std::process::id());
            }
            let destination = self.log_path(&log.id)?;
            let json = crate::json::compat::to_string(&log)?;
            match durable::write(&destination, json.as_bytes(), false) {
                Ok(()) => return Ok(log),
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(e) => return Err(e).context("Import log without replacing existing files"),
            }
        }
        anyhow::bail!("Cannot allocate an unused local log ID; no existing log was replaced")
    }

    // -- pending monitoring session ------------------------------------------

    pub fn save_pending(
        &self,
        session: &PendingSession,
        before: &crate::model::Snapshot,
    ) -> Result<()> {
        save_snapshot(&self.pending_dir().join("before.snap"), before)?;
        durable::write(
            &self.pending_dir().join("session.json"),
            crate::json::compat::to_string_pretty(session)?.as_bytes(),
            true,
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
        let _ = self.clear_pending_checked();
    }

    /// Report cleanup failures so cancellation does not silently claim success.
    pub fn clear_pending_checked(&self) -> Result<()> {
        for name in ["session.json", "before.snap"] {
            match std::fs::remove_file(self.pending_dir().join(name)) {
                Ok(()) => {},
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {},
                Err(e) => return Err(e.into()),
            }
        }
        Ok(())
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
    #[test]
    fn unsafe_log_ids_cannot_read_write_or_delete_outside_logs() {
        let tmp = crate::util::tempdir().unwrap();
        let st = Store::at(tmp.path()).unwrap();
        let outside = tmp.path().join("outside.json");
        std::fs::write(&outside, "preserve").unwrap();
        for id in [
            "../outside",
            "..\\outside",
            "C:\\outside",
            "outside:stream",
            "CON",
            "nul",
            "",
            ".",
            "a/b",
        ] {
            let log = InstallLog {
                id: id.into(),
                ..Default::default()
            };
            assert!(st.save_log(&log).is_err(), "{id}");
            assert!(st.load_log(id).is_err());
            assert!(st.delete_log(id).is_err());
        }
        assert_eq!(std::fs::read_to_string(outside).unwrap(), "preserve");
    }
    #[test]
    fn imported_unsafe_id_gets_a_local_id() {
        let tmp = crate::util::tempdir().unwrap();
        let st = Store::at(tmp.path()).unwrap();
        let input = tmp.path().join("input.json");
        let log = InstallLog {
            id: "../escaped".into(),
            ..Default::default()
        };
        std::fs::write(&input, crate::json::compat::to_string(&log).unwrap()).unwrap();
        let imported = st.import_log(&input).unwrap();
        assert!(st.log_path(&imported.id).unwrap().is_file());
        assert!(!tmp.path().join("escaped.json").exists());
    }
    #[test]
    fn simultaneous_imports_preserve_existing_logs() {
        let tmp = crate::util::tempdir().unwrap();
        let st = Store::at(tmp.path()).unwrap();
        let original = InstallLog {
            id: "shared".into(),
            name: "Original".into(),
            ..Default::default()
        };
        st.save_log(&original).unwrap();
        let input = tmp.path().join("input.json");
        let imported = InstallLog {
            id: "shared".into(),
            name: "Imported".into(),
            ..Default::default()
        };
        std::fs::write(&input, crate::json::compat::to_string(&imported).unwrap()).unwrap();
        let gate = std::sync::Arc::new(std::sync::Barrier::new(8));
        let threads: Vec<_> = (0..8)
            .map(|_| {
                let st = st.clone();
                let path = input.clone();
                let gate = gate.clone();
                std::thread::spawn(move || {
                    gate.wait();
                    st.import_log(&path).unwrap().id
                })
            })
            .collect();
        let ids: std::collections::HashSet<_> =
            threads.into_iter().map(|t| t.join().unwrap()).collect();
        assert_eq!(ids.len(), 8);
        assert_eq!(st.list_logs().len(), 9);
        assert_eq!(st.load_log("shared").unwrap().name, "Original");
        for id in ids {
            assert_eq!(st.load_log(&id).unwrap().name, "Imported");
        }
    }
    #[test]
    fn concurrent_updates_remain_complete_and_parseable() {
        let tmp = crate::util::tempdir().unwrap();
        let st = Store::at(tmp.path()).unwrap();
        let make = |c: char| InstallLog {
            id: "active".into(),
            name: c.to_string().repeat(16384),
            ..Default::default()
        };
        st.save_log(&make('a')).unwrap();
        let gate = std::sync::Arc::new(std::sync::Barrier::new(3));
        let threads: Vec<_> = ['a', 'b']
            .into_iter()
            .map(|c| {
                let st = st.clone();
                let gate = gate.clone();
                let log = make(c);
                std::thread::spawn(move || {
                    gate.wait();
                    for _ in 0..20 {
                        st.save_log(&log).unwrap();
                    }
                })
            })
            .collect();
        gate.wait();
        for _ in 0..80 {
            let log = st.load_log("active").unwrap();
            assert!(log.name == "a".repeat(16384) || log.name == "b".repeat(16384));
        }
        for thread in threads {
            thread.join().unwrap();
        }
        assert!(std::fs::read_dir(st.logs_dir()).unwrap().all(|e| {
            let path = e.unwrap().path();
            path.file_name()
                .is_some_and(|n| n == ".setupscope-write.lock")
                || path.extension().is_some_and(|x| x == "json")
        }));
    }
    #[test]
    fn damaged_settings_and_legacy_staging_files_are_preserved() {
        let tmp = crate::util::tempdir().unwrap();
        let st = Store::at(tmp.path()).unwrap();
        std::fs::write(st.settings_path(), "damaged").unwrap();
        assert!(st.save_settings(&Settings::default()).is_err());
        assert_eq!(
            std::fs::read_to_string(st.settings_path()).unwrap(),
            "damaged"
        );
        let stale = st.logs_dir().join("valid.tmp");
        std::fs::write(&stale, "prior interrupted data").unwrap();
        st.save_log(&InstallLog {
            id: "valid".into(),
            ..Default::default()
        })
        .unwrap();
        assert_eq!(
            std::fs::read_to_string(stale).unwrap(),
            "prior interrupted data"
        );
    }
    #[test]
    fn mismatched_receipt_id_never_selects_another_log() {
        let tmp = crate::util::tempdir().unwrap();
        let st = Store::at(tmp.path()).unwrap();
        let log = InstallLog {
            id: "actual".into(),
            ..Default::default()
        };
        st.save_log(&log).unwrap();
        std::fs::copy(
            st.log_path("actual").unwrap(),
            st.log_path("impostor").unwrap(),
        )
        .unwrap();
        assert!(st.load_log("impostor").is_err());
        assert_eq!(st.list_logs().len(), 1);
    }
    #[test]
    fn oversized_import_is_rejected_without_writing_a_log() {
        let tmp = crate::util::tempdir().unwrap();
        let st = Store::at(tmp.path()).unwrap();
        let input = tmp.path().join("huge.json");
        let file = std::fs::File::create(&input).unwrap();
        file.set_len(256 * 1024 * 1024 + 1).unwrap();
        drop(file);
        assert!(st.import_log(&input).is_err());
        assert!(st.list_logs().is_empty());
    }
    #[cfg(windows)]
    #[test]
    fn locked_destination_keeps_previous_complete_log() {
        use std::os::windows::fs::OpenOptionsExt;
        let tmp = crate::util::tempdir().unwrap();
        let st = Store::at(tmp.path()).unwrap();
        let mut log = InstallLog {
            id: "locked".into(),
            name: "Before".into(),
            ..Default::default()
        };
        st.save_log(&log).unwrap();
        let held = std::fs::OpenOptions::new()
            .read(true)
            .share_mode(0)
            .open(st.log_path("locked").unwrap())
            .unwrap();
        log.name = "After".into();
        assert!(st.save_log(&log).is_err());
        drop(held);
        assert_eq!(st.load_log("locked").unwrap().name, "Before");
        assert_eq!(std::fs::read_dir(st.logs_dir()).unwrap().count(), 2);
    }
    #[test]
    fn explicit_reset_preserves_damaged_settings_verbatim() {
        let tmp = crate::util::tempdir().unwrap();
        let st = Store::at(tmp.path()).unwrap();
        let damaged = b"invalid json \xff";
        std::fs::write(st.settings_path(), damaged).unwrap();
        let backup = st.reset_settings(&Settings::default()).unwrap().unwrap();
        assert_eq!(std::fs::read(backup).unwrap(), damaged);
        let _: Settings =
            crate::json::compat::from_str(&std::fs::read_to_string(st.settings_path()).unwrap())
                .unwrap();
    }
    #[test]
    fn retired_gatekeeper_preferences_do_not_reactivate_old_queues() {
        let tmp = crate::util::tempdir().unwrap();
        let st = Store::at(tmp.path()).unwrap();
        let original = Settings {
            gatekeeper_enabled: true,
            gatekeeper_scan_after_install: true,
            ..Default::default()
        };
        st.save_settings(&original).unwrap();
        let bytes = std::fs::read(st.settings_path()).unwrap();
        let loaded = st.load_settings();
        assert!(!loaded.gatekeeper_enabled);
        assert!(!loaded.gatekeeper_scan_after_install);
        assert_eq!(std::fs::read(st.settings_path()).unwrap(), bytes);
    }
}
