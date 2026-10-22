//! Optional, bounded handle metadata; never reads file contents or changes files.
use std::collections::HashMap;
use std::path::Path;

pub const IDENTITY_LIMIT: usize = 100_000;
pub const QUERY_LIMIT: u64 = 200_000;

#[derive(Clone, Debug, Default)]
pub struct Summary {
    pub unique_logical_bytes: u64,
    pub allocated_bytes: u64,
    pub unique_files: u64,
    pub repeated_links: u64,
    pub unknown_files: u64,
    pub limit_reached: bool,
    pub overflow: bool,
    pub unknown_details: Vec<String>,
}
impl Summary {
    pub fn partial(&self) -> bool {
        self.unknown_files != 0 || self.overflow || self.limit_reached
    }
}
#[derive(Hash, PartialEq, Eq, Clone, Copy)]
struct Identity(u64, [u8; 16]);
#[derive(Default)]
pub(crate) struct Accounting {
    pub summary: Summary,
    seen: HashMap<Identity, (u64, u64)>,
    queries: u64,
}
impl Accounting {
    pub fn observe(&mut self, path: &Path, logical: u64) {
        if self.queries >= QUERY_LIMIT {
            self.summary.limit_reached = true;
            self.summary.unknown_files += 1;
            return;
        }
        self.queries += 1;
        match metadata(path, logical) {
            Ok((id, allocation)) => self.record(id, logical, allocation),
            Err(error) => {
                self.summary.unknown_files += 1;
                if self.summary.unknown_details.len() < 10 {
                    self.summary
                        .unknown_details
                        .push(format!("{}: {error}", path.display()));
                }
            }
        }
    }
    fn record(&mut self, id: Identity, logical: u64, allocation: u64) {
        if id.1 == [0; 16] {
            self.summary.unknown_files += 1;
            return;
        }
        if let Some(previous) = self.seen.get(&id) {
            if *previous == (logical, allocation) {
                self.summary.repeated_links += 1;
            } else {
                self.summary.unknown_files += 1;
            }
            return;
        }
        if self.seen.len() >= IDENTITY_LIMIT {
            self.summary.limit_reached = true;
            self.summary.unknown_files += 1;
            return;
        }
        self.seen.insert(id, (logical, allocation));
        self.summary.unique_files += 1;
        match (
            self.summary.unique_logical_bytes.checked_add(logical),
            self.summary.allocated_bytes.checked_add(allocation),
        ) {
            (Some(logical), Some(allocated)) => {
                self.summary.unique_logical_bytes = logical;
                self.summary.allocated_bytes = allocated;
            }
            _ => {
                self.summary.overflow = true;
                self.summary.unknown_files += 1;
            }
        }
    }
}

