//! Fixed HKCU\Environment adapter. No machine-wide key or command execution.
use super::{name, raw_text, Environment, RegValue};
use anyhow::{ensure, Result};
use windows_sys::Win32::{System::Registry::*, UI::WindowsAndMessaging::*};

pub struct UserEnvironment;
struct Key(HKEY);
impl Drop for Key {
    fn drop(&mut self) {
        unsafe { RegCloseKey(self.0) };
    }
}
fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(Some(0)).collect()
}
fn open(access: u32) -> Result<Option<Key>> {
    let mut key = std::ptr::null_mut();
    let rc = unsafe {
        RegOpenKeyExW(
            HKEY_CURRENT_USER,
            wide("Environment").as_ptr(),
            0,
            access | KEY_WOW64_64KEY,
            &mut key,
        )
    };
    if rc == 2 {
        return Ok(None);
    }
    ensure!(rc == 0, "Cannot open HKCU Environment (Windows error {rc})");
    Ok(Some(Key(key)))
}
impl Environment for UserEnvironment {
    fn acquire(&self) -> Result<Box<dyn std::any::Any>> {
        use windows_sys::Win32::{
            Foundation::{CloseHandle, WAIT_ABANDONED, WAIT_OBJECT_0},
            System::Threading::{CreateMutexW, ReleaseMutex, WaitForSingleObject},
        };
        struct Guard(windows_sys::Win32::Foundation::HANDLE);
        impl Drop for Guard {
            fn drop(&mut self) {
                unsafe {
                    ReleaseMutex(self.0);
                    CloseHandle(self.0)
                };
            }
        }
        let id = crate::regression::identity_marker()?;
        let handle = unsafe {
            CreateMutexW(
                std::ptr::null(),
                0,
                wide(&format!("Local\\SetupScope-Environment-v1-{id}")).as_ptr(),
            )
        };
        ensure!(
            !handle.is_null(),
            "Cannot create the environment operation mutex"
        );
        let result = unsafe { WaitForSingleObject(handle, 0) };
        if !matches!(result, WAIT_OBJECT_0 | WAIT_ABANDONED) {
            unsafe { CloseHandle(handle) };
            anyhow::bail!("Another SetupScope environment change is active; wait and reload");
        }
        Ok(Box::new(Guard(handle)))
    }
    fn read(&self, requested: &str) -> Result<Option<RegValue>> {
        let requested = name(requested)?;
        let Some(key) = open(KEY_QUERY_VALUE)? else {
            return Ok(None);
        };
        let mut buffer = vec![0u16; 16385];
        let mut actual = None;
        for index in 0..4096 {
            let mut length = buffer.len() as u32;
            let rc = unsafe {
                RegEnumValueW(
                    key.0,
                    index,
                    buffer.as_mut_ptr(),
                    &mut length,
                    std::ptr::null(),
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                )
            };
            if rc == 259 {
                return Ok(None);
            }
            ensure!(
                rc == 0,
                "Cannot enumerate user environment names (Windows error {rc})"
            );
            let candidate = String::from_utf16(&buffer[..length as usize])?;
            if candidate.eq_ignore_ascii_case(requested) {
                actual = Some(candidate);
                break;
            }
        }
        let actual = actual.ok_or_else(|| {
            anyhow::anyhow!("User environment exceeds the 4096-name inspection limit")
        })?;
        let (mut kind, mut length) = (0u32, 0u32);
        let variable = wide(&actual);
        let rc = unsafe {
            RegQueryValueExW(
                key.0,
                variable.as_ptr(),
                std::ptr::null(),
                &mut kind,
                std::ptr::null_mut(),
                &mut length,
            )
        };
        if rc == 2 {
            return Ok(None);
        }
        ensure!(
            rc == 0 && length <= 64 * 1024,
            "Cannot read user variable or value exceeds 64 KiB (Windows error {rc})"
        );
        let mut data = vec![0u8; length as usize];
        let rc = unsafe {
            RegQueryValueExW(
                key.0,
                variable.as_ptr(),
                std::ptr::null(),
                &mut kind,
                data.as_mut_ptr(),
                &mut length,
            )
        };
        ensure!(
            rc == 0 && length as usize <= data.len(),
            "User variable changed or could not be read (Windows error {rc}); preview again"
        );
        data.truncate(length as usize);
        let value = RegValue {
            name: actual,
            kind,
            data,
        };
        raw_text(&value)?;
        Ok(Some(value))
    }
    fn write(&self, requested: &str, value: Option<&RegValue>) -> Result<()> {
        let requested = name(requested)?;
        let key=open(KEY_SET_VALUE)?.ok_or_else(||anyhow::anyhow!("HKCU Environment does not exist; initialize it through Windows Environment Variables settings first. No key was created."))?;
        let rc = if let Some(value) = value {
            ensure!(
                value.name.eq_ignore_ascii_case(requested),
                "Variable name mismatch"
            );
            raw_text(value)?;
            unsafe {
                RegSetValueExW(
                    key.0,
                    wide(&value.name).as_ptr(),
                    0,
                    value.kind,
                    value.data.as_ptr(),
                    value.data.len() as u32,
                )
            }
        } else {
            unsafe { RegDeleteValueW(key.0, wide(requested).as_ptr()) }
        };
        ensure!(
            rc == 0 || (value.is_none() && rc == 2),
            "Cannot update user environment (Windows error {rc})"
        );
        Ok(())
    }
    fn flush(&self) -> Result<()> {
        // One flush for the transaction, rather than blocking the user hive for
        // every value. RegFlushKey explicitly requires KEY_QUERY_VALUE access.
        let key = open(KEY_QUERY_VALUE)?
            .ok_or_else(|| anyhow::anyhow!("User environment key disappeared before flush"))?;
        let rc = unsafe { RegFlushKey(key.0) };
        ensure!(rc==0,"User value was written but flush failed (Windows error {rc}); preserve the transaction receipt and review recovery");
        Ok(())
    }
    fn notify(&self) -> String {
        let value = wide("Environment");
        let mut result = 0;
        let ok = unsafe {
            SendMessageTimeoutW(
                HWND_BROADCAST,
                WM_SETTINGCHANGE,
                0,
                value.as_ptr() as isize,
                SMTO_ABORTIFHUNG,
                250,
                &mut result,
            )
        };
        if ok == 0 {
            "Values saved. Environment notification was incomplete or timed out; restart terminals/IDEs. Existing processes may retain old values.".into()
        } else {
            "Values saved and change notification sent. Restart terminals/IDEs that retain old values; PATH ordering still depends on Windows and the launching process.".into()
        }
    }
}
