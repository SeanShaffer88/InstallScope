//! Core data types shared by every part of InstallScope.
//!
//! Registry and file paths are compared case-insensitively (like Windows does),
//! so every map is keyed by a *folded* (lower-cased) path while the entry itself
//! keeps the original spelling for display and for writing back.

use std::collections::BTreeMap;

/// Raw registry value types (winnt.h).
pub mod reg_type {
    pub const NONE: u32 = 0;
    pub const SZ: u32 = 1;
    pub const EXPAND_SZ: u32 = 2;
    pub const BINARY: u32 = 3;
    pub const DWORD: u32 = 4;
    pub const DWORD_BIG_ENDIAN: u32 = 5;
    pub const LINK: u32 = 6;
    pub const MULTI_SZ: u32 = 7;
    pub const QWORD: u32 = 11;
}

/// Case-fold a registry or file path / value name for use as a map key.
pub fn fold(s: &str) -> String {
    s.to_lowercase()
}

/// Byte vectors as compact hex text (how registry data is stored).
pub mod hexbytes {
    pub fn to_hex(data: &[u8]) -> String {
        const HEX: &[u8; 16] = b"0123456789abcdef";
        let mut out = String::with_capacity(data.len() * 2);
        for b in data {
            out.push(HEX[(b >> 4) as usize] as char);
            out.push(HEX[(b & 0xf) as usize] as char);
        }
        out
    }

    pub fn from_hex(s: &str) -> Result<Vec<u8>, String> {
        if !s.len().is_multiple_of(2) {
            return Err("odd-length hex string".into());
        }
        (0..s.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&s[i..i + 2], 16).map_err(|e| e.to_string()))
            .collect()
    }
}

/// A single registry value with its raw type and bytes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RegValue {
    /// Value name; the empty string is the key's "(Default)" value.
    pub name: String,
    pub kind: u32,
    pub data: Vec<u8>,
}
crate::json_struct!(RegValue, { name, kind, data [hex] });

impl RegValue {
    pub fn string(name: &str, s: &str) -> Self {
        RegValue {
            name: name.into(),
            kind: reg_type::SZ,
            data: encode_utf16z(s),
        }
    }

    pub fn dword(name: &str, v: u32) -> Self {
        RegValue {
            name: name.into(),
            kind: reg_type::DWORD,
            data: v.to_le_bytes().to_vec(),
        }
    }

    /// The value interpreted as text, when it is a string type.
    pub fn as_string(&self) -> Option<String> {
        match self.kind {
            reg_type::SZ | reg_type::EXPAND_SZ | reg_type::LINK => Some(decode_utf16z(&self.data)),
            reg_type::MULTI_SZ => Some(decode_multi_sz(&self.data).join("; ")),
            _ => None,
        }
    }

    pub fn as_u32(&self) -> Option<u32> {
        match self.kind {
            reg_type::DWORD if self.data.len() >= 4 => Some(u32::from_le_bytes([
                self.data[0],
                self.data[1],
                self.data[2],
                self.data[3],
            ])),
            reg_type::DWORD_BIG_ENDIAN if self.data.len() >= 4 => Some(u32::from_be_bytes([
                self.data[0],
                self.data[1],
                self.data[2],
                self.data[3],
            ])),
            _ => None,
        }
    }

    pub fn type_name(&self) -> &'static str {
        match self.kind {
            reg_type::NONE => "REG_NONE",
            reg_type::SZ => "REG_SZ",
            reg_type::EXPAND_SZ => "REG_EXPAND_SZ",
            reg_type::BINARY => "REG_BINARY",
            reg_type::DWORD => "REG_DWORD",
            reg_type::DWORD_BIG_ENDIAN => "REG_DWORD_BIG_ENDIAN",
            reg_type::LINK => "REG_LINK",
            reg_type::MULTI_SZ => "REG_MULTI_SZ",
            reg_type::QWORD => "REG_QWORD",
            _ => "REG_UNKNOWN",
        }
    }

    /// Short human-readable rendering of the data.
    pub fn display_data(&self) -> String {
        if let Some(s) = self.as_string() {
            return s;
        }
        if let Some(v) = self.as_u32() {
            return format!("0x{v:08x} ({v})");
        }
        if self.kind == reg_type::QWORD && self.data.len() >= 8 {
            let mut b = [0u8; 8];
            b.copy_from_slice(&self.data[..8]);
            let v = u64::from_le_bytes(b);
            return format!("0x{v:016x} ({v})");
        }
        let mut s = String::new();
        for (i, b) in self.data.iter().take(48).enumerate() {
            if i > 0 {
                s.push(' ');
            }
            s.push_str(&format!("{b:02x}"));
        }
        if self.data.len() > 48 {
            s.push_str(&format!(" … ({} bytes)", self.data.len()));
        }
        s
    }

    pub fn display_name(&self) -> &str {
        if self.name.is_empty() {
            "(Default)"
        } else {
            &self.name
        }
    }
}