#[cfg(windows)]
fn metadata(path: &Path, expected_length: u64) -> std::io::Result<(Identity, u64)> {
    use std::os::windows::{fs::OpenOptionsExt, io::AsRawHandle};
    use windows_sys::Win32::Storage::FileSystem::*;
    let invalid = || std::io::Error::other("Unsupported, linked or changing allocation metadata");
    // Refuse UNC/device/mapped-network inputs even when logical scanning was
    // explicitly requested there. Only local drive metadata is supported here.
    let text = path.to_str().ok_or_else(invalid)?;
    let drive_path = text.strip_prefix("\\\\?\\").unwrap_or(text);
    let b = drive_path.as_bytes();
    if b.len() < 3 || !b[0].is_ascii_alphabetic() || &b[1..3] != b":\\" {
        return Err(invalid());
    }
    let drive: Vec<u16> = drive_path[..3].encode_utf16().chain(Some(0)).collect();
    if unsafe { GetDriveTypeW(drive.as_ptr()) } == 4 {
        return Err(invalid());
    }
    let file = std::fs::OpenOptions::new()
        .access_mode(FILE_READ_ATTRIBUTES)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT | FILE_FLAG_OPEN_NO_RECALL)
        .open(path)?;
    unsafe {
        let handle = file.as_raw_handle();
        let mut tags: FILE_ATTRIBUTE_TAG_INFO = std::mem::zeroed();
        let mut standard: FILE_STANDARD_INFO = std::mem::zeroed();
        let mut id: FILE_ID_INFO = std::mem::zeroed();
        if GetFileInformationByHandleEx(
            handle,
            FileAttributeTagInfo,
            (&mut tags as *mut FILE_ATTRIBUTE_TAG_INFO).cast(),
            std::mem::size_of_val(&tags) as u32,
        ) == 0
            || tags.FileAttributes & FILE_ATTRIBUTE_REPARSE_POINT != 0
        {
            return Err(invalid());
        }
        if GetFileInformationByHandleEx(
            handle,
            FileStandardInfo,
            (&mut standard as *mut FILE_STANDARD_INFO).cast(),
            std::mem::size_of_val(&standard) as u32,
        ) == 0
            || GetFileInformationByHandleEx(
                handle,
                FileIdInfo,
                (&mut id as *mut FILE_ID_INFO).cast(),
                std::mem::size_of_val(&id) as u32,
            ) == 0
        {
            return Err(std::io::Error::last_os_error());
        }
        if standard.Directory != 0
            || standard.DeletePending != 0
            || standard.AllocationSize < 0
            || standard.EndOfFile < 0
            || standard.EndOfFile as u64 != expected_length
        {
            return Err(invalid());
        }
        Ok((
            Identity(id.VolumeSerialNumber, id.FileId.Identifier),
            standard.AllocationSize as u64,
        ))
    }
}
#[cfg(not(windows))]
fn metadata(_path: &Path, _expected_length: u64) -> std::io::Result<(Identity, u64)> {
    Err(std::io::Error::other(
        "Windows allocation metadata unavailable",
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn id(volume: u64, file: u8) -> Identity {
        Identity(volume, [file; 16])
    }
    #[test]
    fn deduplicates_across_folders_but_not_volumes() {
        let mut a = Accounting::default();
        a.record(id(1, 1), 30, 4096);
        a.record(id(1, 1), 30, 4096);
        a.record(id(2, 1), 30, 4096);
        assert_eq!(
            (
                a.summary.unique_files,
                a.summary.repeated_links,
                a.summary.unique_logical_bytes,
                a.summary.allocated_bytes
            ),
            (2, 1, 60, 8192)
        );
        assert!(!a.summary.partial());
    }
    #[test]
    fn changed_or_unspecified_identity_is_unknown() {
        let mut a = Accounting::default();
        a.record(id(1, 0), 1, 1);
        a.record(id(1, 1), 10, 10);
        a.record(id(1, 1), 20, 20);
        assert_eq!(a.summary.unknown_files, 2);
        assert!(a.summary.partial());
        assert_eq!(a.summary.unique_logical_bytes, 10);
    }
    #[test]
    fn limits_and_overflow_are_explicit() {
        let mut a = Accounting {
            queries: QUERY_LIMIT,
            ..Default::default()
        };
        a.observe(Path::new("not-read"), 5);
        assert!(a.summary.limit_reached && a.summary.partial());
        a.record(id(1, 1), u64::MAX, 1);
        a.record(id(1, 2), 1, 1);
        assert!(a.summary.overflow);
        assert_eq!(a.summary.unique_logical_bytes, u64::MAX);
    }
    #[test]
    fn identity_memory_is_bounded_and_missing_metadata_stays_unknown() {
        let mut a = Accounting::default();
        for index in 1..=IDENTITY_LIMIT + 1 {
            a.record(Identity(1, (index as u128).to_le_bytes()), 1, 1);
        }
        assert_eq!(a.seen.len(), IDENTITY_LIMIT);
        assert!(a.summary.limit_reached);
        assert_eq!(a.summary.unknown_files, 1);
        let mut a = Accounting::default();
        let tmp = crate::util::tempdir().unwrap();
        for _ in 0..20 {
            a.observe(&tmp.path().join("absent"), 1);
        }
        assert_eq!(a.summary.unknown_files, 20);
        assert_eq!(a.summary.unknown_details.len(), 10);
        assert_eq!(a.summary.unique_files, 0);
    }
    #[cfg(windows)]
    #[test]
    fn real_files_and_hardlinks_are_read_only() {
        let tmp = crate::util::tempdir().unwrap();
        let source = tmp.path().join("original");
        std::fs::write(&source, vec![7u8; 8192]).unwrap();
        std::fs::create_dir(tmp.path().join("nested")).unwrap();
        let link = tmp.path().join("nested/linked");
        std::fs::hard_link(&source, &link).unwrap();
        let mut a = Accounting::default();
        a.observe(&source, 8192);
        a.observe(&link, 8192);
        assert_eq!(
            (
                a.summary.unique_files,
                a.summary.repeated_links,
                a.summary.unknown_files
            ),
            (1, 1, 0)
        );
        assert_eq!(a.summary.unique_logical_bytes, 8192);
        assert_eq!(std::fs::read(&link).unwrap(), vec![7u8; 8192]);
        assert!(metadata(&source, 1).is_err());
    }
}
