//! Development-only read-only Project Doctor probe; not an application CLI command.
use setupscope_core::{
    devcheck::Target,
    projectdoctor::{self, State},
    snapshot::Progress,
};
use std::path::PathBuf;
fn main() -> anyhow::Result<()> {
    let root = PathBuf::from(
        std::env::args_os()
            .nth(1)
            .ok_or_else(|| anyhow::anyhow!("Pass an absolute project folder"))?,
    );
    let manifest = root.join("Cargo.toml");
    let before = std::fs::read(&manifest).ok();
    let path = std::env::var_os("PATH");
    let report = projectdoctor::inspect(
        &projectdoctor::Request::current(root, Target::X64)?,
        &Progress::default(),
    )?;
    anyhow::ensure!(std::env::var_os("PATH") == path, "PATH changed");
    anyhow::ensure!(
        std::fs::read(manifest).ok() == before,
        "Manifest contents changed"
    );
    println!(
        "files={} findings={} partial={} cancelled={} PATH_unchanged=true manifest_unchanged=true",
        report.files.len(),
        report.findings.len(),
        report.partial,
        report.cancelled
    );
    for row in report.findings {
        println!(
            "{}: {}",
            row.tool,
            match row.state {
                State::Candidate => "candidate metadata",
                State::Different => "different metadata",
                State::Missing => "not found in checked locations",
                State::Unknown => "unknown",
                State::Review => "review",
            }
        );
    }
    Ok(())
}
