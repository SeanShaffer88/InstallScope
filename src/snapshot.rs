//! Capturing system state and comparing two captures.

use crate::model::*;
use crate::platform::{self, expand_env, Registry};
use crate::settings::Settings;
use crate::util::Exclusions;
use anyhow::{Context, Result};
use std::io::{Read, Write};
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Mutex;
#[cfg(not(windows))]
use std::time::UNIX_EPOCH;

/// Shared progress/cancel state for long-running work on a background thread.
#[derive(Default)]
pub struct Progress {
    pub phase: Mutex<String>,
    pub items: AtomicU64,
    pub cancel: AtomicBool,
    item_limit: AtomicU64,
    limit_reached: AtomicBool,
    byte_limit: AtomicU64,
    captured_bytes: AtomicU64,
    byte_limit_reached: AtomicBool,
}

impl Progress {
    pub fn set_phase(&self, s: impl Into<String>) {
        *self.phase.lock().unwrap() = s.into();
    }
    pub fn phase(&self) -> String {
        self.phase.lock().unwrap().clone()
    }
    pub fn tick(&self) {
        let previous = self.items.fetch_add(1, Ordering::Relaxed);
        let limit = self.item_limit.load(Ordering::Relaxed);
        if limit != 0 && previous >= limit.saturating_sub(1) {
            self.limit_reached.store(true, Ordering::Relaxed);
            self.cancel.store(true, Ordering::Relaxed);
        }
    }
    /// Set before capture starts; zero keeps the existing unlimited behavior.
    /// Workers stop cooperatively, so this is not a hard memory allocation cap.
    pub fn set_item_limit(&self, limit: u64) {
        self.item_limit.store(limit, Ordering::Relaxed);
    }
    pub fn limit_reached(&self) -> bool {
        self.limit_reached.load(Ordering::Relaxed)
    }
    /// Accounting budget for snapshot payloads, not total process memory.
    pub fn set_byte_limit(&self, limit: u64) {
        self.byte_limit.store(limit, Ordering::Relaxed);
    }
    pub fn reserve_capture_bytes(&self, amount: u64) -> bool {
        let limit = self.byte_limit.load(Ordering::Relaxed);
        if limit == 0 {
            return true;
        }
        let mut used = self.captured_bytes.load(Ordering::Relaxed);
        loop {
            let Some(next) = used.checked_add(amount).filter(|next| *next <= limit) else {
                self.exceed_byte_limit();
                return false;
            };
            match self.captured_bytes.compare_exchange_weak(
                used,
                next,
                Ordering::Relaxed,
                Ordering::Relaxed,
            ) {
                Ok(_) => return true,
                Err(current) => used = current,
            }
        }
    }
    pub fn bounded_capture(&self) -> bool {
        self.byte_limit.load(Ordering::Relaxed) != 0
    }
    pub fn exceed_byte_limit(&self) {
        self.byte_limit_reached.store(true, Ordering::Relaxed);
        self.cancel.store(true, Ordering::Relaxed);
    }
    pub fn count(&self) -> u64 {
        self.items.load(Ordering::Relaxed)
    }
    pub fn cancelled(&self) -> bool {
        self.cancel.load(Ordering::Relaxed)
    }
}

/// Capture the configured registry roots and folders.
pub fn take_snapshot(
    reg: &dyn Registry,
    settings: &Settings,
    progress: &Progress,
) -> Result<Snapshot> {
    let mut snap = Snapshot {
        created: crate::util::now_string(),
        computer: platform::computer_name(),
        user: platform::user_name(),
        ..Default::default()
    };
    let reg_excl: Vec<String> = settings.effective_registry_exclusions();
    let file_excl: Vec<String> = settings.effective_file_exclusions();
    let roots: Vec<(String, bool)> = settings
        .file_roots
        .iter()
        .map(|r| (expand_env(&r.path), r.recursive))
        .filter(|(p, _)| !p.contains('%') && Path::new(p).exists())
        .collect();
    // Folders are walked on worker threads (a few at a time) while this
    // thread reads the registry; the parts are merged at the end.
    let workers = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4)
        .clamp(2, 6);
    let next = std::sync::atomic::AtomicUsize::new(0);
    let parts = std::thread::scope(|scope| {
        let handles: Vec<_> = (0..workers.min(roots.len()))
            .map(|_| {
                scope.spawn(|| {
                    let mut part = Snapshot::default();
                    loop {
                        let i = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        let Some((path, recursive)) = roots.get(i) else {
                            break;
                        };
                        if progress.cancelled() {
                            break;
                        }
                        capture_files(Path::new(path), *recursive, &file_excl, &mut part, progress);
                    }
                    part
                })
            })
            .collect();
        for root in &settings.registry_roots {
            let root = platform::normalize_reg_path(root);
            progress.set_phase(format!("Registry: {root}"));
            capture_registry(reg, &root, &reg_excl, &mut snap, progress);
            if progress.cancelled() {
                break;
            }
        }
        progress.set_phase("Files");
        // Join every worker before returning an error: never silently drop a
        // failed worker's evidence and present the remainder as a full capture.
        let joined: Vec<_> = handles.into_iter().map(|h| h.join()).collect();
        joined
            .into_iter()
            .map(|part| {
                part.map_err(|_| {
                    anyhow::anyhow!("snapshot file worker failed; evidence was not saved")
                })
            })
            .collect::<Result<Vec<_>>>()
    })?;
    if progress.cancelled() {
        if progress.byte_limit_reached.load(Ordering::Relaxed) {
            anyhow::bail!("snapshot payload byte budget exceeded; evidence was not saved");
        }
        if progress.limit_reached() {
            anyhow::bail!("snapshot item limit reached; evidence was not saved");
        }
        anyhow::bail!("cancelled");
    }
    for part in parts {
        snap.absorb(part);
    }
    progress.set_phase("Sorting");
    snap.finish();
    progress.set_phase("Done");
    Ok(snap)
}

