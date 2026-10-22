use anyhow::{ensure, Result};
use setupscope_core::{build_log, snapshot::Progress};
fn main() -> Result<()> {
    let path = std::path::PathBuf::from(std::env::args_os().nth(1).expect("explicit fixture path"));
    let before = std::fs::read(&path)?;
    let metadata = std::fs::metadata(&path)?;
    let report = build_log::inspect(&path, &Progress::default())?;
    ensure!(report.findings.len() >= 2, "expected fixture findings");
    ensure!(
        !report.text().contains("PRIVATE_FIXTURE_TOKEN"),
        "raw source leaked"
    );
    ensure!(std::fs::read(&path)? == before, "log bytes changed");
    ensure!(
        std::fs::metadata(&path)?.modified()? == metadata.modified()?,
        "log modified time changed"
    );
    println!("PASS: bounded live file triage; {} grouped findings, source bytes/mtime unchanged, raw secret omitted.", report.findings.len());
    Ok(())
}
