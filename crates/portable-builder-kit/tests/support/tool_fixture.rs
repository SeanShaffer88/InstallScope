//! Test-only helper; never enabled in normal builds.
use std::{
    ffi::OsString,
    fs,
    process::{Command, ExitCode},
    thread,
    time::Duration,
};
#[allow(clippy::zombie_processes)] // Deliberately leaves a child for supervisor lifecycle tests.
fn main() -> ExitCode {
    let mut args = std::env::args_os().skip(1);
    match args.next().as_deref().and_then(|s| s.to_str()) {
        Some("success") => ExitCode::SUCCESS,
        Some("fail") => ExitCode::from(23),
        Some("sleep") => {
            thread::sleep(Duration::from_secs(20));
            ExitCode::SUCCESS
        }
        Some("touch") => {
            fs::write(args.next().unwrap(), b"started").unwrap();
            ExitCode::SUCCESS
        }
        Some("echo") => {
            let out = args.next().unwrap();
            // A lossless per-argument length framing for the Unicode test inputs.
            let mut bytes = Vec::new();
            for arg in args {
                let arg = arg.to_str().unwrap().as_bytes();
                bytes.extend_from_slice(&(arg.len() as u64).to_le_bytes());
                bytes.extend_from_slice(arg);
            }
            fs::write(out, bytes).unwrap();
            ExitCode::SUCCESS
        }
        Some("spawn") | Some("orphan") => {
            let mode: OsString = std::env::args_os().nth(1).unwrap();
            let child = Command::new(std::env::current_exe().unwrap())
                .arg("sleep")
                .spawn()
                .unwrap();
            let pid = std::path::PathBuf::from(args.next().unwrap());
            let pending = pid.with_extension("pending");
            fs::write(&pending, child.id().to_string()).unwrap();
            fs::rename(pending, pid).unwrap();
            if mode == "spawn" {
                thread::sleep(Duration::from_secs(20));
            }
            ExitCode::SUCCESS
        }
        _ => ExitCode::from(2),
    }
}