/// Depth-first walk of a registry subtree, skipping excluded branches.
pub fn capture_registry(
    reg: &dyn Registry,
    root: &str,
    excl: &[String],
    snap: &mut Snapshot,
    progress: &Progress,
) {
    reg.capture(root, &Exclusions::new(excl), snap, progress);
}

#[cfg(not(windows))]
fn mtime_of(meta: &std::fs::Metadata) -> i64 {
    meta.modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Record every file and folder under `root` (not following links or
/// junctions). Excluded locations are neither recorded nor entered.
pub fn capture_files(
    root: &Path,
    recursive: bool,
    excl: &[String],
    snap: &mut Snapshot,
    progress: &Progress,
) {
    let excl = Exclusions::new(excl);
    let mut folded = String::new();
    let mut chars = Vec::new();
    #[cfg(windows)]
    {
        let root = root.to_string_lossy();
        let mut warnings = Vec::new();
        platform::windows::walk_files(
            &root,
            recursive,
            &mut |path, is_dir, size, mtime| {
                if progress.cancelled() {
                    return platform::windows::Walk::Stop;
                }
                if progress.bounded_capture() {
                    progress.tick();
                    if progress.cancelled() {
                        return platform::windows::Walk::Stop;
                    }
                }
                crate::util::fold_into(path, &mut folded);
                if excl.matches_folded(&folded, &mut chars) {
                    return platform::windows::Walk::Skip;
                }
                if !progress.reserve_capture_bytes(path.len() as u64 + 32) {
                    return platform::windows::Walk::Stop;
                }
                snap.push_file(path, is_dir, size, mtime);
                progress.tick();
                platform::windows::Walk::Continue
            },
            &mut |path| {
                if progress.reserve_capture_bytes(path.len() as u64 + 64) {
                    warnings.push(format!("Could not read {path}"));
                }
            },
        );
        snap.warnings.extend(warnings);
    }
    #[cfg(not(windows))]
    {
        // Development builds: plain std walk (allocates per entry).
        let mut stack = vec![(root.to_path_buf(), 0usize)];
        while let Some((dir, depth)) = stack.pop() {
            let rd = match std::fs::read_dir(&dir) {
                Ok(r) => r,
                Err(_) => {
                    snap.warnings
                        .push(format!("Could not read {}", dir.display()));
                    continue;
                }
            };
            for e in rd.flatten() {
                if progress.cancelled() {
                    return;
                }
                let Ok(ft) = e.file_type() else { continue };
                if ft.is_symlink() {
                    continue;
                }
                let p = e.path();
                let ps = p.to_string_lossy();
                crate::util::fold_into(&ps, &mut folded);
                if excl.matches_folded(&folded, &mut chars) {
                    continue;
                }
                let Ok(meta) = e.metadata() else { continue };
                let is_dir = meta.is_dir();
                if !progress.reserve_capture_bytes(ps.len() as u64 + 32) {
                    return;
                }
                snap.push_file(
                    &ps,
                    is_dir,
                    if is_dir { 0 } else { meta.len() },
                    mtime_of(&meta),
                );
                progress.tick();
                if is_dir && recursive {
                    stack.push((p, depth + 1));
                }
            }
        }
    }
}

fn sorted(s: &Snapshot) -> std::borrow::Cow<'_, Snapshot> {
    if s.sorted {
        std::borrow::Cow::Borrowed(s)
    } else {
        let mut c = s.clone();
        c.finish();
        std::borrow::Cow::Owned(c)
    }
}

/// Compare two snapshots in one merge over their sorted records.
/// Results are sorted by path.
pub fn diff(before: &Snapshot, after: &Snapshot) -> ChangeSet {
    use std::cmp::Ordering::*;
    let (b, a) = (sorted(before), sorted(after));
    let (b, a) = (b.as_ref(), a.as_ref());
    let mut cs = ChangeSet::default();

    let (mut i, mut j) = (0usize, 0usize);
    while i < b.key_count() || j < a.key_count() {
        let ord = match (i < b.key_count(), j < a.key_count()) {
            (true, true) => cmp_fold(b.key_path(i), a.key_path(j)),
            (true, false) => Less,
            _ => Greater,
        };
        match ord {
            Less => {
                cs.keys_removed.push(b.key_data(i));
                i += 1;
            }
            Greater => {
                cs.keys_added.push(a.key_data(j));
                j += 1;
            }
            Equal => {
                diff_values(b, i, a, j, &mut cs);
                i += 1;
                j += 1;
            }
        }
    }

    let (mut i, mut j) = (0usize, 0usize);
    while i < b.file_count() || j < a.file_count() {
        let ord = match (i < b.file_count(), j < a.file_count()) {
            (true, true) => cmp_fold(b.file(i).path, a.file(j).path),
            (true, false) => Less,
            _ => Greater,
        };
        match ord {
            Less => {
                cs.files_removed.push(b.file(i).to_entry());
                i += 1;
            }
            Greater => {
                cs.files_added.push(a.file(j).to_entry());
                j += 1;
            }
            Equal => {
                let (bf, af) = (b.file(i), a.file(j));
                // Folder timestamps change whenever their contents do; only
                // files count as modified.
                if !af.is_dir && (bf.size != af.size || bf.mtime != af.mtime || bf.is_dir) {
                    cs.files_modified.push(FileChange {
                        before: bf.to_entry(),
                        after: af.to_entry(),
                    });
                }
                i += 1;
                j += 1;
            }
        }
    }
    cs
}

/// The value differences of one key present in both snapshots.
fn diff_values(b: &Snapshot, bi: usize, a: &Snapshot, ai: usize, cs: &mut ChangeSet) {
    use std::cmp::Ordering::*;
    let mut bv = b.key_values(bi).peekable();
    let mut av = a.key_values(ai).peekable();
    let path = a.key_path(ai);
    loop {
        let ord = match (bv.peek(), av.peek()) {
            (Some(x), Some(y)) => cmp_fold(x.name, y.name),
            (Some(_), None) => Less,
            (None, Some(_)) => Greater,
            (None, None) => break,
        };
        match ord {
            Less => {
                let x = bv.next().unwrap();
                cs.values_removed.push(ValueRef {
                    key: path.to_string(),
                    value: x.to_value(),
                });
            }
            Greater => {
                let y = av.next().unwrap();
                cs.values_added.push(ValueRef {
                    key: path.to_string(),
                    value: y.to_value(),
                });
            }
            Equal => {
                let (x, y) = (bv.next().unwrap(), av.next().unwrap());
                if x.kind != y.kind || x.data != y.data {
                    cs.values_modified.push(ValueChange {
                        key: path.to_string(),
                        before: x.to_value(),
                        after: y.to_value(),
                    });
                }
            }
        }
    }
}

/// Drop changes matching exclusion patterns after the fact (used when the
/// user adds an exclusion while reviewing a capture).
pub fn filter_changes(cs: &mut ChangeSet, reg_excl: &[String], file_excl: &[String]) {
    let reg = Exclusions::new(reg_excl);
    let files = Exclusions::new(file_excl);
    cs.keys_added.retain(|k| !reg.matches(&k.path));
    cs.keys_removed.retain(|k| !reg.matches(&k.path));
    cs.values_added.retain(|v| !reg.matches(&v.key));
    cs.values_removed.retain(|v| !reg.matches(&v.key));
    cs.values_modified.retain(|v| !reg.matches(&v.key));
    cs.files_added.retain(|f| !files.matches(&f.path));
    cs.files_removed.retain(|f| !files.matches(&f.path));
    cs.files_modified.retain(|f| !files.matches(&f.after.path));
}

/// Program identity found in a change set (from a new Uninstall entry).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DetectedProgram {
    pub key: String,
    pub name: String,
    pub publisher: String,
    pub version: String,
}

