//! Backups: everything InstallScope removes or changes is first copied here, so any
//! uninstall, cleanup or startup change can be rolled back.

use crate::model::{KeyData, RegValue};
use crate::platform::{self, Registry};
use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq)]
pub enum BackupItem {
    /// A file copied into the backup before deletion.
    File {
        original: String,
        stored: String,
        size: u64,
    },
    /// A folder that existed (recreated on restore).
    Dir { original: String },
    /// A registry subtree captured before deletion.
    RegTree { keys: Vec<KeyData> },
    /// A value's previous state (`None` = it did not exist).
    RegValue {
        key: String,
        name: String,
        previous: Option<RegValue>,
    },
    /// A key that did not exist before the operation created it.
    KeyAbsent { path: String },
    /// A file that did not exist before the operation created it
    /// (restoring deletes it again).
    FileAbsent { path: String },
    /// A folder the operation created (restoring removes it if empty).
    DirAbsent { path: String },
}
crate::json_enum!(BackupItem { File { original, stored, size }, Dir { original }, RegTree { keys }, RegValue { key, name, previous }, KeyAbsent { path }, FileAbsent { path }, DirAbsent { path } });

#[derive(Clone, Debug, Default)]
pub struct Backup {
    pub id: String,
    pub title: String,
    pub created: String,
    pub items: Vec<BackupItem>,
    pub restored: bool,
    /// False while the operation runs. A backup left unfinished means the
    /// operation was interrupted (a crash or power loss); everything it had
    /// changed so far is recorded here, so it can be undone.
    pub complete: bool,
    /// The process that made it, to tell a running operation from one that
    /// was interrupted.
    pub pid: u32,
}
crate::json_struct!(Backup, { id, title, created, items, restored [default], complete [or = finished_by_default], pid [default] });

fn finished_by_default() -> bool {
    true
}

impl Backup {
    pub fn bytes(&self) -> u64 {
        self.items
            .iter()
            .map(|i| {
                if let BackupItem::File { size, .. } = i {
                    *size
                } else {
                    0
                }
            })
            .sum()
    }

    pub fn summary(&self) -> String {
        let (mut files, mut keys, mut values) = (0, 0, 0);
        for i in &self.items {
            match i {
                BackupItem::File { .. } => files += 1,
                BackupItem::RegTree { keys: k } => keys += k.len(),
                BackupItem::RegValue { .. } => values += 1,
                _ => {}
            }
        }
        format!(
            "{files} files ({}), {keys} keys, {values} values",
            crate::model::human_size(self.bytes())
        )
    }
}

/// Result of one step of an operation, shown to the user afterwards.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ActionResult {
    pub description: String,
    pub ok: bool,
    pub message: String,
}
crate::json_struct!(ActionResult, { description, ok, message });

impl ActionResult {
    pub fn ok(desc: impl Into<String>, msg: impl Into<String>) -> Self {
        ActionResult {
            description: desc.into(),
            ok: true,
            message: msg.into(),
        }
    }
    pub fn err(desc: impl Into<String>, msg: impl Into<String>) -> Self {
        ActionResult {
            description: desc.into(),
            ok: false,
            message: msg.into(),
        }
    }
}

/// Collects backup items while an operation runs.
pub struct BackupWriter {
    dir: PathBuf,
    backup: Backup,
    counter: usize,
    /// Set by `finish` or `discard`; otherwise dropping the writer marks
    /// the backup complete (the operation ended, even if with errors).
    done: bool,
}

impl BackupWriter {
    pub fn new(backups_root: &Path, title: &str) -> Result<Self> {
        let id = crate::util::new_id();
        let dir = backups_root.join(&id);
        std::fs::create_dir_all(dir.join("files"))
            .with_context(|| format!("create {}", dir.display()))?;
        let w = BackupWriter {
            dir,
            backup: Backup {
                id,
                title: title.to_string(),
                created: crate::util::now_string(),
                complete: false,
                pid: std::process::id(),
                ..Default::default()
            },
            counter: 0,
            done: false,
        };
        w.save()?;
        Ok(w)
    }

