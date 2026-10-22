use anyhow::{ensure, Result};
use setupscope_core::{container_readiness, snapshot::Progress};
fn main() -> Result<()> {
    let before: Vec<_> = std::env::vars_os().collect();
    let progress = Progress::default();
    let report = container_readiness::inspect(&progress);
    ensure!(!report.cancelled, "unexpected cancellation");
    ensure!(report.clients.len() == 4, "fixed client inventory incomplete");
    ensure!(report.host.len() >= 2, "processor observations missing");
    ensure!(report.host.iter().any(|v| v.name == "WslService") || report.partial, "service query missing without partial flag");
    ensure!(before == std::env::vars_os().collect::<Vec<_>>(), "process environment changed");
    let text = report.text();
    for name in ["DOCKER_HOST", "DOCKER_CONTEXT", "CONTAINER_HOST"] {
        if let Ok(value) = std::env::var(name) {
            if value.starts_with("PRIVATE_PROBE_") { ensure!(!text.contains(&value), "override value leaked"); }
        }
    }
    println!("PASS: passive live Windows scan; four fixed clients, native host evidence, environment unchanged, override values omitted; partial={}", report.partial);
    Ok(())
}