pub fn is_uninstall_entry(path: &str) -> bool {
    let f = fold(path);
    let marker = "\\microsoft\\windows\\currentversion\\uninstall\\";
    match f.find(marker) {
        Some(i) => !f[i + marker.len()..].contains('\\'),
        None => false,
    }
}

/// Find the Uninstall entry an installation created (or updated).
pub fn detect_program(cs: &ChangeSet) -> Option<DetectedProgram> {
    let from_key = |k: &KeyData| {
        k.get_string("DisplayName").map(|name| DetectedProgram {
            key: k.path.clone(),
            name,
            publisher: k.get_string("Publisher").unwrap_or_default(),
            version: k.get_string("DisplayVersion").unwrap_or_default(),
        })
    };
    let mut found: Vec<DetectedProgram> = cs
        .keys_added
        .iter()
        .filter(|k| is_uninstall_entry(&k.path))
        .filter_map(from_key)
        .collect();
    if found.is_empty() {
        // An upgrade rewrites an existing entry instead.
        let mut keys: std::collections::BTreeMap<String, KeyData> = Default::default();
        for v in &cs.values_modified {
            if is_uninstall_entry(&v.key) {
                keys.entry(fold(&v.key))
                    .or_insert_with(|| KeyData::new(&v.key))
                    .values
                    .insert(fold(&v.after.name), v.after.clone());
            }
        }
        for v in &cs.values_added {
            if is_uninstall_entry(&v.key) {
                keys.entry(fold(&v.key))
                    .or_insert_with(|| KeyData::new(&v.key))
                    .values
                    .insert(fold(&v.value.name), v.value.clone());
            }
        }
        found = keys.values().filter_map(from_key).collect();
    }
    // Prefer the entry that isn't a hidden component / update.
    found.into_iter().next()
}

