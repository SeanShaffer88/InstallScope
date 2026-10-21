//! Opt-in local persistence of the latest reviewed cleanup report.
//! Display-only records: importing a receipt never authorizes file operations.
use crate::cleaner::{CleanOutcome, CleanReport, CleanResult, ScanResult};
use anyhow::{bail, Context, Result};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

const VERSION: u32 = 1;
const MAX_BYTES: u64 = 16 * 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
struct Row {
    path: String,
    status: String,
    error: Option<String>,
}
crate::json_struct!(Row, { path, status, error });
#[derive(Clone, Debug, PartialEq, Eq)]
struct Report {
    files: u64,
    bytes: u64,
    recycle_bin: bool,
    error: Option<String>,
    rows: Vec<Row>,
}
crate::json_struct!(Report, { files, bytes, recycle_bin, error, rows });
#[derive(Clone, Debug, PartialEq, Eq)]
struct Receipt {
    version: u32,
    saved_at: i64,
    reports: Vec<Report>,
}
crate::json_struct!(Receipt, { version, saved_at, reports });

pub fn path(root: &Path) -> PathBuf {
    root.join("cleanup-report.json")
}

fn encode(reports: &[CleanReport]) -> Receipt {
    Receipt {
        version: VERSION,
        saved_at: crate::time::unix_now(),
        reports: reports
            .iter()
            .map(|r| Report {
                files: r.freed.files,
                bytes: r.freed.bytes,
                recycle_bin: r.recycle_bin,
                error: r.error.clone(),
                rows: r
                    .results
                    .iter()
                    .map(|r| {
                        let (status, error) = match &r.outcome {
                            CleanOutcome::Removed => ("removed", None),
                            CleanOutcome::Missing => ("missing", None),
                            CleanOutcome::ChangedSize => ("changed-size", None),
                            CleanOutcome::NotRegularFile => ("not-file", None),
                            CleanOutcome::Failed(error) => ("failed", Some(error.clone())),
                        };
                        Row {
                            path: r.path.to_string_lossy().into_owned(),
                            status: status.into(),
                            error,
                        }
                    })
                    .collect(),
            })
            .collect(),
    }
}

fn decode(receipt: Receipt) -> Result<(i64, Vec<CleanReport>)> {
    if receipt.version != VERSION {
        bail!("Unsupported cleanup report version {}; this build reads version {VERSION}. The saved file was left untouched.", receipt.version);
    }
    let mut total_bytes = 0u64;
    for report in &receipt.reports {
        total_bytes = total_bytes
            .checked_add(report.bytes)
            .context("Cleanup report byte totals overflow; file left untouched")?;
    }
    let mut reports = Vec::new();
    for r in receipt.reports {
        let mut results = Vec::new();
        for row in r.rows {
            let outcome = match (row.status.as_str(), row.error) {
                ("removed", None) => CleanOutcome::Removed,
                ("missing", None) => CleanOutcome::Missing,
                ("changed-size", None) => CleanOutcome::ChangedSize,
                ("not-file", None) => CleanOutcome::NotRegularFile,
                ("failed", Some(error)) => CleanOutcome::Failed(error),
                _ => bail!("Invalid cleanup report outcome; saved file left untouched."),
            };
            results.push(CleanResult {
                path: row.path.into(),
                outcome,
            });
        }
        reports.push(CleanReport {
            freed: ScanResult {
                files: r.files,
                bytes: r.bytes,
            },
            results,
            recycle_bin: r.recycle_bin,
            error: r.error,
        });
    }
    Ok((receipt.saved_at, reports))
}

/// Missing is normal; malformed, oversized, inaccessible and future files are errors.
pub fn load(root: &Path) -> Result<Option<(i64, Vec<CleanReport>)>> {
    let file = match std::fs::File::open(path(root)) {
        Ok(f) => f,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e).context("Open saved cleanup report"),
    };
    let mut bytes = Vec::new();
    file.take(MAX_BYTES + 1)
        .read_to_end(&mut bytes)
        .context("Read saved cleanup report")?;
    if bytes.len() as u64 > MAX_BYTES {
        bail!("Saved cleanup report exceeds the 16 MiB limit; file left untouched.");
    }
    let receipt: Receipt = crate::json::compat::from_slice(&bytes).context(
        "Saved cleanup report is damaged; copy it elsewhere for recovery or explicitly delete it",
    )?;
    decode(receipt).map(Some)
}