pub fn encode_utf16z(s: &str) -> Vec<u8> {
    let mut out = Vec::with_capacity(s.len() * 2 + 2);
    for u in s.encode_utf16() {
        out.extend_from_slice(&u.to_le_bytes());
    }
    out.extend_from_slice(&[0, 0]);
    out
}

pub fn decode_utf16z(data: &[u8]) -> String {
    let units: Vec<u16> = data
        .chunks_exact(2)
        .map(|c| u16::from_le_bytes([c[0], c[1]]))
        .collect();
    let end = units.iter().position(|&u| u == 0).unwrap_or(units.len());
    String::from_utf16_lossy(&units[..end])
}

/// Encode strings as REG_MULTI_SZ (each NUL-terminated, plus a final NUL).
pub fn encode_multi_sz(items: &[String]) -> Vec<u8> {
    let mut out = Vec::new();
    for s in items {
        for u in s.encode_utf16() {
            out.extend_from_slice(&u.to_le_bytes());
        }
        out.extend_from_slice(&[0, 0]);
    }
    out.extend_from_slice(&[0, 0]);
    out
}

pub fn decode_multi_sz(data: &[u8]) -> Vec<String> {
    let units: Vec<u16> = data
        .chunks_exact(2)
        .map(|c| u16::from_le_bytes([c[0], c[1]]))
        .collect();
    units
        .split(|&u| u == 0)
        .filter(|s| !s.is_empty())
        .map(String::from_utf16_lossy)
        .collect()
}

/// One registry key and its values (not its subkeys).
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct KeyData {
    /// Full path in original case, e.g. `HKLM\SOFTWARE\Vendor\App`.
    pub path: String,
    /// Values keyed by folded name.
    pub values: BTreeMap<String, RegValue>,
}
crate::json_struct!(KeyData, { path, values });

impl KeyData {
    pub fn new(path: &str) -> Self {
        KeyData {
            path: path.to_string(),
            values: BTreeMap::new(),
        }
    }

    pub fn with(mut self, v: RegValue) -> Self {
        self.values.insert(fold(&v.name), v);
        self
    }

    pub fn get(&self, name: &str) -> Option<&RegValue> {
        self.values.get(&fold(name))
    }

    pub fn get_string(&self, name: &str) -> Option<String> {
        self.get(name)
            .and_then(|v| v.as_string())
            .filter(|s| !s.trim().is_empty())
    }

    pub fn get_u32(&self, name: &str) -> Option<u32> {
        self.get(name).and_then(|v| v.as_u32())
    }
}

/// Metadata for a file or directory captured in a snapshot.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileEntry {
    pub path: String,
    pub is_dir: bool,
    pub size: u64,
    /// Last-write time, seconds since the Unix epoch.
    pub mtime: i64,
}
crate::json_struct!(FileEntry, { path, is_dir, size, mtime });

