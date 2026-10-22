//! Explicit, reviewed local execution. Project code is arbitrary code, not a sandbox.
use crate::{dependencies::local_path, snapshot::Progress};
use anyhow::{ensure, Context, Result};
use std::{
    fs::OpenOptions,
    io::{Read, Write},
    path::PathBuf,
    time::Duration,
};
#[cfg(windows)]
mod windows;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Request {
    pub project: PathBuf,
    pub executable: PathBuf,
    pub arguments: Vec<String>,
    pub logs: PathBuf,
    pub timeout_seconds: u32,
}
pub struct Review {
    request: Request,
    _executable: crate::dependencies::Locked,
    text: String,
}
impl Review {
    pub fn text(&self) -> &str {
        &self.text
    }
}
pub struct Outcome {
    pub log: PathBuf,
    pub summary: String,
    pub successful: bool,
}
pub const MAX_LOG: usize = 2 * 1024 * 1024 - 65536;
type Captured = (u32, String, Vec<u8>, Vec<u8>, bool);

pub fn review(request: Request) -> Result<Review> {
    local_path(&request.project)?;
    local_path(&request.executable)?;
    local_path(&request.logs)?;
    ensure!(
        request.project.is_dir() && request.logs.is_dir(),
        "Project and log folders must already exist"
    );
    ensure!(
        (1..=7200).contains(&request.timeout_seconds),
        "Timeout must be 1–7200 seconds"
    );
    ensure!(
        request.arguments.len() <= 64
            && request
                .arguments
                .iter()
                .all(|a| a.len() <= 4096 && !a.chars().any(char::is_control)),
        "Arguments exceed limits or contain control characters"
    );
    ensure!(
        request.arguments.iter().map(String::len).sum::<usize>() <= 16000,
        "Combined arguments exceed 16000 bytes"
    );
    ensure!(
        request
            .executable
            .extension()
            .is_some_and(|s| s.eq_ignore_ascii_case("exe")),
        "Select an absolute .exe, not a shell script or PATH command"
    );
    let executable = crate::dependencies::lock(&request.executable)
        .context("Cannot hold selected executable")?;
    let text = format!("REVIEW LOCAL EXECUTION\nExecutable: {}\nArguments (one literal argument per line):\n{}\nWorking folder: {}\nLog folder: {}\nTimeout: {} seconds\n\nThis runs arbitrary project/tool code with your current account and inherited environment, credentials and network access. No elevation, shell parsing, or sandbox. Builds/tests can download packages, execute hooks/tests, modify or delete files, and contact services. Review the project, executable and arguments yourself. Standard input is closed. Output is capped at 2 MiB; excess is drained and discarded. Cancellation/timeout ends the contained process tree; it cannot undo files, downloads, remote effects or changes made before stopping. Remaining child processes are stopped when the root process exits. Logs can contain secrets and local paths; review before sharing. A zero exit code is not proof of correctness. The executable is held against normal write/delete during review, but project/dependency files may change.\n", request.executable.display(), request.arguments.iter().map(|s|format!("{s:?}")).collect::<Vec<_>>().join("\n"), request.project.display(), request.logs.display(), request.timeout_seconds);
    Ok(Review {
        request,
        _executable: executable,
        text,
    })
}
pub fn run(review: Review, acknowledged: bool, progress: &Progress) -> Result<Outcome> {
    ensure!(
        acknowledged,
        "Execution requires separate review acknowledgment"
    );
    ensure!(!progress.cancelled(), "Cancelled before execution");
    // Revalidate paths and limits immediately before creating any process.
    let _recheck = self::review(review.request.clone())?;
    let path = review
        .request
        .logs
        .join(format!("SetupScope-build-{}.log", crate::util::new_id()));
    let mut log = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
        .context("Cannot create a new log; nothing launched")?;
    log.write_all(format!("SetupScope reviewed run\nWithout a final status, execution outcome is unknown; inspect project state before retrying.\n{}\n", review.text).as_bytes())?;
    log.sync_all()?;
    #[cfg(windows)]
    let result = windows::execute(&review.request, progress);
    #[cfg(not(windows))]
    let result: Result<(u32, String, Vec<u8>, Vec<u8>, bool)> =
        Err(anyhow::anyhow!("Reviewed execution requires Windows"));
    match result {
        Ok((exit, state, stdout, stderr, truncated)) => {
            let summary = format!("{state}; exit {exit}; output truncated: {truncated}. Child tree closed. Effects are not rolled back.\nLog: {}",path.display());
            let saved = (|| -> std::io::Result<()> {
                log.write_all(b"\n--- STDOUT (stream order only) ---\n")?;
                log.write_all(&stdout)?;
                log.write_all(b"\n--- STDERR (stream order only) ---\n")?;
                log.write_all(&stderr)?;
                log.write_all(format!("\n{summary}\n").as_bytes())?;
                log.sync_all()
            })();
            Ok(Outcome { log: path, successful: exit==0 && state=="Completed" && saved.is_ok(), summary: match saved { Ok(())=>summary, Err(e)=>format!("{summary}\nLOG SAVE FAILED AFTER EXECUTION: {e}; output may be incomplete.") } })
        }
        Err(e) => {
            let _ = writeln!(
                log,
                "\nExecution could not complete: {e:#}. Inspect project state before retrying."
            );
            let _ = log.sync_all();
            Err(e.context(format!("Run log: {}", path.display())))
        }
    }
}
fn read_bounded(mut input: impl Read, limit: usize) -> (Vec<u8>, bool) {
    let mut output = Vec::new();
    let mut buffer = [0u8; 8192];
    let mut truncated = false;
    loop {
        match input.read(&mut buffer) {
            Ok(0) => break,
            Ok(n) => {
                let keep = n.min(limit.saturating_sub(output.len()));
                output.extend_from_slice(&buffer[..keep]);
                truncated |= keep < n;
            }
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(_) => {
                truncated = true;
                break;
            }
        }
    }
    // Convert arbitrary tool bytes to valid UTF-8 for the existing log inspector.
    let mut text = String::from_utf8_lossy(&output).into_owned();
    if text.len() > limit {
        let mut end = limit;
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        text.truncate(end);
        truncated = true;
    }
    (text.into_bytes(), truncated)
}
/// Literal Windows CRT quoting; never evaluated by cmd.exe or PowerShell.
pub(crate) fn quote(argument: &str) -> String {
    let mut text = String::from("\"");
    let mut slashes = 0;
    for c in argument.chars() {
        if c == '\\' {
            slashes += 1;
            continue;
        }
        if c == '"' {
            text.push_str(&"\\".repeat(slashes * 2 + 1));
        } else {
            text.push_str(&"\\".repeat(slashes));
        }
        slashes = 0;
        text.push(c);
    }
    text.push_str(&"\\".repeat(slashes * 2));
    text.push('"');
    text
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn arguments_are_literal_and_trailing_slashes_survive() {
        assert_eq!(quote("a b"), "\"a b\"");
        assert_eq!(quote(""), "\"\"");
        assert_eq!(quote("a\\"), "\"a\\\\\"");
        assert_eq!(quote("a\"b"), "\"a\\\"b\"");
        assert_eq!(quote("& whoami"), "\"& whoami\"");
    }
    #[test]
    fn output_is_drained_but_bounded() {
        let (bytes, truncated) = read_bounded(&b"123456789"[..], 4);
        assert_eq!(bytes, b"1234");
        assert!(truncated);
    }
    #[test]
    fn malformed_output_becomes_bounded_utf8() {
        let (bytes, truncated) = read_bounded(&[0xff; 10][..], 10);
        assert!(truncated);
        assert!(bytes.len() <= 10);
        assert!(std::str::from_utf8(&bytes).is_ok());
    }
    #[test]
    fn invalid_request_never_reaches_execution() {
        let r = Request {
            project: "relative".into(),
            executable: "cmd.exe".into(),
            arguments: vec![],
            logs: "relative".into(),
            timeout_seconds: 0,
        };
        assert!(review(r).is_err());
    }
    #[cfg(windows)]
    fn fixture_request(root: &std::path::Path, name: &str) -> Request {
        Request {
            project: root.into(),
            executable: std::env::current_exe().unwrap(),
            arguments: vec![
                "--ignored".into(),
                "--exact".into(),
                format!("build_run::tests::{name}"),
                "--nocapture".into(),
            ],
            logs: root.into(),
            timeout_seconds: 10,
        }
    }
    #[test]
    #[ignore]
    fn execution_fixture() {
        println!("error E0432: synthetic diagnostic fixture");
        eprintln!("fixture stderr");
    }
    #[test]
    #[ignore]
    fn failure_fixture() {
        std::process::exit(7);
    }
    #[test]
    #[ignore]
    fn sleeping_fixture() {
        std::thread::sleep(Duration::from_secs(60));
    }
    #[test]
    #[ignore]
    fn noisy_fixture() {
        let bytes = vec![b'x'; 3 * 1024 * 1024];
        std::io::stdout().write_all(&bytes).unwrap();
    }
    #[test]
    #[ignore]
    #[allow(clippy::zombie_processes)] // Deliberate detached child; owning Job Object must terminate it.
    fn descendant_fixture() {
        let child = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--ignored",
                "--exact",
                "build_run::tests::sleeping_fixture",
                "--nocapture",
            ])
            .spawn()
            .unwrap();
        println!("CHILD_PID={}", child.id());
    }
    #[cfg(windows)]
    #[test]
    fn reviewed_execution_captures_failure_and_triages_real_log() {
        let tmp = crate::util::tempdir().unwrap();
        let p = Progress::default();
        let outcome = run(
            review(fixture_request(tmp.path(), "execution_fixture")).unwrap(),
            true,
            &p,
        )
        .unwrap();
        assert!(outcome.summary.contains("exit 0"), "{}", outcome.summary);
        let report = crate::build_log::inspect(&outcome.log, &p).unwrap();
        assert!(report
            .findings
            .iter()
            .any(|f| crate::build_log::RULES[f.rule].id == "rust-import"));
        let outcome = run(
            review(fixture_request(tmp.path(), "failure_fixture")).unwrap(),
            true,
            &p,
        )
        .unwrap();
        assert!(outcome.summary.contains("exit 7"));
    }
    #[cfg(windows)]
    #[test]
    fn no_consent_or_precancelled_never_creates_run_log() {
        let tmp = crate::util::tempdir().unwrap();
        let p = Progress::default();
        assert!(run(
            review(fixture_request(tmp.path(), "execution_fixture")).unwrap(),
            false,
            &p
        )
        .is_err());
        p.cancel.store(true, std::sync::atomic::Ordering::Relaxed);
        assert!(run(
            review(fixture_request(tmp.path(), "execution_fixture")).unwrap(),
            true,
            &p
        )
        .is_err());
        assert_eq!(std::fs::read_dir(tmp.path()).unwrap().count(), 0);
    }
    #[cfg(windows)]
    #[test]
    fn timeout_stops_process_and_output_flood_stays_bounded() {
        let tmp = crate::util::tempdir().unwrap();
        let p = Progress::default();
        let mut r = fixture_request(tmp.path(), "sleeping_fixture");
        r.timeout_seconds = 1;
        let start = std::time::Instant::now();
        let out = run(review(r).unwrap(), true, &p).unwrap();
        assert!(out.summary.contains("Timed out"));
        assert!(start.elapsed() < Duration::from_secs(15));
        let out = run(
            review(fixture_request(tmp.path(), "noisy_fixture")).unwrap(),
            true,
            &p,
        )
        .unwrap();
        assert!(out.summary.contains("truncated: true"));
        assert!(std::fs::metadata(out.log).unwrap().len() < 2 * 1024 * 1024);
    }
    #[cfg(windows)]
    #[test]
    fn root_exit_stops_background_descendants() {
        let tmp = crate::util::tempdir().unwrap();
        let p = Progress::default();
        let out = run(
            review(fixture_request(tmp.path(), "descendant_fixture")).unwrap(),
            true,
            &p,
        )
        .unwrap();
        let text = std::fs::read_to_string(out.log).unwrap();
        let pid: u32 = text
            .lines()
            .find_map(|s| s.strip_prefix("CHILD_PID="))
            .unwrap()
            .parse()
            .unwrap();
        unsafe {
            use windows_sys::Win32::{Foundation::*, System::Threading::*};
            let process = OpenProcess(PROCESS_SYNCHRONIZE, 0, pid);
            if !process.is_null() {
                assert_eq!(WaitForSingleObject(process, 5000), WAIT_OBJECT_0);
                CloseHandle(process);
            }
        }
    }
    #[cfg(windows)]
    #[test]
    fn active_cancellation_stops_running_code() {
        let tmp = crate::util::tempdir().unwrap();
        let p = Progress::default();
        let review = review(fixture_request(tmp.path(), "sleeping_fixture")).unwrap();
        std::thread::scope(|s| {
            s.spawn(|| {
                std::thread::sleep(Duration::from_millis(300));
                p.cancel.store(true, std::sync::atomic::Ordering::Relaxed);
            });
            let out = run(review, true, &p).unwrap();
            assert!(out.summary.contains("Cancelled"));
        });
    }
}
