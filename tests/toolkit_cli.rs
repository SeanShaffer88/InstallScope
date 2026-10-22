//! Black-box tests of the supported console entry point; no project code runs.
use std::{
    path::PathBuf,
    process::{Command, Output, Stdio},
};
fn cli(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_setupscope-cli"))
        .args(args)
        .stdin(Stdio::null())
        .output()
        .unwrap()
}
struct Scratch(PathBuf);
impl Scratch {
    fn new() -> Self {
        let p = std::env::temp_dir().join(format!(
            "setupscope-cli-test-{}",
            setupscope_core::util::new_id()
        ));
        std::fs::create_dir(&p).unwrap();
        Self(p)
    }
}
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
#[test]
fn console_help_catalog_and_json_are_scriptable() {
    assert!(cli(&["--help"]).status.success());
    let output = cli(&["system", "--json"]);
    assert!(output.status.success());
    let value = setupscope_core::json::parse(std::str::from_utf8(&output.stdout).unwrap()).unwrap();
    assert_eq!(
        value.get("command").and_then(|v| v.as_str()),
        Some("system")
    );
    let output = cli(&["dependencies", "catalog"]);
    assert!(output.status.success());
    let catalog: setupscope_core::dependencies::Catalog =
        setupscope_core::json::from_json_str(std::str::from_utf8(&output.stdout).unwrap()).unwrap();
    assert_eq!(catalog.packages.len(), 6);
    assert_eq!(cli(&["programs"]).status.code(), Some(2));
}
#[test]
fn bounded_triage_does_not_modify_or_echo_raw_secret() {
    let temp = Scratch::new();
    let file = temp.0.join("build.log");
    let bytes = b"PRIVATE_CLI_TOKEN\nerror C1083: Cannot open include file: 'example.h'\n";
    std::fs::write(&file, bytes).unwrap();
    let output = cli(&["triage", file.to_str().unwrap()]);
    assert!(output.status.success());
    assert_eq!(std::fs::read(file).unwrap(), bytes);
    assert!(!String::from_utf8_lossy(&output.stdout).contains("PRIVATE_CLI_TOKEN"));
}
#[test]
#[cfg(windows)]
fn piped_execution_cannot_bypass_exact_review() {
    let temp = Scratch::new();
    let root = temp.0.to_str().unwrap();
    let output = cli(&[
        "execute",
        root,
        env!("CARGO_BIN_EXE_setupscope-cli"),
        root,
        "1",
        "--",
        "--version",
    ]);
    assert_eq!(output.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&output.stderr).contains("Interactive terminal required"));
    assert_eq!(std::fs::read_dir(&temp.0).unwrap().count(), 0);
}
#[test]
fn malformed_invocations_exit_2_and_operational_failures_exit_1() {
    for args in [
        &["programs"][..],
        &["system", "extra"],
        &["dependencies"],
        &["dependencies", "verify", "/tmp"],
        &["dependencies", "nonsense", "/tmp", "x"],
        &["dependencies", "export", "/tmp/a", "/tmp/b", "--accept-license"],
        &["run", "a", "b", "c", "d"],
        &["diagnostics", "rust"],
    ] {
        let output = cli(args);
        assert_eq!(output.status.code(), Some(2), "{args:?}");
    }
    // A well-formed request rejected by validation is an operational failure.
    let temp = Scratch::new();
    let root = temp.0.to_str().unwrap();
    assert_eq!(
        cli(&["dependencies", "verify", root, "unknown-package"]).status.code(),
        Some(1)
    );
    assert_eq!(cli(&["readiness", "relative"]).status.code(), Some(1));
}