/// A run of text or bytes in one of a [`Snapshot`]'s pools.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Span {
    pub off: u32,
    pub len: u32,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct FileRec {
    pub path: Span,
    pub size: u64,
    pub mtime: i64,
    pub is_dir: bool,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct KeyRec {
    pub path: Span,
    /// Its values: `vals[first..first + count]`.
    pub first: u32,
    pub count: u32,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct ValRec {
    pub name: Span,
    pub kind: u32,
    pub data: Span,
}

/// Compare two paths or names the way Windows does (case-insensitively),
/// without allocating.
pub fn cmp_fold(a: &str, b: &str) -> std::cmp::Ordering {
    if a.is_ascii() && b.is_ascii() {
        let (x, y) = (a.as_bytes(), b.as_bytes());
        for (p, q) in x.iter().zip(y) {
            let (p, q) = (p.to_ascii_lowercase(), q.to_ascii_lowercase());
            if p != q {
                return p.cmp(&q);
            }
        }
        return x.len().cmp(&y.len());
    }
    a.chars()
        .flat_map(char::to_lowercase)
        .cmp(b.chars().flat_map(char::to_lowercase))
}

/// A point-in-time capture of the parts of the system InstallScope watches.
///
/// Stored as arenas: every path and value name lives in one text pool and
/// every value's data in one byte pool, referenced by fixed-size records,
/// so a capture of millions of entries costs a handful of allocations
/// rather than several per entry. [`Snapshot::finish`] sorts the records
/// (case-insensitively) so two captures compare in one linear merge.
#[derive(Clone, Debug, Default)]
pub struct Snapshot {
    pub created: String,
    pub computer: String,
    pub user: String,
    /// Things that could not be read (access denied etc.).
    pub warnings: Vec<String>,
    pub(crate) text: String,
    pub(crate) bytes: Vec<u8>,
    pub(crate) files: Vec<FileRec>,
    pub(crate) keys: Vec<KeyRec>,
    pub(crate) vals: Vec<ValRec>,
    /// Records are sorted and free of duplicates.
    pub(crate) sorted: bool,
}

/// A file or folder in a [`Snapshot`], borrowed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FileView<'a> {
    pub path: &'a str,
    pub is_dir: bool,
    pub size: u64,
    pub mtime: i64,
}

impl FileView<'_> {
    pub fn to_entry(&self) -> FileEntry {
        FileEntry {
            path: self.path.to_string(),
            is_dir: self.is_dir,
            size: self.size,
            mtime: self.mtime,
        }
    }
}

/// A registry value in a [`Snapshot`], borrowed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ValueView<'a> {
    pub name: &'a str,
    pub kind: u32,
    pub data: &'a [u8],
}

impl ValueView<'_> {
    pub fn to_value(&self) -> RegValue {
        RegValue {
            name: self.name.to_string(),
            kind: self.kind,
            data: self.data.to_vec(),
        }
    }
}

impl Snapshot {
    fn put_text(&mut self, s: &str) -> Span {
        let off = self.text.len();
        self.text.push_str(s);
        Span {
            off: off as u32,
            len: s.len() as u32,
        }
    }

    fn put_bytes(&mut self, b: &[u8]) -> Span {
        let off = self.bytes.len();
        self.bytes.extend_from_slice(b);
        Span {
            off: off as u32,
            len: b.len() as u32,
        }
    }

    pub(crate) fn text_of(&self, s: Span) -> &str {
        &self.text[s.off as usize..(s.off + s.len) as usize]
    }

    pub(crate) fn bytes_of(&self, s: Span) -> &[u8] {
        &self.bytes[s.off as usize..(s.off + s.len) as usize]
    }

    /// Whether the pools still fit 32-bit offsets after adding this much.
    pub(crate) fn has_room(&self, text: usize, bytes: usize) -> bool {
        self.text.len() + text <= u32::MAX as usize && self.bytes.len() + bytes <= u32::MAX as usize
    }

    /// Record a file or folder (copied into the pool; nothing else is
    /// allocated once the pools have grown).
    pub fn push_file(&mut self, path: &str, is_dir: bool, size: u64, mtime: i64) {
        if !self.has_room(path.len(), 0) {
            return;
        }
        let path = self.put_text(path);
        self.files.push(FileRec {
            path,
            size,
            mtime,
            is_dir,
        });
        self.sorted = false;
    }

    /// Start a registry key; the values pushed next belong to it.
    pub fn begin_key(&mut self, path: &str) {
        if !self.has_room(path.len(), 0) {
            return;
        }
        let path = self.put_text(path);
        self.keys.push(KeyRec {
            path,
            first: self.vals.len() as u32,
            count: 0,
        });
        self.sorted = false;
    }

    /// Add a value to the key last started with [`Snapshot::begin_key`].
    pub fn push_value(&mut self, name: &str, kind: u32, data: &[u8]) {
        if self.keys.is_empty() || !self.has_room(name.len(), data.len()) {
            return;
        }
        let name = self.put_text(name);
        let data = self.put_bytes(data);
        self.vals.push(ValRec { name, kind, data });
        if let Some(k) = self.keys.last_mut() {
            k.count += 1;
        }
    }

    pub fn insert_file(&mut self, f: FileEntry) {
        self.push_file(&f.path, f.is_dir, f.size, f.mtime);
    }

    pub fn insert_key(&mut self, k: KeyData) {
        self.begin_key(&k.path);
        for v in k.values.values() {
            self.push_value(&v.name, v.kind, &v.data);
        }
    }

