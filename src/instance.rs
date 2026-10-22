//! Session-wide GUI lifetime gate, shared by all copies of the executable.
#[cfg(windows)]
mod windows {
    use windows_sys::Win32::{
        Foundation::{CloseHandle, GetLastError, ERROR_ALREADY_EXISTS, HANDLE},
        System::Threading::CreateMutexW,
        UI::WindowsAndMessaging::{FindWindowW, SetForegroundWindow, ShowWindow, SW_RESTORE},
    };
    pub struct Instance(HANDLE);
    impl Drop for Instance {
        fn drop(&mut self) {
            unsafe {
                CloseHandle(self.0);
            }
        }
    }
    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(Some(0)).collect()
    }
    impl Instance {
        pub fn acquire_named(name: &str) -> std::io::Result<Option<Self>> {
            let name = wide(name);
            let handle = unsafe { CreateMutexW(std::ptr::null(), 0, name.as_ptr()) };
            if handle.is_null() {
                return Err(std::io::Error::last_os_error());
            }
            let exists = unsafe { GetLastError() } == ERROR_ALREADY_EXISTS;
            let guard = Self(handle);
            if exists {
                drop(guard);
                Ok(None)
            } else {
                Ok(Some(guard))
            }
        }
        pub fn acquire() -> std::io::Result<Option<Self>> {
            Self::acquire_named("Local\\SetupScope.Desktop.v1")
        }
    }
    pub fn focus_existing() {
        let title = wide("SetupScope");
        let window = unsafe { FindWindowW(std::ptr::null(), title.as_ptr()) };
        if !window.is_null() {
            unsafe {
                // The board can be hidden while independent tools remain open.
                ShowWindow(window, SW_RESTORE);
                SetForegroundWindow(window);
            }
        }
    }
    #[cfg(test)]
    mod tests {
        use super::*;
        #[test]
        fn gate_blocks_duplicate_and_releases_after_exit() {
            let name = format!("Local\\SetupScope.Test.{}", setupscope_core::util::new_id());
            let first = Instance::acquire_named(&name).unwrap().unwrap();
            assert!(Instance::acquire_named(&name).unwrap().is_none());
            drop(first);
            assert!(Instance::acquire_named(&name).unwrap().is_some());
        }
    }
}
#[cfg(windows)]
pub use windows::*;
