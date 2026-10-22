//! Opt-in, bounded storage baselines. Saved data is display-only and never
//! authorizes filesystem changes or automatic scans.
use crate::storage::{StorageReport, ROW_LIMIT};
use anyhow::{bail, Context, Result};
use std::collections::HashSet;
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};

const VERSION: u32 = 1;
const MAX_BYTES: u64 = 2 * 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BaselineRow {
    pub name: String,
    pub folder: bool,
    pub bytes: u64,
}
crate::json_struct!(BaselineRow, { name, folder, bytes });

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Baseline {
    version: u32,
    pub saved_at: i64,
    pub scanned_at: i64,
    pub root: String,
    pub bytes: u64,
    pub root_entries: u64,
    pub rows: Vec<BaselineRow>,
}
crate::json_struct!(Baseline, { version, saved_at, scanned_at, root, bytes, root_entries, rows });

#[derive(Debug)]
pub struct Comparison {
    pub total_change: i128,
    /// One entry per current report row. None means it was outside the saved
    /// top list, absent, or has changed type; its previous size is unknown.
    pub row_changes: Vec<Option<i128>>,
    pub unmatched_baseline_rows: usize,
}

pub fn path(root: &Path) -> PathBuf {
    root.join("storage-baseline.json")
}

impl Baseline {
    pub fn from_report(report: &StorageReport) -> Result<Self> {
        if report.incomplete() || report.rows.iter().any(|row| row.incomplete) {
            bail!("Finish a complete storage scan before saving a baseline. Cancelled scans, skipped links, and errors cannot be used for growth comparisons.");
        }
        let rows = report
            .rows
            .iter()
            .map(|row| {
                // A lossy path must never silently identify a different entry.
                let name = row
                    .path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .context("A storage entry name cannot be saved as Unicode")?;
                if row.path.parent() != Some(report.root.as_path()) {
                    bail!("Storage row is outside the scanned folder");
                }
                Ok(BaselineRow {
                    name: name.into(),
                    folder: row.folder,
                    bytes: row.totals.bytes,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        let baseline = Self {
            version: VERSION,
            saved_at: crate::time::unix_now(),
            scanned_at: report.scanned_at,
            root: report
                .root
                .to_str()
                .context("The storage root cannot be saved as Unicode")?
                .into(),
            bytes: report.totals.bytes,
            root_entries: report.root_entries,
            rows,
        };
        baseline.validate()?;
        Ok(baseline)
    }

    fn validate(&self) -> Result<()> {
        if self.version != VERSION {
            bail!("Unsupported storage baseline version {}; this build reads version {VERSION}. Saved data was left untouched.", self.version);
        }
        if !Path::new(&self.root).is_absolute()
            || self.root.contains('\0')
            || !(0..=253_402_300_799).contains(&self.saved_at)
            || !(0..=253_402_300_799).contains(&self.scanned_at)
        {
            bail!("Invalid storage baseline root or timestamp; saved data was left untouched.");
        }
        if self.rows.len() > ROW_LIMIT
            || self.rows.len() as u64 != self.root_entries.min(ROW_LIMIT as u64)
        {
            bail!("Invalid storage baseline row count; saved data was left untouched.");
        }
        let mut names = HashSet::new();
        let mut sum = 0u64;
        for row in &self.rows {
            let mut components = Path::new(&row.name).components();
            if !matches!(components.next(), Some(Component::Normal(_)))
                || components.next().is_some()
                || row.name.contains(['/', '\\', '\0'])
                || !names.insert(&row.name)
            {
                bail!(
                    "Invalid or duplicate storage baseline entry; saved data was left untouched."
                );
            }
            sum = sum
                .checked_add(row.bytes)
                .context("Storage baseline sizes overflow")?;
        }
        if sum > self.bytes || (self.root_entries <= ROW_LIMIT as u64 && sum != self.bytes) {
            bail!("Inconsistent storage baseline sizes; saved data was left untouched.");
        }
        Ok(())
    }

    pub fn compare(&self, report: &StorageReport) -> Result<Comparison> {
        self.validate()?;
        // Validate current scan completeness and row shape by the same rules.
        let current = Self::from_report(report)?;
        if Path::new(&self.root) != report.root {
            bail!("This baseline belongs to another folder. Scan its displayed root to compare, or explicitly replace the baseline.");
        }
        if report.scanned_at < self.scanned_at {
            bail!("The displayed scan is older than this baseline. Rescan the same folder before comparing growth.");
        }
        let row_changes = current
            .rows
            .iter()
            .map(|row| {
                self.rows
                    .iter()
                    .find(|old| old.name == row.name && old.folder == row.folder)
                    .map(|old| i128::from(row.bytes) - i128::from(old.bytes))
            })
            .collect();
        let unmatched_baseline_rows = self
            .rows
            .iter()
            .filter(|old| {
                !current
                    .rows
                    .iter()
                    .any(|row| row.name == old.name && row.folder == old.folder)
            })
            .count();
        Ok(Comparison {
            total_change: i128::from(report.totals.bytes) - i128::from(self.bytes),
            row_changes,
            unmatched_baseline_rows,
        })
    }
}

/// Missing is normal. Corrupt, future, oversized, and inaccessible files are errors.
pub fn load(root: &Path) -> Result<Option<Baseline>> {
    let file = match std::fs::File::open(path(root)) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error).context("Open saved storage baseline"),
    };
    let mut bytes = Vec::new();
    file.take(MAX_BYTES + 1)
        .read_to_end(&mut bytes)
        .context("Read saved storage baseline")?;
    if bytes.len() as u64 > MAX_BYTES {
        bail!("Saved storage baseline exceeds the 2 MiB limit; file left untouched.");
    }
    let baseline: Baseline = crate::json::compat::from_slice(&bytes).context(
        "Saved storage baseline is damaged; copy it elsewhere for recovery or explicitly delete it",
    )?;
    baseline.validate()?;
    Ok(Some(baseline))
}

/// Replaces only the baseline, after an explicit UI confirmation. Refuses to
/// replace unreadable or future data. A failed write leaves no partial record.
pub fn save(root: &Path, baseline: &Baseline) -> Result<()> {
    baseline.validate()?;
    let _ = load(root)?;
    let bytes = crate::json::compat::to_vec(baseline)?;
    if bytes.len() as u64 > MAX_BYTES {
        bail!("Storage baseline exceeds the 2 MiB save limit; previous data was left untouched.");
    }
    let tmp = root.join(format!("storage-baseline-{}.tmp", crate::util::new_id()));
    let result = (|| -> Result<()> {
        let mut file = std::fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&tmp)?;
        file.write_all(&bytes)?;
        file.sync_all()?;
        drop(file);
        std::fs::rename(&tmp, path(root)).context("Replace saved storage baseline")?;
        Ok(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result.context("Save storage baseline; scan results remain available")
}

/// Never follows or modifies any scanned path stored in the baseline.
pub fn delete(root: &Path) -> Result<()> {
    match std::fs::remove_file(path(root)) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error).context("Delete saved storage baseline"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::snapshot::Progress;
    use crate::storage;
    use std::fs;

    #[test]
    fn save_load_compare_replace_and_delete_never_touch_scanned_files() {
        let data = crate::util::tempdir().unwrap();
        let source = crate::util::tempdir().unwrap();
        fs::create_dir(source.path().join("folder")).unwrap();
        fs::write(source.path().join("folder/file"), b"123").unwrap();
        fs::write(source.path().join("other"), b"12345").unwrap();
        assert!(load(data.path()).unwrap().is_none());
        let before = storage::scan(source.path(), &Progress::default()).unwrap();
        let baseline = Baseline::from_report(&before).unwrap();
        save(data.path(), &baseline).unwrap();
        assert_eq!(load(data.path()).unwrap().unwrap(), baseline);
        fs::write(source.path().join("folder/file"), b"1234567").unwrap();
        fs::write(source.path().join("other"), b"12").unwrap();
        let after = storage::scan(source.path(), &Progress::default()).unwrap();
        let comparison = baseline.compare(&after).unwrap();
        assert_eq!(comparison.total_change, 1);
        assert_eq!(comparison.row_changes, vec![Some(4), Some(-3)]);
        assert_eq!(comparison.unmatched_baseline_rows, 0);
        let replacement = Baseline::from_report(&after).unwrap();
        save(data.path(), &replacement).unwrap();
        assert_eq!(load(data.path()).unwrap().unwrap(), replacement);
        delete(data.path()).unwrap();
        delete(data.path()).unwrap();
        assert!(load(data.path()).unwrap().is_none());
        assert_eq!(
            fs::read(source.path().join("folder/file")).unwrap(),
            b"1234567"
        );
        assert_eq!(fs::read(source.path().join("other")).unwrap(), b"12");
    }

    #[test]
    fn omitted_entries_and_type_changes_are_unknown_instead_of_zero() {
        let source = crate::util::tempdir().unwrap();
        for index in 0..ROW_LIMIT + 1 {
            fs::write(source.path().join(format!("{index:04}")), b"x").unwrap();
        }
        let before = storage::scan(source.path(), &Progress::default()).unwrap();
        let baseline = Baseline::from_report(&before).unwrap();
        let last = source.path().join(format!("{ROW_LIMIT:04}"));
        fs::write(last, b"now the largest").unwrap();
        let after = storage::scan(source.path(), &Progress::default()).unwrap();
        let compared = baseline.compare(&after).unwrap();
        assert_eq!(compared.row_changes[0], None);
        assert_eq!(compared.unmatched_baseline_rows, 1);
        assert_eq!(compared.total_change, 14);
        fs::remove_file(source.path().join("0000")).unwrap();
        fs::create_dir(source.path().join("0000")).unwrap();
        fs::write(source.path().join("0000/new"), vec![b'x'; 30]).unwrap();
        let changed_type = storage::scan(source.path(), &Progress::default()).unwrap();
        assert_eq!(
            baseline.compare(&changed_type).unwrap().row_changes[0],
            None
        );
    }

    #[test]
    fn partial_cancelled_and_different_root_reports_cannot_be_compared() {
        let source = crate::util::tempdir().unwrap();
        let other = crate::util::tempdir().unwrap();
        let report = storage::scan(source.path(), &Progress::default()).unwrap();
        let baseline = Baseline::from_report(&report).unwrap();
        let mut older = report.clone();
        older.scanned_at -= 1;
        assert!(baseline.compare(&older).is_err());
        assert!(baseline
            .compare(&storage::scan(other.path(), &Progress::default()).unwrap())
            .is_err());
        for (cancelled, errors, links) in [(true, 0, 0), (false, 1, 0), (false, 0, 1)] {
            let mut partial = report.clone();
            partial.cancelled = cancelled;
            partial.errors = errors;
            partial.totals.skipped_links = links;
            assert!(Baseline::from_report(&partial).is_err());
            assert!(baseline.compare(&partial).is_err());
        }
    }

    #[test]
    fn malformed_future_oversized_and_inconsistent_baselines_are_preserved() {
        let data = crate::util::tempdir().unwrap();
        let source = crate::util::tempdir().unwrap();
        fs::write(source.path().join("file"), b"x").unwrap();
        let good =
            Baseline::from_report(&storage::scan(source.path(), &Progress::default()).unwrap())
                .unwrap();
        let mut variants = vec![b"broken".to_vec(), vec![b' '; MAX_BYTES as usize + 1]];
        for modify in [
            |b: &mut Baseline| b.version = VERSION + 1,
            |b: &mut Baseline| b.rows[0].name = "../escape".into(),
            |b: &mut Baseline| b.rows[0].name = "..\\escape".into(),
            |b: &mut Baseline| b.bytes = 0,
            |b: &mut Baseline| b.root_entries = 2,
            |b: &mut Baseline| b.root = "relative".into(),
            |b: &mut Baseline| {
                b.rows.push(b.rows[0].clone());
                b.root_entries = 2;
                b.bytes = 2;
            },
        ] {
            let mut invalid = good.clone();
            modify(&mut invalid);
            variants.push(crate::json::compat::to_vec(&invalid).unwrap());
        }
        for bytes in variants {
            fs::write(path(data.path()), &bytes).unwrap();
            assert!(load(data.path()).is_err());
            assert!(save(data.path(), &good).is_err());
            assert_eq!(fs::read(path(data.path())).unwrap(), bytes);
        }
        assert_eq!(fs::read(source.path().join("file")).unwrap(), b"x");
    }

    #[test]
    fn unsigned_size_extremes_compare_without_overflow_and_failed_writes_leave_no_temp() {
        let source = crate::util::tempdir().unwrap();
        let mut report = storage::scan(source.path(), &Progress::default()).unwrap();
        // Simulate a measured large file without allocating its contents.
        report.root_entries = 1;
        report.rows.push(storage::StorageRow {
            path: report.root.join("huge"),
            folder: false,
            totals: storage::Totals {
                bytes: u64::MAX,
                files: 1,
                ..Default::default()
            },
            incomplete: false,
        });
        report.totals.bytes = u64::MAX;
        let baseline = Baseline::from_report(&report).unwrap();
        report.rows[0].totals.bytes = 0;
        report.totals.bytes = 0;
        assert_eq!(
            baseline.compare(&report).unwrap().total_change,
            -i128::from(u64::MAX)
        );
        let data = crate::util::tempdir().unwrap();
        assert!(save(&data.path().join("missing"), &baseline).is_err());
        assert_eq!(fs::read_dir(data.path()).unwrap().count(), 0);
    }

    #[cfg(windows)]
    #[test]
    fn replacement_failure_keeps_the_old_baseline_and_removes_the_staged_file() {
        use std::os::windows::fs::OpenOptionsExt;
        let source = crate::util::tempdir().unwrap();
        let data = crate::util::tempdir().unwrap();
        let report = storage::scan(source.path(), &Progress::default()).unwrap();
        let baseline = Baseline::from_report(&report).unwrap();
        save(data.path(), &baseline).unwrap();
        let original = fs::read(path(data.path())).unwrap();
        // Permit reading the old record, but deny rename/delete while held.
        let _held = fs::OpenOptions::new()
            .read(true)
            .share_mode(1)
            .open(path(data.path()))
            .unwrap();
        assert!(save(data.path(), &baseline).is_err());
        assert_eq!(fs::read(path(data.path())).unwrap(), original);
        assert_eq!(fs::read_dir(data.path()).unwrap().count(), 1);
    }
}