    /// Append another capture's records (from a worker thread).
    pub fn absorb(&mut self, other: Snapshot) {
        let (t, b, v) = (
            self.text.len() as u32,
            self.bytes.len() as u32,
            self.vals.len() as u32,
        );
        if !self.has_room(other.text.len(), other.bytes.len()) {
            self.warnings
                .push("The capture is too large; part of it was left out".into());
            return;
        }
        self.text.push_str(&other.text);
        self.bytes.extend_from_slice(&other.bytes);
        let shift = |s: Span, by: u32| Span {
            off: s.off + by,
            len: s.len,
        };
        self.files.extend(other.files.into_iter().map(|f| FileRec {
            path: shift(f.path, t),
            ..f
        }));
        self.keys.extend(other.keys.into_iter().map(|k| KeyRec {
            path: shift(k.path, t),
            first: k.first + v,
            ..k
        }));
        self.vals.extend(other.vals.into_iter().map(|x| ValRec {
            name: shift(x.name, t),
            kind: x.kind,
            data: shift(x.data, b),
        }));
        self.warnings.extend(other.warnings);
        self.sorted = false;
    }

    /// Sort the records case-insensitively and drop duplicates (the later
    /// capture of the same path wins). Comparing needs this; capture and
    /// loading do it.
    pub fn finish(&mut self) {
        if self.sorted {
            return;
        }
        let text = std::mem::take(&mut self.text);
        let at = |s: Span| &text[s.off as usize..(s.off + s.len) as usize];
        // Stable sorts keep insertion order among equal paths, so keeping
        // the last of each run keeps the latest capture.
        self.files.sort_by(|a, b| cmp_fold(at(a.path), at(b.path)));
        dedup_keep_last(&mut self.files, |a, b| {
            cmp_fold(at(a.path), at(b.path)).is_eq()
        });
        self.keys.sort_by(|a, b| cmp_fold(at(a.path), at(b.path)));
        dedup_keep_last(&mut self.keys, |a, b| {
            cmp_fold(at(a.path), at(b.path)).is_eq()
        });
        for k in &self.keys {
            let r = k.first as usize..(k.first + k.count) as usize;
            self.vals[r].sort_by(|a, b| cmp_fold(at(a.name), at(b.name)));
        }
        self.text = text;
        self.sorted = true;
    }

    pub fn file_count(&self) -> usize {
        self.files.len()
    }

    pub fn key_count(&self) -> usize {
        self.keys.len()
    }

    pub fn value_count(&self) -> usize {
        self.keys.iter().map(|k| k.count as usize).sum()
    }

    pub fn file(&self, i: usize) -> FileView<'_> {
        let f = &self.files[i];
        FileView {
            path: self.text_of(f.path),
            is_dir: f.is_dir,
            size: f.size,
            mtime: f.mtime,
        }
    }

    pub fn files(&self) -> impl Iterator<Item = FileView<'_>> + '_ {
        (0..self.files.len()).map(|i| self.file(i))
    }

    pub fn key_path(&self, i: usize) -> &str {
        self.text_of(self.keys[i].path)
    }

    pub fn key_values(&self, i: usize) -> impl Iterator<Item = ValueView<'_>> + '_ {
        let k = self.keys[i];
        (k.first..k.first + k.count).map(move |j| {
            let v = &self.vals[j as usize];
            ValueView {
                name: self.text_of(v.name),
                kind: v.kind,
                data: self.bytes_of(v.data),
            }
        })
    }

    pub fn key_data(&self, i: usize) -> KeyData {
        let mut k = KeyData::new(self.key_path(i));
        for v in self.key_values(i) {
            k.values.insert(fold(v.name), v.to_value());
        }
        k
    }

    /// Index of a file by path (case-insensitive). Needs [`Snapshot::finish`].
    pub fn find_file(&self, path: &str) -> Option<usize> {
        debug_assert!(self.sorted);
        self.files
            .binary_search_by(|f| cmp_fold(self.text_of(f.path), path))
            .ok()
    }

    /// Index of a key by path (case-insensitive). Needs [`Snapshot::finish`].
    pub fn find_key(&self, path: &str) -> Option<usize> {
        debug_assert!(self.sorted);
        self.keys
            .binary_search_by(|k| cmp_fold(self.text_of(k.path), path))
            .ok()
    }
}

