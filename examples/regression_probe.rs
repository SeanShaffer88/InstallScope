//! Development-only synthetic lifecycle probe. Does not run vendor software.
use anyhow::{ensure, Context, Result};
use setupscope_core::{
    platform::MemRegistry,
    regression::{self, CaptureRequest, Outcome, Scope, Step, Workspace},
    settings::{FileRoot, Settings},
    snapshot::Progress,
};
use std::{
    fs,
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};

fn main() -> Result<()> {
    let parent = PathBuf::from(
        std::env::args_os()
            .nth(1)
            .context("Pass an absolute scratch parent directory")?,
    );
    ensure!(parent.is_absolute(), "Scratch parent must be absolute");
    fs::create_dir_all(&parent)?;
    let root = parent.join(format!(
        "regression-probe-{}-{}",
        std::process::id(),
        SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos()
    ));
    fs::create_dir(&root)?;
    let data = root.join("data");
    fs::create_dir(&data)?;
    let settings = Settings {
        registry_roots: vec![],
        file_roots: vec![FileRoot {
            path: root
                .to_str()
                .context("Unicode scratch path required")?
                .into(),
            recursive: true,
        }],
        file_exclusions: vec![],
        auto_exclusions: false,
        ..Settings::default()
    };
    let mut workspace = regression::create(
        &data,
        "Synthetic lifecycle — no vendor operations",
        Scope::from_settings(&settings, &data)?,
    )?;
    let reg = MemRegistry::new();
    let checkpoint = |workspace: &Workspace, step: Step, outcome: Outcome| {
        regression::capture(
            &data,
            &workspace.id,
            CaptureRequest {
                revision: workspace.revision,
                step,
                outcome,
                reported_exit_code: None,
                note: "Synthetic scratch files only".into(),
            },
            &reg,
            &Progress::default(),
        )
    };
    workspace = checkpoint(&workspace, Step::Before, Outcome::NotApplicable)?;
    let app = root.join("fixture-program.txt");
    fs::write(&app, "version 1")?;
    workspace = checkpoint(&workspace, Step::Installed, Outcome::Success)?;
    fs::write(&app, "version 2 with different metadata")?;
    workspace = checkpoint(&workspace, Step::Upgraded, Outcome::Success)?;
    fs::remove_file(&app)?;
    fs::write(
        root.join("fixture-leftover.txt"),
        "review this synthetic leftover",
    )?;
    workspace = checkpoint(&workspace, Step::Uninstalled, Outcome::Success)?;
    let reopened = regression::load(&data, &workspace.id)?;
    let upgrade = regression::compare(&data, &reopened.id, reopened.revision, 1, 2)?;
    let remaining = regression::compare(&data, &reopened.id, reopened.revision, 0, 3)?;
    ensure!(
        reopened.revision == 8 && reopened.allowed_steps().is_empty(),
        "Lifecycle receipt mismatch"
    );
    ensure!(
        upgrade.changes.files_modified.len() == 1,
        "Upgrade metadata delta mismatch"
    );
    ensure!(
        remaining.complete_observation
            && remaining.changes.total() == 1
            && remaining.changes.files_added[0]
                .path
                .ends_with("fixture-leftover.txt"),
        "Leftover/self-exclusion mismatch"
    );
    println!("PASS: four checkpoints reopened; upgrade delta detected; one synthetic leftover; own evidence excluded.");
    println!(
        "Vendor operations: none. Evidence retained at {}",
        root.display()
    );
    Ok(())
}
