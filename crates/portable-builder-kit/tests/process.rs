#![cfg(feature = "test-fixture")]
use portable_builder_kit::process::*;
use std::{
    ffi::OsString,
    fs,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
    time::{Duration, Instant},
};
struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let p = std::env::temp_dir().join(format!(
            "pafkit-process-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&p).unwrap();
        Self(p)
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn request(args: &[&std::ffi::OsStr]) -> ToolRequest {
    ToolRequest {
        executable: PathBuf::from(env!("CARGO_BIN_EXE_pafkit-test-tool")),
        arguments: args.iter().map(|s| s.to_os_string()).collect(),
        working_directory: None,
        timeout: Duration::from_secs(10),
    }
}
#[test]
fn real_process_exit_codes_are_checked() {
    assert_eq!(
        run_tool(&request(&["success".as_ref()]), &|| false)
            .unwrap()
            .exit_code,
        0
    );
    assert!(matches!(
        run_tool(&request(&["fail".as_ref()]), &|| false),
        Err(ToolError::Failed(23))
    ));
}
#[test]
fn timeout_stops_a_real_process() {
    let mut r = request(&["sleep".as_ref()]);
    r.timeout = Duration::from_millis(150);
    let now = Instant::now();
    assert!(matches!(run_tool(&r, &|| false), Err(ToolError::TimedOut)));
    assert!(now.elapsed() < Duration::from_secs(8));
}
#[test]
fn cancellation_before_launch_has_no_side_effect() {
    let f = Fixture::new();
    let output = f.0.join("must-not-exist");
    assert!(matches!(
        run_tool(&request(&["touch".as_ref(), output.as_os_str()]), &|| true),
        Err(ToolError::Cancelled)
    ));
    assert!(!output.exists());
}
#[test]
fn cancellation_after_launch_stops_a_real_process() {
    let now = Instant::now();
    assert!(matches!(
        run_tool(&request(&["sleep".as_ref()]), &|| now.elapsed()
            > Duration::from_millis(150)),
        Err(ToolError::Cancelled)
    ));
    assert!(now.elapsed() < Duration::from_secs(8));
}
#[test]
fn arguments_are_not_shell_commands_and_windows_quoting_round_trips() {
    let f = Fixture::new();
    let output = f.0.join("arguments.bin");
    let supplied = [
        "",
        "two words",
        "工具",
        "embedded\"quote",
        "trailing\\",
        "C:\\folder with spaces\\",
        "back\\\\\"quote",
        "$(touch injected)",
        "& echo injected",
    ];
    let mut args = vec![OsString::from("echo"), output.into_os_string()];
    args.extend(supplied.iter().map(OsString::from));
    let mut r = request(&[]);
    r.arguments = args;
    r.working_directory = Some(f.0.clone());
    run_tool(&r, &|| false).unwrap();
    let bytes = fs::read(f.0.join("arguments.bin")).unwrap();
    let mut at = 0;
    for expected in supplied {
        let n = u64::from_le_bytes(bytes[at..at + 8].try_into().unwrap()) as usize;
        at += 8;
        assert_eq!(&bytes[at..at + n], expected.as_bytes());
        at += n;
    }
    assert_eq!(at, bytes.len());
    assert!(!f.0.join("injected").exists());
}
#[test]
fn invalid_configuration_does_not_launch() {
    let mut r = request(&["success".as_ref()]);
    r.timeout = Duration::ZERO;
    assert!(run_tool(&r, &|| false).is_err());
    r.timeout = Duration::from_secs(1);
    r.executable = PathBuf::from("/does/not/exist");
    assert!(run_tool(&r, &|| false).is_err());
}
#[cfg(windows)]
fn assert_process_stopped(pid_path: &std::path::Path) {
    use windows_sys::Win32::{
        Foundation::{CloseHandle, GetLastError, ERROR_INVALID_PARAMETER, WAIT_OBJECT_0},
        System::Threading::{OpenProcess, WaitForSingleObject},
    };
    let pid: u32 = fs::read_to_string(pid_path).unwrap().parse().unwrap();
    unsafe {
        let handle = OpenProcess(0x00100000, 0, pid); // SYNCHRONIZE access only
        if !handle.is_null() {
            let status = WaitForSingleObject(handle, 5000);
            CloseHandle(handle);
            assert_eq!(status, WAIT_OBJECT_0, "compiler descendant still running");
        } else {
            assert_eq!(
                GetLastError(),
                ERROR_INVALID_PARAMETER,
                "could not verify descendant exit"
            );
        }
    }
}
#[cfg(windows)]
#[test]
fn cancellation_stops_windows_descendants() {
    let f = Fixture::new();
    let pid = f.0.join("pid.txt");
    let r = request(&["spawn".as_ref(), pid.as_os_str()]);
    assert!(matches!(
        run_tool(&r, &|| pid.exists()),
        Err(ToolError::Cancelled)
    ));
    assert_process_stopped(&pid);
}
#[cfg(windows)]
#[test]
fn parent_exit_does_not_abandon_windows_descendants() {
    let f = Fixture::new();
    let pid = f.0.join("pid.txt");
    let mut r = request(&["orphan".as_ref(), pid.as_os_str()]);
    r.timeout = Duration::from_secs(3);
    assert!(matches!(run_tool(&r, &|| false), Err(ToolError::TimedOut)));
    assert_process_stopped(&pid);
}

#[cfg(windows)]
#[test]
fn normal_windows_tool_paths_do_not_leak_verbatim_prefixes() {
    let exe = PathBuf::from(env!("CARGO_BIN_EXE_pafkit-test-tool"));
    let resolved = resolve_tool_path(&exe).unwrap();
    assert_eq!(
        fs::canonicalize(&resolved).unwrap(),
        fs::canonicalize(&exe).unwrap()
    );
    assert!(!resolved.to_string_lossy().starts_with(r"\\?\"));
    let fixture = Fixture::new();
    let directory = resolve_tool_path(&fixture.0).unwrap();
    assert!(!directory.to_string_lossy().starts_with(r"\\?\"));
    assert_eq!(
        fs::canonicalize(directory).unwrap(),
        fs::canonicalize(&fixture.0).unwrap()
    );
}