/// Remove consecutive duplicates, keeping the last of each run.
fn dedup_keep_last<T: Copy>(v: &mut Vec<T>, same: impl Fn(&T, &T) -> bool) {
    if v.len() < 2 {
        return;
    }
    let mut w = 0usize;
    for r in 0..v.len() {
        if r + 1 < v.len() && same(&v[r], &v[r + 1]) {
            continue;
        }
        v[w] = v[r];
        w += 1;
    }
    v.truncate(w);
}

/// A registry value located in a specific key.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ValueRef {
    pub key: String,
    pub value: RegValue,
}
crate::json_struct!(ValueRef, { key, value });

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ValueChange {
    pub key: String,
    pub before: RegValue,
    pub after: RegValue,
}
crate::json_struct!(ValueChange, { key, before, after });

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileChange {
    pub before: FileEntry,
    pub after: FileEntry,
}
crate::json_struct!(FileChange, { before, after });

/// Everything that differs between two snapshots.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ChangeSet {
    /// New keys, with the values they contain.
    pub keys_added: Vec<KeyData>,
    /// Keys that disappeared, with the values they had.
    pub keys_removed: Vec<KeyData>,
    /// New values in keys that already existed.
    pub values_added: Vec<ValueRef>,
    /// Values removed from keys that still exist.
    pub values_removed: Vec<ValueRef>,
    pub values_modified: Vec<ValueChange>,
    pub files_added: Vec<FileEntry>,
    pub files_removed: Vec<FileEntry>,
    pub files_modified: Vec<FileChange>,
}
crate::json_struct!(ChangeSet, { keys_added, keys_removed, values_added, values_removed, values_modified, files_added, files_removed, files_modified });

impl ChangeSet {
    pub fn is_empty(&self) -> bool {
        self.total() == 0
    }

    pub fn total(&self) -> usize {
        self.keys_added.len()
            + self.keys_removed.len()
            + self.values_added.len()
            + self.values_removed.len()
            + self.values_modified.len()
            + self.files_added.len()
            + self.files_removed.len()
            + self.files_modified.len()
    }

