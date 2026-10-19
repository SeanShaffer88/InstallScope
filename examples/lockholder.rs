//! Test helper for InstallScope's locked-file tests; not part of InstallScope. It
//! holds files the way real programs do:
//!
//!   lockholder open <file> <seconds>      keep a file open, sharing nothing
//!   lockholder load <dll> <seconds>       keep a DLL loaded
//!   lockholder service <name> <file>      run as the Windows service <name>
//!                                         with <file> open until stopped
//!
//! It prints "ready" once the file is held. As a service it reports
//! "stopped" while still holding the file and exits two seconds later, as
//! slow real services do, so a removal must wait for the process to exit.

fn main() {
    #[cfg(windows)]
    win::main();
    #[cfg(not(windows))]
    eprintln!("lockholder only runs on Windows");
}

#[cfg(windows)]
mod win {
    use std::ffi::c_void;
    use std::fs::{File, OpenOptions};
    use std::io::Write;
    use std::os::windows::fs::OpenOptionsExt;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::OnceLock;
    use std::time::Duration;
    use windows_sys::Win32::System::LibraryLoader::LoadLibraryW;
    use windows_sys::Win32::System::Services::{
        RegisterServiceCtrlHandlerExW, SetServiceStatus, StartServiceCtrlDispatcherW,
        SERVICE_STATUS, SERVICE_STATUS_HANDLE, SERVICE_TABLE_ENTRYW,
    };

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    fn open_exclusive(path: &str) -> File {
        OpenOptions::new()
            .read(true)
            .write(true)
            .share_mode(0)
            .open(path)
            .unwrap_or_else(|e| {
                eprintln!("open {path}: {e}");
                std::process::exit(2)
            })
    }

    fn ready_then_wait(secs: &str) {
        println!("ready");
        let _ = std::io::stdout().flush();
        std::thread::sleep(Duration::from_secs(secs.parse().unwrap_or(60)));
    }

    pub fn main() {
        let a: Vec<String> = std::env::args().collect();
        let arg = |i: usize| a.get(i).cloned().unwrap_or_default();
        match arg(1).as_str() {
            "open" => {
                let _f = open_exclusive(&arg(2));
                ready_then_wait(&arg(3));
            }
            "load" => {
                let w = wide(&arg(2));
                let h = unsafe { LoadLibraryW(w.as_ptr()) };
                if h.is_null() {
                    eprintln!(
                        "LoadLibrary {}: {}",
                        arg(2),
                        std::io::Error::last_os_error()
                    );
                    std::process::exit(2);
                }
                ready_then_wait(&arg(3));
            }
            "service" => service(arg(2), arg(3)),
            _ => {
                eprintln!("usage: lockholder open|load <path> <seconds> | service <name> <file>");
                std::process::exit(1);
            }
        }
    }

    // ---- Service mode ----

    static NAME: OnceLock<Vec<u16>> = OnceLock::new();
    static FILE: OnceLock<String> = OnceLock::new();
    static STOP: AtomicBool = AtomicBool::new(false);
    static HANDLE: AtomicUsize = AtomicUsize::new(0);

    const OWN_PROCESS: u32 = 0x10;
    const STOPPED: u32 = 1;
    const RUNNING: u32 = 4;
    const ACCEPT_STOP: u32 = 0x1;
    const CONTROL_STOP: u32 = 1;
    const CONTROL_INTERROGATE: u32 = 4;
    const NO_ERROR: u32 = 0;
    const CALL_NOT_IMPLEMENTED: u32 = 120;

    fn report(state: u32) {
        let st = SERVICE_STATUS {
            dwServiceType: OWN_PROCESS,
            dwCurrentState: state,
            dwControlsAccepted: if state == RUNNING { ACCEPT_STOP } else { 0 },
            dwWin32ExitCode: 0,
            dwServiceSpecificExitCode: 0,
            dwCheckPoint: 0,
            dwWaitHint: 0,
        };
        let h = HANDLE.load(Ordering::SeqCst) as SERVICE_STATUS_HANDLE;
        unsafe { SetServiceStatus(h, &st) };
    }

    unsafe extern "system" fn handler(
        control: u32,
        _event: u32,
        _data: *mut c_void,
        _ctx: *mut c_void,
    ) -> u32 {
        match control {
            CONTROL_STOP => {
                STOP.store(true, Ordering::SeqCst);
                NO_ERROR
            }
            CONTROL_INTERROGATE => NO_ERROR,
            _ => CALL_NOT_IMPLEMENTED,
        }
    }

    unsafe extern "system" fn service_main(_argc: u32, _argv: *mut *mut u16) {
        let name = NAME.get().unwrap();
        let h = RegisterServiceCtrlHandlerExW(name.as_ptr(), Some(handler), std::ptr::null());
        HANDLE.store(h as usize, Ordering::SeqCst);
        let file = open_exclusive(FILE.get().unwrap());
        report(RUNNING);
        while !STOP.load(Ordering::SeqCst) {
            std::thread::sleep(Duration::from_millis(50));
        }
        // Stopped as far as Windows knows, but the file is still open until
        // the process has finished exiting.
        report(STOPPED);
        std::thread::sleep(Duration::from_secs(2));
        drop(file);
    }

    fn service(name: String, file: String) {
        let mut w = wide(&name);
        NAME.set(w.clone()).unwrap();
        FILE.set(file).unwrap();
        let table = [
            SERVICE_TABLE_ENTRYW {
                lpServiceName: w.as_mut_ptr(),
                lpServiceProc: Some(service_main),
            },
            SERVICE_TABLE_ENTRYW {
                lpServiceName: std::ptr::null_mut(),
                lpServiceProc: None,
            },
        ];
        if unsafe { StartServiceCtrlDispatcherW(table.as_ptr()) } == 0 {
            eprintln!(
                "not started by the service manager: {}",
                std::io::Error::last_os_error()
            );
            std::process::exit(3);
        }
    }
}