const MAGIC: &[u8; 8] = b"EXSNAP2\n";

fn put_u32(out: &mut Vec<u8>, v: u32) {
    out.extend_from_slice(&v.to_le_bytes());
}
fn put_u64(out: &mut Vec<u8>, v: u64) {
    out.extend_from_slice(&v.to_le_bytes());
}
fn put_str(out: &mut Vec<u8>, s: &str) {
    put_u64(out, s.len() as u64);
    out.extend_from_slice(s.as_bytes());
}

/// Save a snapshot: a compact binary layout of its pools and records,
/// gzip-compressed. Written to a temporary file and renamed into place, so
/// an interrupted save never leaves a damaged snapshot behind.
pub fn save_snapshot(path: &Path, snap: &Snapshot) -> Result<()> {
    save_snapshot_with(path, snap, true)
}

/// Save new evidence without replacing a prior named snapshot, even in a race.
pub fn save_new_snapshot(path: &Path, snap: &Snapshot) -> Result<()> {
    save_snapshot_with(path, snap, false)
}

fn save_snapshot_with(path: &Path, snap: &Snapshot, replace: bool) -> Result<()> {
    if let Some(p) = path.parent() {
        std::fs::create_dir_all(p)?;
    }
    let snap = sorted(snap);
    let s = snap.as_ref();
    crate::store::durable::write_with(path, replace, |file| {
      let encode = || -> Result<()> {
        let mut gz = crate::deflate::GzWriter::fast(file)?;
        let mut head = Vec::with_capacity(4096);
        head.extend_from_slice(MAGIC);
        put_str(&mut head, &s.created);
        put_str(&mut head, &s.computer);
        put_str(&mut head, &s.user);
        put_u64(&mut head, s.warnings.len() as u64);
        for w in &s.warnings {
            put_str(&mut head, w);
        }
        put_u64(&mut head, s.text.len() as u64);
        gz.write_all(&head)?;
        gz.write_all(s.text.as_bytes())?;
        let mut buf = Vec::with_capacity(64 * 1024);
        put_u64(&mut buf, s.bytes.len() as u64);
        gz.write_all(&buf)?;
        gz.write_all(&s.bytes)?;
        // Records go out through one reused buffer.
        let flush = |gz: &mut crate::deflate::GzWriter<_>, buf: &mut Vec<u8>| -> Result<()> {
            gz.write_all(buf)?;
            buf.clear();
            Ok(())
        };
        buf.clear();
        put_u64(&mut buf, s.files.len() as u64);
        for f in &s.files {
            put_u32(&mut buf, f.path.off);
            put_u32(&mut buf, f.path.len);
            put_u64(&mut buf, f.size);
            put_u64(&mut buf, f.mtime as u64);
            buf.push(f.is_dir as u8);
            if buf.len() > 60 * 1024 {
                flush(&mut gz, &mut buf)?;
            }
        }
        put_u64(&mut buf, s.keys.len() as u64);
        for k in &s.keys {
            put_u32(&mut buf, k.path.off);
            put_u32(&mut buf, k.path.len);
            put_u32(&mut buf, k.first);
            put_u32(&mut buf, k.count);
            if buf.len() > 60 * 1024 {
                flush(&mut gz, &mut buf)?;
            }
        }
        put_u64(&mut buf, s.vals.len() as u64);
        for v in &s.vals {
            put_u32(&mut buf, v.name.off);
            put_u32(&mut buf, v.name.len);
            put_u32(&mut buf, v.kind);
            put_u32(&mut buf, v.data.off);
            put_u32(&mut buf, v.data.len);
            if buf.len() > 60 * 1024 {
                flush(&mut gz, &mut buf)?;
            }
        }
        flush(&mut gz, &mut buf)?;
        gz.finish()?;
        Ok(())
      };
      encode().map_err(|e| std::io::Error::other(format!("{e:#}")))
    }).with_context(|| format!("save {} (existing evidence is never removed)", path.display()))?;
    Ok(())
}