    pub fn id(&self) -> &str {
        &self.backup.id
    }

    /// Write the manifest before each change is made (write-ahead), atomically
    /// and flushed to disk, so a crash at any moment leaves a readable record
    /// of everything changed so far.
    fn save(&self) -> Result<()> {
        write_manifest(&self.dir.join("manifest.json"), &self.backup)
    }

    pub fn add_file(&mut self, path: &Path) -> Result<()> {
        self.copy_file(path)?;
        self.save()
    }

    fn copy_file(&mut self, path: &Path) -> Result<()> {
        let meta = std::fs::metadata(path)?;
        self.counter += 1;
        let ext = path
            .extension()
            .map(|e| format!(".{}", e.to_string_lossy()))
            .unwrap_or_default();
        let stored = format!("{:06}{ext}", self.counter);
        std::fs::copy(path, self.dir.join("files").join(&stored))
            .with_context(|| format!("back up {}", path.display()))?;
        self.backup.items.push(BackupItem::File {
            original: path.to_string_lossy().to_string(),
            stored,
            size: meta.len(),
        });
        Ok(())
    }

    /// Back up a whole folder: the folder entries and every file inside.
    pub fn add_dir_tree(&mut self, path: &Path) -> Result<()> {
        for e in crate::fswalk::WalkDir::new(path)
            .follow_links(false)
            .into_iter()
            .flatten()
        {
            if e.file_type().is_dir() {
                self.backup.items.push(BackupItem::Dir {
                    original: e.path().to_string_lossy().to_string(),
                });
            } else if e.file_type().is_file() {
                // One manifest write for the whole folder: nothing in it is
                // deleted until this returns.
                self.copy_file(e.path())?;
            }
        }
        self.save()
    }

    pub fn add_dir(&mut self, path: &Path) -> Result<()> {
        self.backup.items.push(BackupItem::Dir {
            original: path.to_string_lossy().to_string(),
        });
        self.save()
    }

    pub fn add_tree(&mut self, reg: &dyn Registry, path: &str) -> Result<()> {
        let keys = platform::read_tree(reg, path);
        if !keys.is_empty() {
            self.backup.items.push(BackupItem::RegTree { keys });
        }
        self.save()
    }

    pub fn add_value(&mut self, reg: &dyn Registry, key: &str, name: &str) -> Result<()> {
        let previous = reg.read_key(key).and_then(|k| k.get(name).cloned());
        self.backup.items.push(BackupItem::RegValue {
            key: key.into(),
            name: name.into(),
            previous,
        });
        self.save()
    }

    /// Record that the operation is about to create this file.
    pub fn add_file_absent(&mut self, path: &str) -> Result<()> {
        self.backup
            .items
            .push(BackupItem::FileAbsent { path: path.into() });
        self.save()
    }

    /// Record that the operation is about to create this folder.
    pub fn add_dir_absent(&mut self, path: &str) -> Result<()> {
        self.backup
            .items
            .push(BackupItem::DirAbsent { path: path.into() });
        self.save()
    }

    pub fn add_key_absent(&mut self, path: &str) -> Result<()> {
        self.backup
            .items
            .push(BackupItem::KeyAbsent { path: path.into() });
        self.save()
    }

    pub fn is_empty(&self) -> bool {
        self.backup.items.is_empty()
    }