/// Only explicit user action saves reports. One receipt bounds retention.
/// Refuse to replace an unreadable/future receipt; explicit deletion is separate.
pub fn save(root: &Path, reports: &[CleanReport]) -> Result<()> {
    let _ = load(root)?;
    let bytes = crate::json::compat::to_vec(&encode(reports))?;
    if bytes.len() as u64 > MAX_BYTES {
        bail!("Cleanup report exceeds the 16 MiB save limit; current saved report left untouched.");
    }
    let tmp = root.join(format!("cleanup-report-{}.tmp", crate::util::new_id()));
    let result = (|| -> Result<()> {
        let mut file = std::fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&tmp)?;
        file.write_all(&bytes)?;
        file.sync_all()?;
        drop(file);
        std::fs::rename(&tmp, path(root)).context("Replace saved cleanup report")?;
        Ok(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result.context("Save cleanup report; the in-memory report remains available")
}

/// Deletes only the saved receipt, never any path mentioned inside it.
pub fn delete(root: &Path) -> Result<()> {
    match std::fs::remove_file(path(root)) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e).context("Delete saved cleanup report"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn round_trip_all_outcomes_replace_and_delete_without_touching_targets() {
        let tmp = crate::util::tempdir().unwrap();
        assert!(load(tmp.path()).unwrap().is_none());
        let target = tmp.path().join("keep.txt");
        std::fs::write(&target, "keep").unwrap();
        let report = CleanReport {
            freed: ScanResult { files: 1, bytes: 4 },
            results: [
                CleanOutcome::Removed,
                CleanOutcome::Missing,
                CleanOutcome::ChangedSize,
                CleanOutcome::NotRegularFile,
                CleanOutcome::Failed("Access denied".into()),
            ]
            .into_iter()
            .map(|outcome| CleanResult {
                path: target.clone(),
                outcome,
            })
            .collect(),
            ..Default::default()
        };
        save(tmp.path(), std::slice::from_ref(&report)).unwrap();
        assert_eq!(load(tmp.path()).unwrap().unwrap().1, vec![report]);
        let recycle = CleanReport {
            recycle_bin: true,
            error: Some("shell error".into()),
            ..Default::default()
        };
        save(tmp.path(), std::slice::from_ref(&recycle)).unwrap();
        assert_eq!(load(tmp.path()).unwrap().unwrap().1, vec![recycle]);
        delete(tmp.path()).unwrap();
        delete(tmp.path()).unwrap();
        assert!(load(tmp.path()).unwrap().is_none());
        assert_eq!(std::fs::read_to_string(target).unwrap(), "keep");
    }
    #[test]
    fn corrupt_and_future_reports_survive_load_and_save_attempts() {
        let tmp = crate::util::tempdir().unwrap();
        for bytes in [b"broken".to_vec(), br#"{"version":2,"saved_at":0,"reports":[]}"#.to_vec(), br#"{"version":1,"saved_at":0,"reports":[{"files":0,"bytes":0,"recycle_bin":false,"error":null,"rows":[{"path":"x","status":"unknown","error":null}]}]}"#.to_vec()] {
            std::fs::write(path(tmp.path()), &bytes).unwrap();
            assert!(load(tmp.path()).is_err());
            assert!(save(tmp.path(), &[]).is_err());
            assert_eq!(std::fs::read(path(tmp.path())).unwrap(), bytes);
        }
    }
    #[test]
    fn version_one_fixture_loads_and_overflow_totals_are_rejected() {
        let tmp = crate::util::tempdir().unwrap();
        let fixture = br#"{"version":1,"saved_at":1792560000,"reports":[{"files":1,"bytes":4,"recycle_bin":false,"error":null,"rows":[{"path":"C:\\Users\\Example\\cache.tmp","status":"removed","error":null}]}]}"#;
        std::fs::write(path(tmp.path()), fixture).unwrap();
        let (at, reports) = load(tmp.path()).unwrap().unwrap();
        assert_eq!(at, 1792560000);
        assert_eq!(reports[0].freed.bytes, 4);
        assert_eq!(reports[0].results[0].outcome, CleanOutcome::Removed);
        let mut invalid = encode(&[]);
        invalid.reports = vec![
            Report {
                files: 0,
                bytes: u64::MAX,
                recycle_bin: true,
                error: None,
                rows: vec![],
            },
            Report {
                files: 0,
                bytes: 1,
                recycle_bin: true,
                error: None,
                rows: vec![],
            },
        ];
        let bytes = crate::json::compat::to_vec(&invalid).unwrap();
        std::fs::write(path(tmp.path()), &bytes).unwrap();
        assert!(load(tmp.path()).is_err());
        assert!(save(tmp.path(), &[]).is_err());
        assert_eq!(std::fs::read(path(tmp.path())).unwrap(), bytes);
    }

    #[test]
    fn oversized_report_is_rejected_without_overwrite() {
        let tmp = crate::util::tempdir().unwrap();
        let file = std::fs::File::create(path(tmp.path())).unwrap();
        file.set_len(MAX_BYTES + 1).unwrap();
        assert!(load(tmp.path()).is_err());
        assert!(save(tmp.path(), &[]).is_err());
        assert_eq!(
            std::fs::metadata(path(tmp.path())).unwrap().len(),
            MAX_BYTES + 1
        );
    }
    #[test]
    fn failed_save_leaves_no_partial_receipt() {
        let tmp = crate::util::tempdir().unwrap();
        let missing = tmp.path().join("missing");
        assert!(save(&missing, &[]).is_err());
        assert!(!path(&missing).exists());
        assert!(!missing.exists());
    }
}