/// Reads the binary layout back, checking every record against its pools.
struct Reader<'a> {
    b: &'a [u8],
    at: usize,
}

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8]> {
        let end = self
            .at
            .checked_add(n)
            .filter(|e| *e <= self.b.len())
            .ok_or_else(|| anyhow::anyhow!("the snapshot is damaged (cut off)"))?;
        let s = &self.b[self.at..end];
        self.at = end;
        Ok(s)
    }
    fn u32(&mut self) -> Result<u32> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }
    fn u64(&mut self) -> Result<u64> {
        Ok(u64::from_le_bytes(self.take(8)?.try_into().unwrap()))
    }
    fn len(&mut self) -> Result<usize> {
        usize::try_from(self.u64()?).map_err(|_| anyhow::anyhow!("the snapshot is damaged"))
    }
    fn string(&mut self) -> Result<String> {
        let n = self.len()?;
        Ok(std::str::from_utf8(self.take(n)?)?.to_string())
    }
    fn count(&mut self, rec: usize) -> Result<usize> {
        let n = self.len()?;
        if n.saturating_mul(rec) > self.b.len() - self.at {
            anyhow::bail!("the snapshot is damaged (bad record count)");
        }
        Ok(n)
    }
}

fn parse_binary(b: &[u8]) -> Result<Snapshot> {
    let mut r = Reader { b, at: MAGIC.len() };
    let mut s = Snapshot {
        created: r.string()?,
        computer: r.string()?,
        user: r.string()?,
        ..Default::default()
    };
    for _ in 0..r.count(8)? {
        s.warnings.push(r.string()?);
    }
    let n = r.len()?;
    s.text = std::str::from_utf8(r.take(n)?)?.to_string();
    let n = r.len()?;
    s.bytes = r.take(n)?.to_vec();
    let text_ok = |sp: Span, text: &str| -> bool {
        let (a, e) = (sp.off as usize, sp.off as usize + sp.len as usize);
        e <= text.len() && text.is_char_boundary(a) && text.is_char_boundary(e)
    };
    let n = r.count(25)?;
    s.files.reserve_exact(n);
    for _ in 0..n {
        let path = Span {
            off: r.u32()?,
            len: r.u32()?,
        };
        let size = r.u64()?;
        let mtime = r.u64()? as i64;
        let is_dir = r.take(1)?[0] != 0;
        if !text_ok(path, &s.text) {
            anyhow::bail!("the snapshot is damaged (file record)");
        }
        s.files.push(FileRec {
            path,
            size,
            mtime,
            is_dir,
        });
    }
    let n = r.count(16)?;
    s.keys.reserve_exact(n);
    for _ in 0..n {
        let k = KeyRec {
            path: Span {
                off: r.u32()?,
                len: r.u32()?,
            },
            first: r.u32()?,
            count: r.u32()?,
        };
        if !text_ok(k.path, &s.text) {
            anyhow::bail!("the snapshot is damaged (key record)");
        }
        s.keys.push(k);
    }
    let n = r.count(20)?;
    s.vals.reserve_exact(n);
    for _ in 0..n {
        let v = ValRec {
            name: Span {
                off: r.u32()?,
                len: r.u32()?,
            },
            kind: r.u32()?,
            data: Span {
                off: r.u32()?,
                len: r.u32()?,
            },
        };
        if !text_ok(v.name, &s.text) || (v.data.off as usize + v.data.len as usize) > s.bytes.len()
        {
            anyhow::bail!("the snapshot is damaged (value record)");
        }
        s.vals.push(v);
    }
    if s.keys
        .iter()
        .any(|k| (k.first as usize + k.count as usize) > s.vals.len())
    {
        anyhow::bail!("the snapshot is damaged (key values)");
    }
    // Written sorted; sort again only if it wasn't (cheap check).
    s.sorted = false;
    s.finish();
    Ok(s)
}

/// The JSON layout earlier versions saved.
struct LegacySnapshot {
    created: String,
    computer: String,
    user: String,
    registry: std::collections::BTreeMap<String, KeyData>,
    files: std::collections::BTreeMap<String, FileEntry>,
    warnings: Vec<String>,
}
crate::json_struct!(LegacySnapshot, { created [default], computer [default], user [default], registry [default], files [default], warnings [default] });

pub fn load_snapshot(path: &Path) -> Result<Snapshot> {
    load_snapshot_bounded(path, 512 * 1024 * 1024)
}

