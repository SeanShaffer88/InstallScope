//! Same-directory staged writes. Readers see the old complete file or the new
//! complete file. This does not merge concurrent updates or establish a security
//! boundary against a process changing parent paths during the operation.
use std::{
    fs::{self, OpenOptions},
    io::{self, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

pub(crate) fn check_path(path: &Path) -> io::Result<()> {
    for part in path.ancestors() {
        if part.as_os_str().is_empty() {
            continue;
        }
        match fs::symlink_metadata(part) {
            Ok(meta) => {
                let mut linked = meta.file_type().is_symlink();
                #[cfg(windows)]
                {
                    use std::os::windows::fs::MetadataExt;
                    linked |= meta.file_attributes() & 0x400 != 0;
                }
                if linked {
                    return Err(io::Error::other(format!(
                        "Linked/reparse data paths are unsupported: {}",
                        part.display()
                    )));
                }
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => return Err(e),
        }
    }
    Ok(())
}
struct Staged(PathBuf);
impl Drop for Staged {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}
pub(crate) fn write(path: &Path, bytes: &[u8], replace: bool) -> io::Result<()> {
    write_with(path, replace, |file| file.write_all(bytes))
}

/// Stream to a unique staged file; failed generation never publishes partial data.
pub(crate) fn write_with(path: &Path, replace: bool, generate: impl FnOnce(&mut fs::File) -> io::Result<()>) -> io::Result<()> {
    check_path(path)?;
    let parent = path
        .parent()
        .ok_or_else(|| io::Error::other("Data file has no parent"))?;
    // Serialize publishers from different app instances, with a bounded wait.
    // The OS releases this lock on process exit; the persistent file is harmless.
    let lock_path = parent.join(".setupscope-write.lock");
    check_path(&lock_path)?;
    let lock = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(lock_path)?;
    let mut acquired = false;
    // Durable flushes on busy/virtual disks can exceed 200 ms. Give competing
    // publishers a bounded ten-second window instead of failing normal saves.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        match lock.try_lock() {
            Ok(()) => {
                acquired = true;
                break;
            }
            Err(fs::TryLockError::WouldBlock) => {
                if std::time::Instant::now() >= deadline {
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(5))
            }
            Err(fs::TryLockError::Error(e)) => return Err(e),
        }
    }
    if !acquired {
        return Err(io::Error::new(
            io::ErrorKind::WouldBlock,
            "Another data write is active; wait and retry",
        ));
    }
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let (staged, mut file) = (0..128)
        .find_map(|_| {
            let name = format!(
                ".setupscope-{}-{}.tmp",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            );
            let temp = parent.join(name);
            match OpenOptions::new().write(true).create_new(true).open(&temp) {
                Ok(file) => Some(Ok((Staged(temp), file))),
                Err(e) if e.kind() == io::ErrorKind::AlreadyExists => None,
                Err(e) => Some(Err(e)),
            }
        })
        .unwrap_or_else(|| {
            Err(io::Error::other(
                "Cannot allocate a staging file; preserve existing temporary files and retry",
            ))
        })?;
    generate(&mut file)?;
    file.sync_all()?;
    drop(file);
    check_path(path)?;
    publish(&staged.0, path, replace)
}
#[cfg(windows)]
fn publish(from: &Path, to: &Path, replace: bool) -> io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{
        MoveFileExW, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH,
    };
    let from: Vec<_> = from.as_os_str().encode_wide().chain(Some(0)).collect();
    let to: Vec<_> = to.as_os_str().encode_wide().chain(Some(0)).collect();
    let flags = MOVEFILE_WRITE_THROUGH
        | if replace {
            MOVEFILE_REPLACE_EXISTING
        } else {
            0
        };
    for attempt in 0..40 {
        if unsafe { MoveFileExW(from.as_ptr(), to.as_ptr(), flags) } != 0 {
            return Ok(());
        }
        let error = io::Error::last_os_error();
        // Windows can briefly retain an old destination opened by a reader.
        // Retry sharing/delete-pending errors; never remove the destination.
        if attempt == 39 || !matches!(error.raw_os_error(), Some(5 | 32 | 33)) {
            return Err(error);
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    unreachable!()
}
#[cfg(not(windows))]
fn publish(from: &Path, to: &Path, replace: bool) -> io::Result<()> {
    if replace {
        fs::rename(from, to)
    } else {
        fs::hard_link(from, to)
    }
}

#[cfg(test)]
mod audit_tests {
    use super::*;
    #[test]
    fn failed_stream_keeps_original_and_cleans_partial_stage() {
        let temp = crate::util::tempdir().unwrap();
        let path = temp.path().join("evidence.bin");
        fs::write(&path, b"original").unwrap();
        let result = write_with(&path, true, |file| {
            file.write_all(b"partial")?;
            Err(io::Error::other("simulated encoder failure"))
        });
        assert!(result.is_err());
        assert_eq!(fs::read(&path).unwrap(), b"original");
        assert!(!fs::read_dir(temp.path()).unwrap().any(|e| e.unwrap().path().extension().is_some_and(|ext| ext == "tmp")));
    }
}
