//! Durable extension moves. A journal precedes every move; reconciliation never
//! moves or deletes extension data. Unknown/ambiguous states stop further actions.
use super::{extinv_dir, records_path, DisabledRecord, InstalledExtension, BROWSERS};
use crate::json::{self, ToJson};
use anyhow::{bail, Context, Result};
use std::{
    collections::HashSet,
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Component, Path, PathBuf},
};

const MAX_BYTES: u64 = 2 * 1024 * 1024;
const MAX_RECORDS: usize = 1000;
#[derive(Clone, Debug)]
struct Pending {
    version: u32,
    kind: String,
    record: DisabledRecord,
}
crate::json_struct!(Pending, { version, kind, record });
fn journal(root: &Path) -> PathBuf {
    extinv_dir(root).join("operation.json")
}

fn guard(path: &Path) -> Result<()> {
    if !path.is_absolute()
        || path
            .components()
            .any(|c| matches!(c, Component::ParentDir | Component::CurDir))
    {
        bail!(
            "Extension recovery requires absolute paths without dot components: {}",
            path.display()
        );
    }
    for part in path.ancestors() {
        match fs::symlink_metadata(part) {
            Ok(meta) => {
                let mut linked = meta.file_type().is_symlink();
                #[cfg(windows)]
                {
                    use std::os::windows::fs::MetadataExt;
                    linked |= meta.file_attributes() & 0x400 != 0;
                }
                if linked {
                    bail!(
                        "Linked/reparse paths are unsupported for extension moves: {}",
                        part.display()
                    );
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e).context("Inspect extension recovery path"),
        }
    }
    Ok(())
}
fn component(text: &str) -> bool {
    !text.is_empty()
        && text.len() <= 256
        && !matches!(text, "." | "..")
        && !text.ends_with(['.', ' '])
        && !text.chars().any(|c| c < ' ' || "<>:\"/\\|?*".contains(c))
}
fn validate(root: &Path, rec: &DisabledRecord) -> Result<()> {
    if !component(&rec.id)
        || rec.profile.is_empty()
        || rec.profile.len() > 256
        || rec.profile.chars().any(char::is_control)
        || rec.name.len() > 4096
        || rec.version.len() > 256
        || !BROWSERS
            .iter()
            .any(|b| b.name == rec.browser && b.family.label() == rec.family)
    {
        bail!("Invalid extension recovery record; files and receipts were left untouched.");
    }
    let profile = Path::new(&rec.profile_dir);
    let source = Path::new(&rec.source_path);
    let held = Path::new(&rec.held_path);
    guard(profile)?;
    guard(source)?;
    guard(held)?;
    let extension_root = profile.join(if rec.family == "chromium" {
        "Extensions"
    } else {
        "extensions"
    });
    if source != extension_root.join(&rec.id)
        && !(rec.family == "firefox" && source == extension_root.join(format!("{}.xpi", rec.id)))
    {
        bail!("Recovery source does not match this extension's exact profile layout.");
    }
    let holding = extinv_dir(root)
        .join("disabled")
        .join(crate::util::safe_file_name(&rec.browser))
        .join(crate::util::safe_file_name(&rec.profile));
    let relative = held
        .strip_prefix(&holding)
        .context("Held copy is outside the expected recovery folder")?;
    let parts: Vec<_> = relative.components().collect();
    if !(parts.len() == 1 || parts.len() == 2)
        || held.file_name() != source.file_name()
        || parts.iter().any(|c| !matches!(c, Component::Normal(_)))
    {
        bail!("Invalid held-copy layout in recovery record.");
    }
    if parts.len() == 2
        && !parts[0]
            .as_os_str()
            .to_string_lossy()
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || c == b'-')
    {
        bail!("Invalid recovery operation identifier.");
    }
    Ok(())
}
fn profile_ready(rec: &DisabledRecord) -> Result<()> {
    let profile = Path::new(&rec.profile_dir);
    let markers: &[&str] = if rec.family == "chromium" {
        &["Preferences", "Secure Preferences"]
    } else {
        &["prefs.js", "extensions.json"]
    };
    let mut found = false;
    for name in markers {
        let marker = profile.join(name);
        guard(&marker)?;
        found |= fs::symlink_metadata(marker).is_ok_and(|m| m.is_file());
    }
    if !found {
        bail!("The browser profile is missing or unrecognized. Keep the held copy and restore the profile before retrying.");
    }
    Ok(())
}
fn read_optional(path: &Path) -> Result<Option<Vec<u8>>> {
    guard(path)?;
    let metadata = match fs::symlink_metadata(path) {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e.into()),
    };
    if !metadata.is_file() || metadata.len() > MAX_BYTES {
        bail!(
            "Recovery metadata is not a regular file or exceeds 2 MiB: {}",
            path.display()
        );
    }
    let mut bytes = Vec::new();
    File::open(path)?
        .take(MAX_BYTES + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_BYTES {
        bail!("Recovery metadata exceeds 2 MiB; left untouched.");
    }
    Ok(Some(bytes))
}
pub fn load_records_checked(root: &Path) -> Result<Vec<DisabledRecord>> {
    let Some(bytes) = read_optional(&records_path(root))? else {
        let operation = pending(root)?;
        let ignored = operation.as_ref().map(|op| Path::new(&op.record.held_path));
        if held_data_exists(&extinv_dir(root).join("disabled"), ignored)? {
            bail!("Extension receipts are missing while held files remain. Preserve the holding folder and recover the receipts from backup; no files or records were changed.");
        }
        return Ok(Vec::new());
    };
    let records: Vec<DisabledRecord> = json::from_json_slice(&bytes).context("Extension receipts are unreadable or damaged; operations are blocked to preserve recovery data")?;
    if records.len() > MAX_RECORDS {
        bail!("Extension receipt limit exceeded; operations blocked.");
    }
    let mut keys = HashSet::new();
    let mut held = HashSet::new();
    for rec in &records {
        validate(root, rec)?;
        if !keys.insert((&rec.browser, &rec.profile_dir, &rec.id)) || !held.insert(&rec.held_path) {
            bail!("Duplicate extension recovery receipts; operations blocked.");
        }
    }
    Ok(records)
}
fn held_data_exists(root: &Path, ignored: Option<&Path>) -> Result<bool> {
    guard(root)?;
    if !root.exists() {
        return Ok(false);
    }
    let mut stack = vec![(root.to_path_buf(), 0)];
    let mut count = 0;
    while let Some((path, depth)) = stack.pop() {
        if depth > 8 {
            bail!("Unrecorded holding paths exceed the inspection depth; preserve them for manual recovery.");
        }
        for entry in fs::read_dir(path)? {
            let entry = entry?;
            count += 1;
            if count > 10_000 {
                bail!("Unrecorded holding paths exceed the inspection limit; preserve them for manual recovery.");
            }
            let path = entry.path();
            if ignored == Some(path.as_path()) {
                continue;
            }
            guard(&path)?;
            if entry.file_type()?.is_dir() {
                stack.push((path, depth + 1));
            } else {
                return Ok(true);
            }
        }
    }
    Ok(false)
}
fn valid_type(path: &Path, family: &str) -> Result<()> {
    let meta = fs::symlink_metadata(path)?;
    if !meta.is_dir() && (family == "chromium" || !meta.is_file()) {
        bail!("Extension data has an unsupported file type; no move is allowed.");
    }
    Ok(())
}
/// Display compatibility only. Mutations always use the fallible loader.
pub fn load_records(root: &Path) -> Vec<DisabledRecord> {
    load_records_checked(root).unwrap_or_default()
}
fn pending(root: &Path) -> Result<Option<Pending>> {
    let Some(bytes) = read_optional(&journal(root))? else {
        return Ok(None);
    };
    let pending: Pending = json::from_json_slice(&bytes)
        .context("Interrupted-operation journal is damaged; preserve it for recovery")?;
    if pending.version != 1 || !matches!(pending.kind.as_str(), "disable" | "restore") {
        bail!("Unsupported recovery journal; left untouched.");
    }
    validate(root, &pending.record)?;
    Ok(Some(pending))
}
pub fn recovery_problem(root: &Path) -> Option<String> {
    let checked = (|| -> Result<()> {
        load_records_checked(root)?;
        if pending(root)?.is_some() {
            bail!("An extension operation was interrupted. Reconcile its receipt before another move; extension files are not moved during reconciliation.");
        }
        Ok(())
    })();
    checked.err().map(|e| format!("{e:#}"))
}
fn lock(root: &Path) -> Result<File> {
    guard(&extinv_dir(root))?;
    fs::create_dir_all(extinv_dir(root))?;
    let path = extinv_dir(root).join("operation.lock");
    guard(&path)?;
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(path)?;
    file.try_lock()
        .context("Another extension operation is active; wait and retry")?;
    Ok(file)
}
fn persist(root: &Path, path: &Path, bytes: &[u8], replace: bool) -> Result<()> {
    guard(path)?;
    if bytes.len() as u64 > MAX_BYTES {
        bail!("Recovery metadata exceeds the 2 MiB limit.");
    }
    let temporary = extinv_dir(root).join(format!("receipt-{}.tmp", crate::util::new_id()));
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)?;
    let result = (|| -> Result<()> {
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        rename(&temporary, path, replace)
    })();
    let _ = fs::remove_file(&temporary);
    result
}
fn save_records(root: &Path, records: &[DisabledRecord]) -> Result<()> {
    if records.len() > MAX_RECORDS {
        anyhow::bail!("At most 1000 held extensions are supported.");
    }
    for rec in records {
        validate(root, rec)?;
    }
    let text = json::to_string_pretty(&json::Value::Array(
        records.iter().map(ToJson::to_json).collect(),
    ));
    persist(root, &records_path(root), text.as_bytes(), true)
}
fn prepare(root: &Path, kind: &str, record: DisabledRecord) -> Result<()> {
    let operation = Pending {
        version: 1,
        kind: kind.into(),
        record,
    };
    persist(
        root,
        &journal(root),
        json::to_json_string(&operation).as_bytes(),
        false,
    )
}
#[cfg(windows)]
fn rename(source: &Path, dest: &Path, replace: bool) -> Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{
        MoveFileExW, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH,
    };
    let src: Vec<_> = source.as_os_str().encode_wide().chain(Some(0)).collect();
    let dst: Vec<_> = dest.as_os_str().encode_wide().chain(Some(0)).collect();
    let flags = MOVEFILE_WRITE_THROUGH
        | if replace {
            MOVEFILE_REPLACE_EXISTING
        } else {
            0
        };
    if unsafe { MoveFileExW(src.as_ptr(), dst.as_ptr(), flags) } == 0 {
        return Err(std::io::Error::last_os_error()).context("Atomic extension move failed. Close the browser; source and holding area must be on the same volume. Nothing is copied/deleted as a fallback.");
    }
    Ok(())
}
#[cfg(not(windows))]
fn rename(source: &Path, dest: &Path, replace: bool) -> Result<()> {
    if !replace && fs::symlink_metadata(dest).is_ok() {
        bail!("Destination already exists; not replaced.");
    }
    fs::rename(source, dest)?;
    Ok(())
}
fn available(root: &Path) -> Result<Vec<DisabledRecord>> {
    let records = load_records_checked(root)?;
    if pending(root)?.is_some() {
        bail!("Reconcile the interrupted extension receipt before another move.");
    }
    Ok(records)
}
pub fn disable(root: &Path, ext: &InstalledExtension) -> Result<DisabledRecord> {
    let _lock = lock(root)?;
    let mut records = available(root)?;
    if ext.held.is_some()
        || records.iter().any(|r| {
            r.browser == ext.browser
                && r.profile_dir == ext.profile_dir.to_string_lossy()
                && r.id == ext.id
        })
    {
        bail!("A held copy already exists for this extension; resolve it before disabling another copy.");
    }
    if records.len() >= MAX_RECORDS {
        bail!("At most 1000 held extensions are supported; restore existing copies first.");
    }
    let filename = ext
        .source_path
        .file_name()
        .context("Extension source has no file name")?;
    let held = extinv_dir(root)
        .join("disabled")
        .join(crate::util::safe_file_name(&ext.browser))
        .join(crate::util::safe_file_name(&ext.profile))
        .join(crate::util::new_id())
        .join(filename);
    let rec = DisabledRecord {
        id: ext.id.clone(),
        browser: ext.browser.clone(),
        family: ext.family.label().into(),
        profile: ext.profile.clone(),
        profile_dir: ext.profile_dir.to_string_lossy().into_owned(),
        name: ext.name.clone(),
        version: ext.version.clone(),
        source_path: ext.source_path.to_string_lossy().into_owned(),
        held_path: held.to_string_lossy().into_owned(),
        moved_at: crate::util::now_string(),
    };
    validate(root, &rec)?;
    profile_ready(&rec)?;
    valid_type(&ext.source_path, &rec.family)?;
    fs::create_dir_all(held.parent().unwrap())?;
    prepare(root, "disable", rec.clone())?;
    finish_disable(root, &rec, &mut records)?;
    Ok(rec)
}
fn finish_disable(
    root: &Path,
    rec: &DisabledRecord,
    records: &mut Vec<DisabledRecord>,
) -> Result<()> {
    rename(
        Path::new(&rec.source_path),
        Path::new(&rec.held_path),
        false,
    )
    .context("Receipt journal retained; use Reconcile interrupted receipt before retrying")?;
    records.push(rec.clone());
    save_records(root, records)
        .context("Files are held; recovery journal retained. Reconcile the interrupted receipt")?;
    fs::remove_file(journal(root))
        .context("Move recorded, but journal cleanup failed; reconcile the receipt")?;
    Ok(())
}
pub fn restore(root: &Path, record: &DisabledRecord) -> Result<()> {
    let _lock = lock(root)?;
    let mut records = available(root)?;
    validate(root, record)?;
    profile_ready(record)?;
    if !records.contains(record) {
        bail!(
            "This exact extension receipt is not in the current ledger. Rescan before restoring."
        );
    }
    let source = Path::new(&record.held_path);
    let dest = Path::new(&record.source_path);
    fs::symlink_metadata(source).context("Held copy is unavailable; keep receipts for recovery")?;
    valid_type(source, &record.family)?;
    if fs::symlink_metadata(dest).is_ok() {
        bail!("The browser destination already exists; both copies are preserved. Resolve the conflict outside SetupScope and rescan.");
    }
    fs::create_dir_all(dest.parent().unwrap())?;
    prepare(root, "restore", record.clone())?;
    rename(source, dest, false).context("Recovery journal retained; reconcile before retrying")?;
    records.retain(|r| r != record);
    save_records(root, &records)
        .context("Files restored; recovery journal retained. Reconcile the receipt")?;
    fs::remove_file(journal(root))?;
    Ok(())
}
/// Resolve only receipt metadata from an unambiguous interrupted move.
pub fn reconcile(root: &Path) -> Result<()> {
    let _lock = lock(root)?;
    let mut records = load_records_checked(root)?;
    let Some(operation) = pending(root)? else {
        return Ok(());
    };
    let rec = operation.record;
    let present = |path: &str| -> Result<bool> {
        match fs::symlink_metadata(path) {
            Ok(_) => Ok(true),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
            Err(e) => Err(e.into()),
        }
    };
    let held = present(&rec.held_path)?;
    let original = present(&rec.source_path)?;
    if held == original {
        bail!("Both copies exist, or neither is available. No data or receipts changed; preserve both paths and the journal for manual recovery.");
    }
    valid_type(
        Path::new(if held {
            &rec.held_path
        } else {
            &rec.source_path
        }),
        &rec.family,
    )?;
    records.retain(|r| r != &rec);
    if held {
        if records
            .iter()
            .any(|r| r.browser == rec.browser && r.profile_dir == rec.profile_dir && r.id == rec.id)
        {
            bail!("Conflicting receipt; no recovery metadata changed.");
        }
        records.push(rec);
    }
    save_records(root, &records)?;
    fs::remove_file(journal(root))?;
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::{extinv::Family, util::TestDir};
    fn fixture(td: &TestDir) -> (PathBuf, InstalledExtension) {
        let root = td.path().join("data");
        let profile = td.path().join("profile");
        let source = profile.join("Extensions").join("testid");
        fs::create_dir_all(&source).unwrap();
        fs::write(profile.join("Preferences"), "{}").unwrap();
        fs::write(source.join("payload"), "original").unwrap();
        (
            root,
            InstalledExtension {
                browser: "Chrome".into(),
                family: Family::Chromium,
                profile: "Default".into(),
                profile_dir: profile,
                id: "testid".into(),
                name: "Test".into(),
                version: "1".into(),
                enabled: Some(true),
                source_path: source,
                held: None,
            },
        )
    }
    #[test]
    fn corrupt_receipts_never_overwrite_or_move() {
        let td = crate::util::tempdir().unwrap();
        let (root, ext) = fixture(&td);
        fs::create_dir_all(extinv_dir(&root)).unwrap();
        fs::write(records_path(&root), "broken").unwrap();
        assert!(disable(&root, &ext).is_err());
        assert!(ext.source_path.join("payload").exists());
        assert_eq!(fs::read_to_string(records_path(&root)).unwrap(), "broken");
    }
    #[test]
    fn missing_ledger_with_held_data_stops_moves() {
        let td = crate::util::tempdir().unwrap();
        let (root, ext) = fixture(&td);
        let rec = disable(&root, &ext).unwrap();
        fs::remove_file(records_path(&root)).unwrap();
        assert!(load_records_checked(&root).is_err());
        assert!(restore(&root, &rec).is_err());
        assert!(Path::new(&rec.held_path).join("payload").exists());
    }
    #[test]
    fn restore_conflict_preserves_both_copies() {
        let td = crate::util::tempdir().unwrap();
        let (root, ext) = fixture(&td);
        let rec = disable(&root, &ext).unwrap();
        fs::create_dir_all(&ext.source_path).unwrap();
        fs::write(ext.source_path.join("payload"), "new").unwrap();
        assert!(restore(&root, &rec).is_err());
        assert_eq!(
            fs::read_to_string(ext.source_path.join("payload")).unwrap(),
            "new"
        );
        assert_eq!(
            fs::read_to_string(Path::new(&rec.held_path).join("payload")).unwrap(),
            "original"
        );
        assert_eq!(load_records_checked(&root).unwrap().len(), 1);
    }
    #[test]
    fn interrupted_moves_reconcile_metadata_only() {
        let td = crate::util::tempdir().unwrap();
        let (root, ext) = fixture(&td);
        let rec = disable(&root, &ext).unwrap();
        prepare(&root, "restore", rec.clone()).unwrap();
        rename(Path::new(&rec.held_path), &ext.source_path, false).unwrap();
        assert!(recovery_problem(&root).is_some());
        reconcile(&root).unwrap();
        assert!(load_records_checked(&root).unwrap().is_empty());
        assert!(ext.source_path.join("payload").exists());
        prepare(&root, "disable", rec.clone()).unwrap();
        rename(&ext.source_path, Path::new(&rec.held_path), false).unwrap();
        reconcile(&root).unwrap();
        assert_eq!(load_records_checked(&root).unwrap(), vec![rec]);
    }
    #[test]
    fn ambiguous_interruption_keeps_journal() {
        let td = crate::util::tempdir().unwrap();
        let (root, ext) = fixture(&td);
        let rec = disable(&root, &ext).unwrap();
        prepare(&root, "restore", rec.clone()).unwrap();
        fs::create_dir_all(&ext.source_path).unwrap();
        assert!(reconcile(&root).is_err());
        assert!(journal(&root).exists());
        assert_eq!(load_records_checked(&root).unwrap(), vec![rec]);
    }
    #[test]
    fn forged_receipt_and_missing_profile_are_refused() {
        let td = crate::util::tempdir().unwrap();
        let (root, ext) = fixture(&td);
        let rec = disable(&root, &ext).unwrap();
        let mut forged = rec.clone();
        forged.source_path = ext.profile_dir.to_string_lossy().into();
        assert!(restore(&root, &forged).is_err());
        fs::remove_file(ext.profile_dir.join("Preferences")).unwrap();
        assert!(restore(&root, &rec).is_err());
        assert!(Path::new(&rec.held_path).exists());
    }
    #[test]
    fn operation_lock_is_exclusive_and_reusable() {
        let td = crate::util::tempdir().unwrap();
        let (root, _) = fixture(&td);
        let first = lock(&root).unwrap();
        assert!(lock(&root).is_err());
        drop(first);
        assert!(lock(&root).is_ok());
    }
    #[test]
    fn future_journal_is_preserved() {
        let td = crate::util::tempdir().unwrap();
        let (root, ext) = fixture(&td);
        let rec = disable(&root, &ext).unwrap();
        prepare(&root, "restore", rec).unwrap();
        let bytes = fs::read_to_string(journal(&root))
            .unwrap()
            .replace("\"version\":1", "\"version\":99");
        fs::write(journal(&root), &bytes).unwrap();
        assert!(reconcile(&root).is_err());
        assert_eq!(fs::read_to_string(journal(&root)).unwrap(), bytes);
    }
    #[test]
    fn interrupted_first_disable_recovers_missing_ledger() {
        let td = crate::util::tempdir().unwrap();
        let (root, ext) = fixture(&td);
        let rec = disable(&root, &ext).unwrap();
        prepare(&root, "disable", rec.clone()).unwrap();
        fs::remove_file(records_path(&root)).unwrap();
        reconcile(&root).unwrap();
        assert_eq!(load_records_checked(&root).unwrap(), vec![rec]);
    }
    #[test]
    fn cancelled_move_clears_only_pending_receipt() {
        let td = crate::util::tempdir().unwrap();
        let (root, ext) = fixture(&td);
        let rec = disable(&root, &ext).unwrap();
        restore(&root, &rec).unwrap();
        prepare(&root, "disable", rec).unwrap();
        reconcile(&root).unwrap();
        assert!(ext.source_path.join("payload").exists());
        assert!(load_records_checked(&root).unwrap().is_empty());
    }
    #[test]
    fn duplicate_and_oversized_receipts_fail_closed() {
        let td = crate::util::tempdir().unwrap();
        let (root, ext) = fixture(&td);
        let rec = disable(&root, &ext).unwrap();
        save_records(&root, &[rec.clone(), rec.clone()]).unwrap();
        assert!(load_records_checked(&root).is_err());
        let bytes = vec![b' '; MAX_BYTES as usize + 1];
        fs::write(records_path(&root), &bytes).unwrap();
        assert!(restore(&root, &rec).is_err());
        assert_eq!(
            fs::metadata(records_path(&root)).unwrap().len(),
            MAX_BYTES + 1
        );
    }
    #[test]
    fn firefox_archive_round_trip_preserves_bytes() {
        let td = crate::util::tempdir().unwrap();
        let (root, mut ext) = fixture(&td);
        ext.browser = "Firefox".into();
        ext.family = Family::Firefox;
        ext.id = "test@example.org".into();
        ext.source_path = ext
            .profile_dir
            .join("extensions")
            .join("test@example.org.xpi");
        fs::create_dir_all(ext.source_path.parent().unwrap()).unwrap();
        fs::write(ext.profile_dir.join("prefs.js"), "// fixture").unwrap();
        fs::write(&ext.source_path, b"archive fixture").unwrap();
        let rec = disable(&root, &ext).unwrap();
        restore(&root, &rec).unwrap();
        assert_eq!(fs::read(&ext.source_path).unwrap(), b"archive fixture");
    }
}
