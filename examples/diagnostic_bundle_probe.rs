use anyhow::{ensure, Result};
use setupscope_core::{
    devcheck::{self, Profile, Target},
    diagnostic_bundle::{self, Sections},
    snapshot::Progress,
};
fn main() -> Result<()> {
    let out = std::path::PathBuf::from(std::env::args_os().nth(1).expect("explicit output path"));
    let request = devcheck::Request::current(Profile::RustMsvc, Target::X64, String::new())?;
    let report = devcheck::inspect(&request, &Progress::default())?;
    let text = diagnostic_bundle::preview(
        Sections {
            runtime: true,
            readiness: true,
            tools: true,
        },
        Some(&report),
    )?;
    for entry in &report.entries {
        if entry.raw.len() > 8 {
            ensure!(!text.contains(&entry.raw), "raw PATH entry leaked");
        }
    }
    diagnostic_bundle::export(&out, &text)?;
    ensure!(
        std::fs::read(&out)? == text.as_bytes(),
        "preview/export mismatch"
    );
    ensure!(
        diagnostic_bundle::export(&out, "replacement").is_err(),
        "overwrite allowed"
    );
    ensure!(std::fs::read(&out)? == text.as_bytes(), "original changed");
    println!("PASS: live read-only scan, redacted JSON, exact preview export, existing file retained ({} bytes).", text.len());
    Ok(())
}