    pub fn finish(mut self) -> Result<Backup> {
        self.done = true;
        self.backup.complete = true;
        self.save()?;
        // Also export the registry part as a standard .reg file, so it can be
        // restored by double-clicking even without InstallScope.
        let reg = backup_to_reg(&self.backup);
        if !reg.is_empty() {
            let mut bytes = vec![0xFF, 0xFE];
            reg.encode_utf16()
                .for_each(|u| bytes.extend_from_slice(&u.to_le_bytes()));
            std::fs::write(
                self.dir.join(format!("registry-{}.reg", self.backup.id)),
                bytes,
            )?;
        }
        Ok(self.backup.clone())
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// Remove an empty backup (nothing needed saving).
    pub fn discard(mut self) {
        self.done = true;
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

impl Drop for BackupWriter {
    fn drop(&mut self) {
        if !self.done && !self.backup.complete {
            self.backup.complete = true;
            let _ = self.save();
        }
    }
}

/// Write a manifest atomically: to a temporary file, flushed, then renamed
/// over the old one.
fn write_manifest(path: &Path, b: &Backup) -> Result<()> {
    use std::io::Write;
    let json = crate::json::compat::to_string_pretty(b)?;
    let tmp = path.with_extension("json.tmp");
    {
        let mut f =
            std::fs::File::create(&tmp).with_context(|| format!("write {}", tmp.display()))?;
        f.write_all(json.as_bytes())?;
        f.sync_all()?;
    }
    std::fs::rename(&tmp, path).with_context(|| format!("write {}", path.display()))?;
    Ok(())
}

/// Operations that were interrupted before they finished (a crash, power
/// loss or forced shutdown) and haven't been undone or dismissed.
pub fn interrupted(list: &[Backup]) -> Vec<&Backup> {
    list.iter()
        .filter(|b| {
            !b.complete
                && !b.restored
                && b.pid != std::process::id()
                && !platform::process_alive(b.pid)
        })
        .collect()
}

/// Keep what an interrupted operation did and stop asking about it.
pub fn mark_complete(root: &Path, id: &str) -> Result<()> {
    let manifest = root.join(id).join("manifest.json");
    let mut b: Backup = crate::json::compat::from_str(&std::fs::read_to_string(&manifest)?)?;
    b.complete = true;
    write_manifest(&manifest, &b)
}

/// The registry contents of a backup as a `.reg` file that puts everything
/// back the way it was before the operation. Empty if no registry changes.
pub fn backup_to_reg(b: &Backup) -> String {
    use crate::report::{long_path, reg_data, reg_name};
    let mut body = String::new();
    // Restore in reverse order of the operation, like `restore_backup`.
    for item in b.items.iter().rev() {
        match item {
            BackupItem::RegTree { keys } => {
                for k in keys {
                    body.push_str(&format!("[{}]\r\n", long_path(&k.path)));
                    for v in k.values.values() {
                        body.push_str(&format!("{}={}\r\n", reg_name(&v.name), reg_data(v)));
                    }
                    body.push_str("\r\n");
                }
            }
            BackupItem::RegValue {
                key,
                name,
                previous,
            } => {
                body.push_str(&format!("[{}]\r\n", long_path(key)));
                match previous {
                    Some(v) => {
                        body.push_str(&format!("{}={}\r\n\r\n", reg_name(name), reg_data(v)))
                    }
                    None => body.push_str(&format!("{}=-\r\n\r\n", reg_name(name))),
                }
            }
            BackupItem::KeyAbsent { path } => {
                body.push_str(&format!("[-{}]\r\n\r\n", long_path(path)))
            }
            _ => {}
        }
    }
    if body.is_empty() {
        return body;
    }
    format!(
        "Windows Registry Editor Version 5.00\r\n\r\n; InstallScope backup {} — {}\r\n; Created {}. Double-click (as Administrator) to restore these registry entries.\r\n\r\n{body}",
        b.id, b.title, b.created
    )
}

pub fn list_backups(root: &Path) -> Vec<Backup> {
    let mut out: Vec<Backup> = std::fs::read_dir(root)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| std::fs::read_to_string(e.path().join("manifest.json")).ok())
        .filter_map(|s| crate::json::compat::from_str(&s).ok())
        .collect();
    out.sort_by(|a, b| b.created.cmp(&a.created));
    out
}

pub fn delete_backup(root: &Path, id: &str) -> Result<()> {
    let dir = root.join(id);
    if dir.join("manifest.json").exists() {
        std::fs::remove_dir_all(dir)?;
    }
    Ok(())
}

/// Take a safety backup of the current state of everything restoring
/// backup `id` would change, as an ordinary backup on the same page, so
/// the restore itself can be undone. The capture records the inverse of
/// each step [`restore_backup`] will take, in the order it takes them
/// (folders first, then the remaining items newest-first): a file's
/// current contents, a registry tree or value as it stands now, or the
/// fact that a path the restore will (re)create is absent now.
///
/// Returns `Ok(None)` when nothing the restore touches exists or would
/// be created, so there is nothing to keep. An `Err` means the safety
/// backup could not be taken (any partial capture is removed); the
/// caller must not go ahead with the restore in that case.
pub fn backup_before_restore(root: &Path, id: &str, reg: &dyn Registry) -> Result<Option<Backup>> {
    let manifest = root.join(id).join("manifest.json");
    let source: Backup = crate::json::compat::from_str(&std::fs::read_to_string(&manifest)?)?;
    let mut w = BackupWriter::new(root, &format!("Before restoring: {}", source.title))?;
    if let Err(e) = capture_current_state(&mut w, &source, reg) {
        // Leave no half-taken safety backup behind: marked unfinished,
        // it would surface later as an interrupted operation.
        w.discard();
        return Err(e);
    }
    if w.is_empty() {
        w.discard();
        return Ok(None);
    }
    Ok(Some(w.finish()?))
}

/// Record, into `w`, the current state of everything `restore_backup`
/// would change when replaying `source` — the inverse of each item.
fn capture_current_state(w: &mut BackupWriter, source: &Backup, reg: &dyn Registry) -> Result<()> {
    // Folders first, as in restore_backup: a folder the restore will
    // create is absent now, so record that it must go again.
    for item in &source.items {
        if let BackupItem::Dir { original } = item {
            if !Path::new(original).exists() {
                w.add_dir_absent(original)?;
            }
        }
    }
    // Then the remaining items, newest first, as restore replays them.
    for item in source.items.iter().rev() {
        match item {
            BackupItem::Dir { .. } => {}
            BackupItem::File { original, .. } => {
                // The restore overwrites this path with its stored copy:
                // keep what is there now, or that it is not there.
                let p = Path::new(original);
                if p.exists() {
                    w.add_file(p)?;
                } else {
                    w.add_file_absent(original)?;
                }
            }
            BackupItem::RegTree { keys } => {
                // The restore writes this tree back (merge: keys and
                // values are created or set, nothing is deleted). The
                // first key is the tree's root (read_tree is parents
                // first). Keep the tree as it stands; keys it does not
                // have yet are recorded absent, so the undo removes the
                // ones the restore recreates.
                if let Some(root_key) = keys.first() {
                    if reg.exists(&root_key.path) {
                        w.add_tree(reg, &root_key.path)?;
                        for k in keys {
                            if !reg.exists(&k.path) {
                                w.add_key_absent(&k.path)?;
                            }
                        }
                    } else {
                        w.add_key_absent(&root_key.path)?;
                    }
                }
            }
            BackupItem::RegValue { key, name, .. } => {
                // Captures the value as it stands (or that it is absent):
                // exactly the state undoing the restore must bring back.
                w.add_value(reg, key, name)?;
            }
            BackupItem::FileAbsent { path } => {
                // The restore deletes this file if it is there: keep its
                // current contents.
                let p = Path::new(path);
                if p.exists() {
                    w.add_file(p)?;
                }
            }
            BackupItem::DirAbsent { path } => {
                // The restore removes this folder (its recorded files go
                // through their own FileAbsent items): record that the
                // folder itself exists now, so the undo recreates it.
                if Path::new(path).exists() {
                    w.add_dir(Path::new(path))?;
                }
            }
            BackupItem::KeyAbsent { path } => {
                // The restore deletes this key if it is there: keep its
                // current contents.
                if reg.exists(path) {
                    w.add_tree(reg, path)?;
                }
            }
        }
    }
    Ok(())
}

/// Undo an operation by replaying its backup in reverse order.
pub fn restore_backup(root: &Path, id: &str, reg: &dyn Registry) -> Result<Vec<ActionResult>> {
    let dir = root.join(id);
    let manifest = dir.join("manifest.json");
    let mut backup: Backup = crate::json::compat::from_str(&std::fs::read_to_string(&manifest)?)?;
    let mut results = Vec::new();

    // Folders first so files have somewhere to go.
    for item in &backup.items {
        if let BackupItem::Dir { original } = item {
            let r = std::fs::create_dir_all(original);
            results.push(match r {
                Ok(()) => ActionResult::ok(format!("Folder {original}"), "recreated"),
                Err(e) => ActionResult::err(format!("Folder {original}"), e.to_string()),
            });
        }
    }
    for item in backup.items.iter().rev() {
        let r = match item {
            BackupItem::Dir { .. } => continue,
            BackupItem::File {
                original, stored, ..
            } => {
                let desc = format!("File {original}");
                let dest = Path::new(original);
                let res = dest
                    .parent()
                    .map(std::fs::create_dir_all)
                    .unwrap_or(Ok(()))
                    .and_then(|_| std::fs::copy(dir.join("files").join(stored), dest).map(|_| ()));
                match res {
                    Ok(()) => ActionResult::ok(desc, "restored"),
                    Err(e) => ActionResult::err(desc, e.to_string()),
                }
            }
            BackupItem::RegTree { keys } => {
                let desc = format!(
                    "Key {}",
                    keys.first().map(|k| k.path.as_str()).unwrap_or("?")
                );
                match platform::write_tree(reg, keys) {
                    Ok(()) => ActionResult::ok(desc, format!("restored {} keys", keys.len())),
                    Err(e) => ActionResult::err(desc, e.to_string()),
                }
            }
            BackupItem::RegValue {
                key,
                name,
                previous,
            } => {
                let desc = format!(
                    "Value {key}\\{}",
                    if name.is_empty() { "(Default)" } else { name }
                );
                let res = match previous {
                    Some(v) => reg.set_value(key, v),
                    None => match reg.read_key(key).and_then(|k| k.get(name).cloned()) {
                        Some(_) => reg.delete_value(key, name),
                        None => Ok(()),
                    },
                };
                match res {
                    Ok(()) => ActionResult::ok(desc, "restored"),
                    Err(e) => ActionResult::err(desc, e.to_string()),
                }
            }
            BackupItem::FileAbsent { path } => {
                let desc = format!("File {path}");
                if Path::new(path).exists() {
                    match platform::delete_file(Path::new(path)) {
                        Ok(()) => ActionResult::ok(desc, "removed again"),
                        Err(e) => ActionResult::err(desc, e.to_string()),
                    }
                } else {
                    ActionResult::ok(desc, "already absent")
                }
            }
            BackupItem::DirAbsent { path } => {
                // Items are undone newest first, so its files are gone by now.
                let desc = format!("Folder {path}");
                match std::fs::remove_dir(path) {
                    Ok(()) => ActionResult::ok(desc, "removed again"),
                    Err(_) if !Path::new(path).exists() => ActionResult::ok(desc, "already absent"),
                    Err(e) => ActionResult::err(desc, e.to_string()),
                }
            }
            BackupItem::KeyAbsent { path } => {
                let desc = format!("Key {path}");
                if reg.exists(path) {
                    match reg.delete_tree(path) {
                        Ok(()) => ActionResult::ok(desc, "removed again"),
                        Err(e) => ActionResult::err(desc, e.to_string()),
                    }
                } else {
                    ActionResult::ok(desc, "already absent")
                }
            }
        };
        results.push(r);
    }
    backup.restored = true;
    backup.complete = true;
    write_manifest(&manifest, &backup)?;
    Ok(results)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::platform::MemRegistry;

    #[test]
    fn backup_and_restore_roundtrip() {
        let tmp = crate::util::tempdir().unwrap();
        let broot = tmp.path().join("backups");
        let work = tmp.path().join("work");
        std::fs::create_dir_all(work.join("sub")).unwrap();
        std::fs::write(work.join("sub").join("a.txt"), "hello").unwrap();

        let reg = MemRegistry::new();
        reg.put(KeyData::new("HKCU\\Software\\X\\Y").with(RegValue::string("v", "1")));
        reg.put(KeyData::new("HKCU\\Software\\Z").with(RegValue::dword("n", 5)));

        let mut w = BackupWriter::new(&broot, "test").unwrap();
        w.add_dir_tree(&work).unwrap();
        w.add_tree(&reg, "HKCU\\Software\\X").unwrap();
        w.add_value(&reg, "HKCU\\Software\\Z", "n").unwrap();
        w.add_value(&reg, "HKCU\\Software\\Z", "new").unwrap();
        let b = w.finish().unwrap();
        let reg_file =
            std::fs::read(broot.join(&b.id).join(format!("registry-{}.reg", b.id))).unwrap();
        let text = crate::model::decode_utf16z(&reg_file[2..]);
        assert!(text.starts_with("Windows Registry Editor Version 5.00"));
        assert!(text.contains("[HKEY_CURRENT_USER\\Software\\X\\Y]"));
        assert!(text.contains("\"n\"=dword:00000005"));
        assert!(text.contains("\"new\"=-"));

        // Destroy things.
        std::fs::remove_dir_all(&work).unwrap();
        reg.delete_tree("HKCU\\Software\\X").unwrap();
        reg.set_value("HKCU\\Software\\Z", &RegValue::dword("n", 99))
            .unwrap();
        reg.set_value("HKCU\\Software\\Z", &RegValue::dword("new", 1))
            .unwrap();

        let res = restore_backup(&broot, &b.id, &reg).unwrap();
        assert!(res.iter().all(|r| r.ok), "{res:?}");
        assert_eq!(
            std::fs::read_to_string(work.join("sub").join("a.txt")).unwrap(),
            "hello"
        );
        assert_eq!(
            reg.read_key("HKCU\\Software\\X\\Y")
                .unwrap()
                .get_string("v")
                .unwrap(),
            "1"
        );
        let z = reg.read_key("HKCU\\Software\\Z").unwrap();
        assert_eq!(z.get_u32("n"), Some(5));
        assert!(z.get("new").is_none());
        assert!(list_backups(&broot)[0].restored);
    }

    #[test]
    fn an_interrupted_removal_can_be_found_and_undone() {
        let tmp = crate::util::tempdir().unwrap();
        let broot = tmp.path().join("backups");
        let work = tmp.path().join("work");
        std::fs::create_dir_all(&work).unwrap();
        for n in ["a.txt", "b.txt"] {
            std::fs::write(work.join(n), n).unwrap();
        }
        let reg = MemRegistry::new();
        reg.put(KeyData::new("HKCU\\Software\\Gone").with(RegValue::string("v", "1")));

        // A removal that is cut off half way: the first file and the key
        // are backed up and deleted, then the process dies (no finish, no
        // drop).
        let mut w = BackupWriter::new(&broot, "Removing Gone").unwrap();
        w.add_file(&work.join("a.txt")).unwrap();
        std::fs::remove_file(work.join("a.txt")).unwrap();
        w.add_tree(&reg, "HKCU\\Software\\Gone").unwrap();
        reg.delete_tree("HKCU\\Software\\Gone").unwrap();
        let id = w.id().to_string();
        std::mem::forget(w);

        // While the process that made it runs, it isn't "interrupted".
        let list = list_backups(&broot);
        assert!(!list[0].complete);
        assert!(interrupted(&list).is_empty());

        // As seen after a restart (the process is gone).
        let manifest = broot.join(&id).join("manifest.json");
        let mut b: Backup =
            crate::json::compat::from_str(&std::fs::read_to_string(&manifest).unwrap()).unwrap();
        b.pid = 0;
        write_manifest(&manifest, &b).unwrap();
        let list = list_backups(&broot);
        assert_eq!(interrupted(&list).len(), 1);

        // Undo what was done.
        let res = restore_backup(&broot, &id, &reg).unwrap();
        assert!(res.iter().all(|r| r.ok), "{res:?}");
        assert_eq!(
            std::fs::read_to_string(work.join("a.txt")).unwrap(),
            "a.txt"
        );
        assert!(reg.exists("HKCU\\Software\\Gone"));
        assert!(interrupted(&list_backups(&broot)).is_empty());
    }

    #[test]
    fn keeping_an_interrupted_removal_stops_the_question() {
        let tmp = crate::util::tempdir().unwrap();
        let broot = tmp.path().join("backups");
        let w = BackupWriter::new(&broot, "x").unwrap();
        let id = w.id().to_string();
        std::mem::forget(w);
        let manifest = broot.join(&id).join("manifest.json");
        let mut b: Backup =
            crate::json::compat::from_str(&std::fs::read_to_string(&manifest).unwrap()).unwrap();
        b.pid = 0;
        write_manifest(&manifest, &b).unwrap();
        assert_eq!(interrupted(&list_backups(&broot)).len(), 1);
        mark_complete(&broot, &id).unwrap();
        assert!(interrupted(&list_backups(&broot)).is_empty());
    }

    #[test]
    fn finished_and_dropped_writers_are_complete() {
        let tmp = crate::util::tempdir().unwrap();
        let broot = tmp.path().join("backups");
        let w = BackupWriter::new(&broot, "finished").unwrap();
        w.finish().unwrap();
        {
            let _w = BackupWriter::new(&broot, "dropped after an error").unwrap();
        }
        assert!(list_backups(&broot).iter().all(|b| b.complete));
        // Backups made by older versions have no flag and count as complete.
        let old: Backup =
            crate::json::compat::from_str(r#"{"id":"1","title":"t","created":"c","items":[]}"#)
                .unwrap();
        assert!(old.complete);
    }

    #[test]
    fn absent_paths_require_a_durable_manifest_write() {
        let tmp = crate::util::tempdir().unwrap();
        let root = tmp.path().join("backups");
        let mut writer = BackupWriter::new(&root, "write-ahead").unwrap();
        let manifest = writer.dir().join("manifest.json");
        std::fs::remove_file(&manifest).unwrap();
        std::fs::create_dir(&manifest).unwrap();

        assert!(writer.add_file_absent("new-file").is_err());
        assert!(writer.add_dir_absent("new-folder").is_err());
    }

    #[test]
    fn a_restore_can_itself_be_undone() {
        let tmp = crate::util::tempdir().unwrap();
        let broot = tmp.path().join("backups");
        let work = tmp.path().join("work");
        std::fs::create_dir_all(work.join("sub")).unwrap();
        std::fs::write(work.join("sub").join("a.txt"), "hello").unwrap();

        let reg = MemRegistry::new();
        reg.put(KeyData::new("HKCU\\Software\\X\\Y").with(RegValue::string("v", "1")));
        reg.put(KeyData::new("HKCU\\Software\\Z").with(RegValue::dword("n", 5)));

        // A removal's backup.
        let mut w = BackupWriter::new(&broot, "test").unwrap();
        w.add_dir_tree(&work).unwrap();
        w.add_tree(&reg, "HKCU\\Software\\X").unwrap();
        w.add_value(&reg, "HKCU\\Software\\Z", "n").unwrap();
        w.add_value(&reg, "HKCU\\Software\\Z", "new").unwrap();
        let b = w.finish().unwrap();

        // The state the removal left behind (and later changes on top).
        std::fs::remove_dir_all(&work).unwrap();
        reg.delete_tree("HKCU\\Software\\X").unwrap();
        reg.set_value("HKCU\\Software\\Z", &RegValue::dword("n", 99))
            .unwrap();
        reg.set_value("HKCU\\Software\\Z", &RegValue::dword("new", 1))
            .unwrap();

        // The safety backup captures that current state, as an ordinary
        // backup listed beside the original.
        let safety = backup_before_restore(&broot, &b.id, &reg)
            .unwrap()
            .expect("the restore overwrites and recreates, so there is state to keep");
        assert_eq!(safety.title, "Before restoring: test");
        assert!(list_backups(&broot).iter().any(|x| x.id == safety.id));

        // Restoring the removal works as before…
        let res = restore_backup(&broot, &b.id, &reg).unwrap();
        assert!(res.iter().all(|r| r.ok), "{res:?}");
        assert_eq!(
            std::fs::read_to_string(work.join("sub").join("a.txt")).unwrap(),
            "hello"
        );
        assert!(reg.exists("HKCU\\Software\\X\\Y"));

        // …and restoring the safety backup brings back the exact state
        // the restore replaced.
        let res = restore_backup(&broot, &safety.id, &reg).unwrap();
        assert!(res.iter().all(|r| r.ok), "{res:?}");
        assert!(!work.exists(), "recreated folders are removed again");
        assert!(!reg.exists("HKCU\\Software\\X"));
        let z = reg.read_key("HKCU\\Software\\Z").unwrap();
        assert_eq!(z.get_u32("n"), Some(99));
        assert_eq!(z.get_u32("new"), Some(1));
    }

    #[test]
    fn a_restore_that_recreates_can_itself_be_undone() {
        let tmp = crate::util::tempdir().unwrap();
        let broot = tmp.path().join("backups");
        let newdir = tmp.path().join("newdir");
        let created = tmp.path().join("created.txt");
        let reg = MemRegistry::new();

        // A backup of an operation that only created things (recorded in
        // creation order, as the operations record them).
        let mut w = BackupWriter::new(&broot, "install").unwrap();
        w.add_dir_absent(&newdir.to_string_lossy()).unwrap();
        w.add_file_absent(&newdir.join("inner.txt").to_string_lossy())
            .unwrap();
        w.add_file_absent(&created.to_string_lossy()).unwrap();
        w.add_key_absent("HKCU\\Software\\New").unwrap();
        let b = w.finish().unwrap();

        // The created things, as they stand now.
        std::fs::create_dir_all(&newdir).unwrap();
        std::fs::write(newdir.join("inner.txt"), "inner").unwrap();
        std::fs::write(&created, "added").unwrap();
        reg.put(KeyData::new("HKCU\\Software\\New").with(RegValue::string("v", "on")));

        let safety = backup_before_restore(&broot, &b.id, &reg)
            .unwrap()
            .expect("the restore deletes, so there is state to keep");

        // Restoring removes everything the operation created…
        let res = restore_backup(&broot, &b.id, &reg).unwrap();
        assert!(res.iter().all(|r| r.ok), "{res:?}");
        assert!(!newdir.exists() && !created.exists());
        assert!(!reg.exists("HKCU\\Software\\New"));

        // …and the safety backup puts it all back.
        let res = restore_backup(&broot, &safety.id, &reg).unwrap();
        assert!(res.iter().all(|r| r.ok), "{res:?}");
        assert_eq!(
            std::fs::read_to_string(newdir.join("inner.txt")).unwrap(),
            "inner"
        );
        assert_eq!(std::fs::read_to_string(&created).unwrap(), "added");
        assert_eq!(
            reg.read_key("HKCU\\Software\\New")
                .unwrap()
                .get_string("v")
                .unwrap(),
            "on"
        );
    }

    #[test]
    fn a_safety_backup_with_nothing_to_keep_is_not_made() {
        let tmp = crate::util::tempdir().unwrap();
        let broot = tmp.path().join("backups");
        let reg = MemRegistry::new();
        // A backup that changed nothing.
        let w = BackupWriter::new(&broot, "nothing").unwrap();
        let b = w.finish().unwrap();
        assert!(backup_before_restore(&broot, &b.id, &reg)
            .unwrap()
            .is_none());
        // No half-made folder is left behind either.
        assert_eq!(list_backups(&broot).len(), 1);
    }

    #[test]
    fn a_safety_backup_that_cannot_be_taken_is_an_error() {
        let tmp = crate::util::tempdir().unwrap();
        let broot = tmp.path().join("backups");
        let thing = tmp.path().join("thing.txt");
        std::fs::write(&thing, "data").unwrap();
        let reg = MemRegistry::new();
        let mut w = BackupWriter::new(&broot, "test").unwrap();
        w.add_file(&thing).unwrap();
        let b = w.finish().unwrap();
        // The file has since been replaced by a folder, so its current
        // contents cannot be captured: the caller must not restore.
        std::fs::remove_file(&thing).unwrap();
        std::fs::create_dir(&thing).unwrap();
        assert!(backup_before_restore(&broot, &b.id, &reg).is_err());
        assert_eq!(list_backups(&broot).len(), 1);
    }
}