/// Reject oversized expanded evidence before parsing it. The limit covers
/// decompressed input bytes, not the additional memory used by parsed records.
pub fn load_snapshot_bounded(path: &Path, max_expanded_bytes: u64) -> Result<Snapshot> {
    let read_limit = max_expanded_bytes
        .checked_add(1)
        .context("snapshot byte limit is too large")?;
    let f = std::fs::File::open(path).with_context(|| format!("open {}", path.display()))?;
    let mut raw = Vec::new();
    crate::deflate::GzReader::new(f)
        .with_context(|| format!("read {}", path.display()))?
        .take(read_limit)
        .read_to_end(&mut raw)
        .with_context(|| format!("read {}", path.display()))?;
    anyhow::ensure!(
        raw.len() as u64 <= max_expanded_bytes,
        "snapshot exceeds expanded byte limit ({max_expanded_bytes} bytes): {}",
        path.display()
    );
    if raw.starts_with(MAGIC) {
        return parse_binary(&raw).with_context(|| format!("read {}", path.display()));
    }
    // Saved by an earlier version.
    let old: LegacySnapshot = crate::json::compat::from_slice(&raw)?;
    drop(raw);
    let mut s = Snapshot {
        created: old.created,
        computer: old.computer,
        user: old.user,
        warnings: old.warnings,
        ..Default::default()
    };
    for k in old.registry.into_values() {
        s.insert_key(k);
    }
    for f in old.files.into_values() {
        s.insert_file(f);
    }
    s.finish();
    Ok(s)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::platform::MemRegistry;

    #[test]
    fn new_snapshot_preserves_prior_evidence_and_legacy_staging_files() {
        let tmp = crate::util::tempdir().unwrap();
        let path = tmp.path().join("named.snap");
        let legacy = path.with_extension("snap.tmp");
        std::fs::write(&legacy, b"preserve interrupted evidence").unwrap();
        save_new_snapshot(&path, &Snapshot::default()).unwrap();
        let original = std::fs::read(&path).unwrap();
        assert!(save_new_snapshot(&path, &Snapshot {created:"replacement".into(), ..Default::default()}).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), original);
        assert_eq!(std::fs::read(&legacy).unwrap(), b"preserve interrupted evidence");
        load_snapshot(&path).unwrap();
    }

    #[test]
    fn bounded_snapshot_rejects_expansion_and_accepts_exact_boundary() {
        let tmp = crate::util::tempdir().unwrap();
        let path = tmp.path().join("bounded.snap");
        save_snapshot(&path, &Snapshot::default()).unwrap();
        let mut bytes = Vec::new();
        crate::deflate::GzReader::new(std::fs::File::open(&path).unwrap())
            .unwrap()
            .read_to_end(&mut bytes)
            .unwrap();
        load_snapshot_bounded(&path, bytes.len() as u64).unwrap();
        let error = load_snapshot_bounded(&path, bytes.len() as u64 - 1).unwrap_err();
        assert!(error.to_string().contains("expanded byte limit"));
        assert!(load_snapshot_bounded(&path, u64::MAX).is_err());
    }

    #[test]
    fn bounded_snapshot_still_checks_gzip_integrity() {
        let tmp = crate::util::tempdir().unwrap();
        let path = tmp.path().join("corrupt.snap");
        save_snapshot(&path, &Snapshot::default()).unwrap();
        let mut bytes = std::fs::read(&path).unwrap();
        let trailer = bytes.len() - 8;
        bytes[trailer] ^= 1;
        std::fs::write(&path, bytes).unwrap();
        assert!(load_snapshot_bounded(&path, 1024 * 1024).is_err());
    }

    #[test]
    fn highly_compressed_snapshot_is_stopped_before_parsing() {
        let tmp = crate::util::tempdir().unwrap();
        let path = tmp.path().join("expanded.snap");
        let mut gz = crate::deflate::GzWriter::fast(std::fs::File::create(&path).unwrap()).unwrap();
        gz.write_all(&vec![b' '; 1024 * 1024]).unwrap();
        gz.finish().unwrap();
        assert!(std::fs::metadata(&path).unwrap().len() < 64 * 1024);
        let error = load_snapshot_bounded(&path, 64 * 1024).unwrap_err();
        assert!(error.to_string().contains("expanded byte limit"));
    }

    #[test]
    fn capture_limit_cannot_produce_successful_evidence() {
        let tmp = crate::util::tempdir().unwrap();
        std::fs::write(tmp.path().join("fixture"), "x").unwrap();
        let progress = Progress::default();
        progress.set_item_limit(1);
        let mut settings = settings_for(&tmp.path().to_string_lossy());
        settings.registry_roots.clear();
        let error = take_snapshot(&MemRegistry::new(), &settings, &progress).unwrap_err();
        assert!(progress.limit_reached());
        assert!(error.to_string().contains("item limit"));
        let normal = Progress::default();
        normal.tick();
        assert!(!normal.cancelled());
    }

    fn settings_for(root: &str) -> Settings {
        Settings {
            registry_roots: vec!["HKLM\\SOFTWARE".into()],
            registry_exclusions: vec!["HKLM\\SOFTWARE\\Noise".into()],
            file_roots: vec![crate::settings::FileRoot {
                path: root.into(),
                recursive: true,
            }],
            file_exclusions: vec!["*cache".into()],
            // The test folders live under %TEMP%, which the built-in list ignores.
            auto_exclusions: false,
            ..Settings::default()
        }
    }

    #[test]
    fn snapshot_diff_detects_install() {
        let tmp = crate::util::tempdir().unwrap();
        let root = tmp.path().to_string_lossy().to_string();
        let reg = MemRegistry::new();
        reg.put(
            KeyData::new("HKLM\\SOFTWARE\\Existing")
                .with(RegValue::dword("a", 1))
                .with(RegValue::dword("gone", 9)),
        );
        reg.put(KeyData::new("HKLM\\SOFTWARE\\Old"));
        std::fs::write(tmp.path().join("keep.txt"), "x").unwrap();
        let s = settings_for(&root);
        let p = Progress::default();
        let before = take_snapshot(&reg, &s, &p).unwrap();

        // "Install"
        let un = "HKLM\\SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\Foo";
        reg.put(
            KeyData::new(un)
                .with(RegValue::string("DisplayName", "Foo App"))
                .with(RegValue::string("Publisher", "FooCorp")),
        );
        reg.put(
            KeyData::new("HKLM\\SOFTWARE\\FooCorp\\Foo").with(RegValue::string("Path", "C:\\Foo")),
        );
        reg.set_value("HKLM\\SOFTWARE\\Existing", &RegValue::dword("a", 2))
            .unwrap();
        reg.set_value("HKLM\\SOFTWARE\\Existing", &RegValue::dword("b", 3))
            .unwrap();
        reg.delete_value("HKLM\\SOFTWARE\\Existing", "gone")
            .unwrap();
        reg.delete_tree("HKLM\\SOFTWARE\\Old").unwrap();
        reg.put(KeyData::new("HKLM\\SOFTWARE\\Noise\\X"));
        std::fs::create_dir_all(tmp.path().join("Foo").join("cache")).unwrap();
        std::fs::write(tmp.path().join("Foo").join("foo.exe"), "bin").unwrap();
        std::fs::write(tmp.path().join("Foo").join("cache").join("c.bin"), "c").unwrap();

        let after = take_snapshot(&reg, &s, &p).unwrap();
        let cs = diff(&before, &after);

        let added: Vec<&str> = cs.keys_added.iter().map(|k| k.path.as_str()).collect();
        assert!(added.contains(&un));
        assert!(added.contains(&"HKLM\\SOFTWARE\\FooCorp\\Foo"));
        assert!(!added.iter().any(|k| k.contains("Noise")));
        assert_eq!(cs.values_added.len(), 1);
        assert_eq!(cs.values_modified.len(), 1);
        assert_eq!(cs.values_removed.len(), 1);
        assert_eq!(cs.keys_removed.len(), 1);
        let files: Vec<String> = cs.files_added.iter().map(|f| f.path.clone()).collect();
        assert_eq!(files.len(), 2, "{files:?}"); // Foo dir + foo.exe, cache excluded

        let d = detect_program(&cs).unwrap();
        assert_eq!(d.name, "Foo App");
        assert_eq!(d.publisher, "FooCorp");
    }

    fn sample() -> Snapshot {
        let mut s = Snapshot {
            created: "2026-10-16 05:00:00".into(),
            computer: "PC".into(),
            user: "Sean".into(),
            warnings: vec!["Could not read X".into()],
            ..Default::default()
        };
        s.insert_key(
            KeyData::new("HKCU\\Software\\Zeta")
                .with(RegValue::string("", "default"))
                .with(RegValue {
                    name: "Bin".into(),
                    kind: reg_type::BINARY,
                    data: vec![0, 1, 2, 255],
                }),
        );
        s.insert_key(KeyData::new("HKCU\\Software\\Ärger").with(RegValue::string("ü", "ö")));
        s.insert_file(FileEntry {
            path: "C:\\Program Files\\App\\b.exe".into(),
            is_dir: false,
            size: 1 << 33,
            mtime: -5,
        });
        s.insert_file(FileEntry {
            path: "C:\\Program Files\\App".into(),
            is_dir: true,
            size: 0,
            mtime: 1_700_000_000,
        });
        s.finish();
        s
    }

    fn keys_of(s: &Snapshot) -> Vec<KeyData> {
        (0..s.key_count()).map(|i| s.key_data(i)).collect()
    }

    fn files_of(s: &Snapshot) -> Vec<FileEntry> {
        s.files().map(|f| f.to_entry()).collect()
    }

    #[test]
    fn snapshot_save_load() {
        let tmp = crate::util::tempdir().unwrap();
        let s = sample();
        let p = tmp.path().join("a.snap");
        save_snapshot(&p, &s).unwrap();
        let l = load_snapshot(&p).unwrap();
        assert_eq!(keys_of(&l), keys_of(&s));
        assert_eq!(files_of(&l), files_of(&s));
        assert_eq!(
            (l.created.as_str(), l.computer.as_str(), l.user.as_str()),
            ("2026-10-16 05:00:00", "PC", "Sean")
        );
        assert_eq!(l.warnings, s.warnings);
        assert!(!p.with_extension("snap.tmp").exists());
    }

    #[test]
    fn snapshots_from_earlier_versions_still_load() {
        let tmp = crate::util::tempdir().unwrap();
        let p = tmp.path().join("old.snap");
        let json = r#"{"created":"c","computer":"pc","user":"u",
            "registry":{"hkcu\\software\\a":{"path":"HKCU\\Software\\A","values":{"x":{"name":"x","kind":1,"data":"790000"}}}},
            "files":{"c:\\a.txt":{"path":"C:\\a.txt","is_dir":false,"size":3,"mtime":9}}}"#;
        let f = std::fs::File::create(&p).unwrap();
        let mut gz = crate::deflate::GzWriter::new(f).unwrap();
        gz.write_all(json.as_bytes()).unwrap();
        gz.finish().unwrap();
        let l = load_snapshot(&p).unwrap();
        assert_eq!(l.key_count(), 1);
        assert_eq!(l.key_data(0).get_string("x").as_deref(), Some("y"));
        assert_eq!(files_of(&l)[0].size, 3);
    }

    #[test]
    fn a_damaged_snapshot_is_refused_not_misread() {
        let tmp = crate::util::tempdir().unwrap();
        let p = tmp.path().join("a.snap");
        save_snapshot(&p, &sample()).unwrap();
        let mut raw = Vec::new();
        crate::deflate::GzReader::new(std::fs::File::open(&p).unwrap())
            .unwrap()
            .read_to_end(&mut raw)
            .unwrap();
        for cut in [raw.len() - 3, raw.len() / 2, 12] {
            assert!(parse_binary(&raw[..cut]).is_err(), "cut at {cut}");
        }
        // A record pointing outside its pool.
        let mut bad = raw.clone();
        let n = bad.len();
        bad[n - 20..n - 16].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(parse_binary(&bad).is_err());
    }

    #[test]
    fn duplicates_keep_the_latest_capture_and_order_ignores_case() {
        let mut s = Snapshot::default();
        s.push_file("C:\\b", false, 1, 0);
        s.push_file("C:\\A", false, 1, 0);
        s.push_file("c:\\B", false, 2, 0);
        s.finish();
        let got: Vec<(String, u64)> = s.files().map(|f| (f.path.to_string(), f.size)).collect();
        assert_eq!(got, [("C:\\A".to_string(), 1), ("c:\\B".to_string(), 2)]);
        assert!(s.find_file("c:\\a").is_some() && s.find_file("C:\\c").is_none());
    }

    #[test]
    fn diff_merges_regardless_of_case_and_insertion_order() {
        let mut a = Snapshot::default();
        a.insert_key(
            KeyData::new("HKCU\\Software\\Keep")
                .with(RegValue::string("V", "1"))
                .with(RegValue::string("Gone", "x")),
        );
        a.insert_key(KeyData::new("HKCU\\Software\\Old"));
        a.push_file("C:\\App\\same.txt", false, 1, 1);
        a.push_file("C:\\App\\changed.txt", false, 1, 1);
        a.push_file("C:\\App\\removed.txt", false, 1, 1);
        let mut b = Snapshot::default();
        b.push_file("C:\\app\\NEW.txt", false, 1, 1);
        b.push_file("C:\\APP\\SAME.TXT", false, 1, 1);
        b.push_file("C:\\App\\changed.txt", false, 2, 1);
        b.insert_key(KeyData::new("HKCU\\Software\\New"));
        b.insert_key(
            KeyData::new("hkcu\\software\\keep")
                .with(RegValue::string("v", "2"))
                .with(RegValue::string("Added", "y")),
        );
        // Unsorted inputs are sorted for the comparison.
        let cs = diff(&a, &b);
        let names = |v: &Vec<FileEntry>| v.iter().map(|f| f.path.clone()).collect::<Vec<_>>();
        assert_eq!(names(&cs.files_added), ["C:\\app\\NEW.txt"]);
        assert_eq!(names(&cs.files_removed), ["C:\\App\\removed.txt"]);
        assert_eq!(cs.files_modified.len(), 1);
        assert_eq!(
            cs.keys_added
                .iter()
                .map(|k| k.path.as_str())
                .collect::<Vec<_>>(),
            ["HKCU\\Software\\New"]
        );
        assert_eq!(
            cs.keys_removed
                .iter()
                .map(|k| k.path.as_str())
                .collect::<Vec<_>>(),
            ["HKCU\\Software\\Old"]
        );
        assert_eq!(
            cs.values_added
                .iter()
                .map(|v| v.value.name.as_str())
                .collect::<Vec<_>>(),
            ["Added"]
        );
        assert_eq!(
            cs.values_removed
                .iter()
                .map(|v| v.value.name.as_str())
                .collect::<Vec<_>>(),
            ["Gone"]
        );
        assert_eq!(cs.values_modified.len(), 1);
        assert_eq!(
            cs.values_modified[0].after.as_string().as_deref(),
            Some("2")
        );
    }

    #[test]
    fn uninstall_entry_detection() {
        assert!(is_uninstall_entry(
            "HKLM\\SOFTWARE\\WOW6432Node\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\X"
        ));
        assert!(!is_uninstall_entry(
            "HKLM\\SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\X\\Sub"
        ));
    }
}
