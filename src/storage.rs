//! Bounded, read-only storage survey. Sizes are logical, not reclaim estimates.
use crate::snapshot::Progress;
use anyhow::{bail, Context, Result};
use std::fs::{self, Metadata, ReadDir};
use std::path::{Path, PathBuf};

pub const ROW_LIMIT: usize = 200;
const ERROR_LIMIT: usize = 100;
const DEPTH_LIMIT: usize = 256;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Totals {
    pub bytes: u64,
    pub files: u64,
    pub folders: u64,
    pub skipped_links: u64,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StorageRow {
    pub path: PathBuf,
    pub folder: bool,
    pub totals: Totals,
    pub incomplete: bool,
}
#[derive(Clone, Debug, Default)]
pub struct StorageReport {
    pub allocation: Option<crate::storage_allocation::Summary>,
    pub root: PathBuf,
    /// UTC seconds at the start of this live metadata survey, not a snapshot time.
    pub scanned_at: i64,
    pub totals: Totals,
    pub rows: Vec<StorageRow>,
    pub root_entries: u64,
    pub errors: u64,
    pub error_details: Vec<String>,
    pub cancelled: bool,
}
impl StorageReport {
    /// Only measured directory rows can be selected for drill-down. The scan
    /// rechecks their metadata because entries may change after this report.
    pub fn folder_at(&self, index: usize) -> Option<&Path> {
        self.rows
            .get(index)
            .filter(|row| row.folder)
            .map(|row| row.path.as_path())
    }

    pub fn incomplete(&self) -> bool {
        self.cancelled || self.errors != 0 || self.totals.skipped_links != 0
    }
    fn issue(&mut self, path: &Path, error: impl std::fmt::Display) {
        self.errors = self.errors.saturating_add(1);
        if self.error_details.len() < ERROR_LIMIT {
            self.error_details
                .push(format!("{}: {error}", path.display()));
        }
    }
    fn retain(&mut self, row: StorageRow) {
        // Bounded top list: large directories sort first, paths break ties.
        let at = self.rows.partition_point(|existing| {
            existing.totals.bytes > row.totals.bytes
                || (existing.totals.bytes == row.totals.bytes && existing.path < row.path)
        });
        if at < ROW_LIMIT {
            self.rows.insert(at, row);
            self.rows.truncate(ROW_LIMIT);
        }
    }
}

fn link_or_reparse(meta: &Metadata) -> bool {
    if meta.file_type().is_symlink() {
        return true;
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if meta.file_attributes() & 0x400 != 0 {
            return true;
        }
    }
    false
}

fn inspect(
    path: &Path,
    totals: &mut Totals,
    report: &mut StorageReport,
    accounting: &mut Option<crate::storage_allocation::Accounting>,
) -> Option<bool> {
    match fs::symlink_metadata(path) {
        Err(error) => {
            report.issue(path, error);
            None
        }
        Ok(meta) if link_or_reparse(&meta) => {
            totals.skipped_links += 1;
            None
        }
        Ok(meta) if meta.is_file() => {
            if let Some(accounting) = accounting {
                accounting.observe(path, meta.len());
            }
            totals.files += 1;
            match totals.bytes.checked_add(meta.len()) {
                Some(bytes) => totals.bytes = bytes,
                None => {
                    totals.bytes = u64::MAX;
                    report.issue(path, "Logical size overflow; totals are incomplete");
                }
            }
            Some(false)
        }
        Ok(meta) if meta.is_dir() => {
            totals.folders += 1;
            Some(true)
        }
        Ok(_) => {
            report.issue(path, "Special filesystem entry not measured");
            None
        }
    }
}

fn subtree(
    path: &Path,
    totals: &mut Totals,
    report: &mut StorageReport,
    progress: &Progress,
    accounting: &mut Option<crate::storage_allocation::Accounting>,
) {
    let first = match fs::read_dir(path) {
        Ok(rd) => rd,
        Err(error) => {
            report.issue(path, error);
            return;
        }
    };
    let mut stack: Vec<(PathBuf, ReadDir)> = vec![(path.into(), first)];
    while !stack.is_empty() {
        if progress.cancelled() {
            report.cancelled = true;
            break;
        }
        let next = stack.last_mut().unwrap().1.next();
        let entry = match next {
            None => {
                stack.pop();
                continue;
            }
            Some(Err(error)) => {
                report.issue(&stack.last().unwrap().0, error);
                continue;
            }
            Some(Ok(entry)) => entry,
        };
        progress.tick();
        let path = entry.path();
        if inspect(&path, totals, report, accounting) == Some(true) {
            if stack.len() >= DEPTH_LIMIT {
                report.issue(
                    &path,
                    "Directory depth limit reached; contents not measured",
                );
            } else {
                match fs::read_dir(&path) {
                    Ok(rd) => stack.push((path, rd)),
                    Err(error) => report.issue(&path, error),
                }
            }
        }
    }
}

/// Enumerate only the explicitly selected root. Links/reparse entries observed
/// during metadata checks are skipped. Filesystem changes can race these checks;
/// this read-only survey is neither a snapshot nor a security boundary.
pub fn scan(root: &Path, progress: &Progress) -> Result<StorageReport> {
    scan_with_allocation(root, progress, false)
}

pub fn scan_with_allocation(
    root: &Path,
    progress: &Progress,
    allocation: bool,
) -> Result<StorageReport> {
    let mut accounting = allocation.then(crate::storage_allocation::Accounting::default);
    let scanned_at = crate::time::unix_now();
    let meta = fs::symlink_metadata(root).context("Inspect selected storage folder")?;
    if link_or_reparse(&meta) {
        bail!("Choose a real folder instead of a link or reparse point.");
    }
    if !meta.is_dir() {
        bail!("Choose a folder or drive root, not a file.");
    }
    let root = fs::canonicalize(root).context("Resolve selected storage folder")?;
    let entries = fs::read_dir(&root)
        .context("Read selected storage folder; check access or choose another folder")?;
    let mut report = StorageReport {
        root: root.clone(),
        scanned_at,
        ..Default::default()
    };
    for entry in entries {
        if progress.cancelled() {
            report.cancelled = true;
            break;
        }
        let entry = match entry {
            Ok(entry) => entry,
            Err(error) => {
                report.issue(&root, error);
                continue;
            }
        };
        progress.tick();
        report.root_entries += 1;
        let path = entry.path();
        let errors_before = report.errors;
        let mut totals = Totals::default();
        let folder = inspect(&path, &mut totals, &mut report, &mut accounting).unwrap_or(false);
        if folder {
            subtree(&path, &mut totals, &mut report, progress, &mut accounting);
        }
        report.totals.files += totals.files;
        report.totals.folders += totals.folders;
        report.totals.skipped_links += totals.skipped_links;
        match report.totals.bytes.checked_add(totals.bytes) {
            Some(bytes) => report.totals.bytes = bytes,
            None => {
                report.totals.bytes = u64::MAX;
                report.issue(&root, "Logical size overflow; totals are incomplete");
            }
        }
        let incomplete =
            report.errors != errors_before || totals.skipped_links != 0 || report.cancelled;
        report.retain(StorageRow {
            path,
            folder,
            totals,
            incomplete,
        });
        if report.cancelled {
            break;
        }
    }
    report.allocation = accounting.map(|a| a.summary);
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(windows)]
    #[test]
    fn optional_accounting_deduplicates_across_rows_without_changing_logical_baselines() {
        let tmp = crate::util::tempdir().unwrap();
        fs::create_dir(tmp.path().join("first")).unwrap();
        fs::create_dir(tmp.path().join("second")).unwrap();
        let original = tmp.path().join("first/a");
        fs::write(&original, vec![1u8; 8192]).unwrap();
        fs::hard_link(&original, tmp.path().join("second/b")).unwrap();
        let logical = scan(tmp.path(), &Progress::default()).unwrap();
        assert!(logical.allocation.is_none());
        let measured = scan_with_allocation(tmp.path(), &Progress::default(), true).unwrap();
        assert_eq!(measured.totals, logical.totals);
        assert_eq!(measured.totals.bytes, 16384);
        let accounting = measured.allocation.as_ref().unwrap();
        assert_eq!(
            (
                accounting.unique_logical_bytes,
                accounting.unique_files,
                accounting.repeated_links,
                accounting.unknown_files
            ),
            (8192, 1, 1, 0)
        );
        let baseline = crate::storage_baseline::Baseline::from_report(&logical).unwrap();
        assert_eq!(baseline.compare(&measured).unwrap().total_change, 0);
        let progress = Progress::default();
        progress
            .cancel
            .store(true, std::sync::atomic::Ordering::Relaxed);
        let partial = scan_with_allocation(tmp.path(), &progress, true).unwrap();
        assert!(partial.cancelled && partial.incomplete());
        assert_eq!(partial.allocation.unwrap().unique_files, 0);
    }
    #[test]
    fn drill_down_measures_only_the_selected_folder_and_rechecks_it() {
        let tmp = crate::util::tempdir().unwrap();
        let nested = tmp.path().join("nested");
        fs::create_dir(&nested).unwrap();
        fs::write(nested.join("inside"), b"123456").unwrap();
        fs::write(tmp.path().join("outside"), b"789").unwrap();
        let report = scan(tmp.path(), &Progress::default()).unwrap();
        let index = report.rows.iter().position(|row| row.folder).unwrap();
        let selected = report.folder_at(index).unwrap();
        let detail = scan(selected, &Progress::default()).unwrap();
        assert_eq!(detail.totals.bytes, 6);
        assert_eq!(detail.totals.files, 1);
        assert_eq!(detail.rows.len(), 1);
        assert_eq!(detail.rows[0].path.file_name().unwrap(), "inside");
        let file_index = report.rows.iter().position(|row| !row.folder).unwrap();
        assert!(report.folder_at(file_index).is_none());
        assert!(report.folder_at(usize::MAX).is_none());
        fs::remove_file(nested.join("inside")).unwrap();
        fs::remove_dir(&nested).unwrap();
        fs::write(&nested, b"now a file").unwrap();
        assert!(scan(selected, &Progress::default()).is_err());
        assert_eq!(fs::read(tmp.path().join("outside")).unwrap(), b"789");
    }

    #[test]
    fn nested_sizes_sort_and_scanning_changes_nothing() {
        let tmp = crate::util::tempdir().unwrap();
        fs::create_dir_all(tmp.path().join("nested/deeper")).unwrap();
        fs::write(tmp.path().join("nested/a"), b"1234").unwrap();
        fs::write(tmp.path().join("nested/deeper/b"), b"56").unwrap();
        fs::write(tmp.path().join("small"), b"7").unwrap();
        let report = scan(tmp.path(), &Progress::default()).unwrap();
        assert_eq!(
            report.totals,
            Totals {
                bytes: 7,
                files: 3,
                folders: 2,
                skipped_links: 0
            }
        );
        assert_eq!(report.rows.len(), 2);
        assert_eq!(report.rows[0].totals.bytes, 6);
        assert!(!report.incomplete());
        assert_eq!(fs::read(tmp.path().join("nested/a")).unwrap(), b"1234");
    }
    #[test]
    fn cancelled_scan_is_explicitly_partial_and_invalid_roots_fail() {
        let tmp = crate::util::tempdir().unwrap();
        let progress = Progress::default();
        progress
            .cancel
            .store(true, std::sync::atomic::Ordering::Relaxed);
        fs::write(tmp.path().join("file"), b"data").unwrap();
        let report = scan(tmp.path(), &progress).unwrap();
        assert!(report.cancelled && report.incomplete());
        assert!(report.rows.is_empty());
        assert!(scan(&tmp.path().join("file"), &Progress::default()).is_err());
        assert!(scan(&tmp.path().join("missing"), &Progress::default()).is_err());
    }
    #[test]
    fn top_list_is_bounded_without_losing_total_sizes() {
        let tmp = crate::util::tempdir().unwrap();
        for i in 0..ROW_LIMIT + 5 {
            fs::write(tmp.path().join(format!("{i:04}")), b"x").unwrap();
        }
        let report = scan(tmp.path(), &Progress::default()).unwrap();
        assert_eq!(report.rows.len(), ROW_LIMIT);
        assert_eq!(report.root_entries, (ROW_LIMIT + 5) as u64);
        assert_eq!(report.totals.bytes, (ROW_LIMIT + 5) as u64);
    }
    #[cfg(unix)]
    #[test]
    fn depth_limit_is_reported_instead_of_silently_omitting_descendants() {
        let tmp = crate::util::tempdir().unwrap();
        let mut path = tmp.path().to_path_buf();
        for _ in 0..DEPTH_LIMIT + 2 {
            path.push("d");
        }
        fs::create_dir_all(&path).unwrap();
        fs::write(path.join("beyond-limit"), b"not measured").unwrap();
        let report = scan(tmp.path(), &Progress::default()).unwrap();
        assert!(report.incomplete());
        assert!(report.errors > 0);
        assert!(report
            .error_details
            .iter()
            .any(|e| e.contains("depth limit")));
        assert!(report.rows[0].incomplete);
        assert_eq!(report.totals.files, 0);
    }

    #[cfg(unix)]
    #[test]
    fn links_are_skipped_and_not_counted_as_zero_sized_successes() {
        let tmp = crate::util::tempdir().unwrap();
        let outside = crate::util::tempdir().unwrap();
        fs::write(outside.path().join("secret"), b"outside data").unwrap();
        std::os::unix::fs::symlink(outside.path(), tmp.path().join("link")).unwrap();
        let report = scan(tmp.path(), &Progress::default()).unwrap();
        assert_eq!(report.totals.bytes, 0);
        assert_eq!(report.totals.skipped_links, 1);
        assert!(report.incomplete() && report.rows[0].incomplete);
        assert!(scan(&tmp.path().join("link"), &Progress::default()).is_err());
    }
}