    pub fn stats(&self) -> ChangeStats {
        ChangeStats {
            keys_added: self.keys_added.len(),
            keys_removed: self.keys_removed.len(),
            values_added: self.values_added.len()
                + self
                    .keys_added
                    .iter()
                    .map(|k| k.values.len())
                    .sum::<usize>(),
            values_removed: self.values_removed.len(),
            values_modified: self.values_modified.len(),
            files_added: self.files_added.iter().filter(|f| !f.is_dir).count(),
            dirs_added: self.files_added.iter().filter(|f| f.is_dir).count(),
            files_removed: self.files_removed.len(),
            files_modified: self.files_modified.len(),
            bytes_added: self.files_added.iter().map(|f| f.size).sum(),
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ChangeStats {
    pub keys_added: usize,
    pub keys_removed: usize,
    pub values_added: usize,
    pub values_removed: usize,
    pub values_modified: usize,
    pub files_added: usize,
    pub dirs_added: usize,
    pub files_removed: usize,
    pub files_modified: usize,
    pub bytes_added: u64,
}
crate::json_struct!(ChangeStats, { keys_added, keys_removed, values_added, values_removed, values_modified, files_added, dirs_added, files_removed, files_modified, bytes_added });

/// What the live watcher observed while an install was being monitored.
/// This supplements the final snapshot diff; it is never a completeness claim.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MonitoringCoverage {
    pub tracking_started: String,
    pub tracking_ended: String,
    pub file_roots: Vec<String>,
    pub registry_roots: Vec<String>,
    pub active_file_roots: Vec<String>,
    pub active_registry_roots: Vec<String>,
    pub warnings: Vec<String>,
    pub file_buffer_overflows: u64,
    pub distinct_path_counts_limited: bool,
    pub file_notifications: u64,
    pub registry_notifications: u64,
    pub added_paths: u64,
    pub changed_paths: u64,
    pub removed_paths: u64,
    pub registry_paths: u64,
}
crate::json_struct!(MonitoringCoverage, default, {
    tracking_started, tracking_ended, file_roots, registry_roots,
    active_file_roots, active_registry_roots, warnings,
    file_buffer_overflows, distinct_path_counts_limited, file_notifications, registry_notifications,
    added_paths, changed_paths, removed_paths, registry_paths
});

/// A saved record of everything one installation changed: the heart of a
/// monitored program. Reverting it is how InstallScope uninstalls precisely.
#[derive(Clone, Debug, Default)]
pub struct InstallLog {
    pub id: String,
    pub name: String,
    pub publisher: String,
    pub version: String,
    pub created: String,
    /// Installer that was run under monitoring, if any.
    pub installer: String,
    /// The program's own Uninstall registry key, if one was created.
    pub uninstall_key: String,
    pub notes: String,
    pub group: String,
    pub tags: Vec<String>,
    /// Supplemental live-watcher coverage diagnostics. The final diff in
    /// `changes` remains authoritative if live notifications were incomplete.
    pub monitoring: MonitoringCoverage,
    pub changes: ChangeSet,
}
crate::json_struct!(InstallLog, { id, name, publisher [default], version [default], created, installer [default], uninstall_key [default], notes [default], group [default], tags [skip_empty], monitoring [skip_default], changes });

impl InstallLog {
    /// Whether the log matches a search: name, publisher, group or a tag.
    pub fn matches(&self, query_lower: &str) -> bool {
        query_lower.is_empty()
            || self.name.to_lowercase().contains(query_lower)
            || self.publisher.to_lowercase().contains(query_lower)
            || self.group.to_lowercase().contains(query_lower)
            || self
                .tags
                .iter()
                .any(|t| t.to_lowercase().contains(query_lower))
    }
}

/// Split "a, b ,c" into tidy, unique tags.
pub fn parse_tags(s: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for t in s.split([',', ';']) {
        let t = t.trim();
        if !t.is_empty() && !out.iter().any(|o| o.eq_ignore_ascii_case(t)) {
            out.push(t.to_string());
        }
    }
    out
}

/// Human-friendly byte size.
pub fn human_size(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut v = bytes as f64;
    let mut i = 0;
    while v >= 1024.0 && i < UNITS.len() - 1 {
        v /= 1024.0;
        i += 1;
    }
    if i == 0 {
        format!("{bytes} B")
    } else {
        format!("{v:.1} {}", UNITS[i])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_roundtrip() {
        let d = vec![0u8, 1, 0xab, 0xff];
        assert_eq!(hexbytes::from_hex(&hexbytes::to_hex(&d)).unwrap(), d);
    }

    #[test]
    fn string_values() {
        let v = RegValue::string("DisplayName", "Foo Bar");
        assert_eq!(v.as_string().unwrap(), "Foo Bar");
        assert_eq!(RegValue::dword("x", 7).as_u32(), Some(7));
    }

    #[test]
    fn sizes() {
        assert_eq!(human_size(512), "512 B");
        assert_eq!(human_size(1536), "1.5 KB");
    }

    #[test]
    fn install_log_coverage_roundtrips_and_older_logs_default_it() {
        let log = InstallLog {
            id: "coverage-test".into(),
            name: "Example".into(),
            monitoring: MonitoringCoverage {
                file_roots: vec![r"C:\Apps".into()],
                active_file_roots: vec![r"C:\Apps".into()],
                file_buffer_overflows: 2,
                added_paths: 3,
                file_notifications: 8,
                ..Default::default()
            },
            ..Default::default()
        };
        use crate::json::ToJson;
        let encoded = crate::json::to_json_string(&log);
        let restored: InstallLog = crate::json::from_json_str(&encoded).unwrap();
        assert_eq!(restored.monitoring.file_buffer_overflows, 2);
        assert_eq!(restored.monitoring.added_paths, 3);
        assert_eq!(restored.monitoring.file_notifications, 8);

        let mut legacy_value = log.to_json();
        if let crate::json::Value::Object(fields) = &mut legacy_value {
            fields.retain(|(key, _)| key != "monitoring");
        }
        let legacy = crate::json::to_string(&legacy_value);
        let restored: InstallLog = crate::json::from_json_str(&legacy).unwrap();
        assert!(restored.monitoring.file_roots.is_empty());
        assert_eq!(restored.monitoring.file_buffer_overflows, 0);
    }
}

#[cfg(test)]
mod tag_tests {
    use super::*;

    #[test]
    fn tags_parse_and_match() {
        assert_eq!(parse_tags(" work, Dev ;work,, dev "), vec!["work", "Dev"]);
        let log = InstallLog {
            name: "Photo Editor".into(),
            group: "Creative".into(),
            tags: vec!["trial".into()],
            ..Default::default()
        };
        assert!(log.matches("creat"));
        assert!(log.matches("tria"));
        assert!(log.matches(""));
        assert!(!log.matches("office"));
    }
}
