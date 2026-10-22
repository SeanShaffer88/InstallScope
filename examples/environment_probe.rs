//! Development-only child environment probe. Never applies persistent changes.
use anyhow::{ensure, Context, Result};
use setupscope_core::{
    envprofile::{self, Environment, Profile, Variable},
    json::compat,
};
use std::{
    fs,
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};
#[derive(Debug)]
struct Evidence {
    cargo_home: String,
    path_prefix: String,
}
setupscope_core::json_struct!(Evidence,{cargo_home,path_prefix});
fn main() -> Result<()> {
    let arguments: Vec<_> = std::env::args_os().collect();
    if arguments.get(1).is_some_and(|a| a == "--child") {
        let output = PathBuf::from(arguments.get(2).context("Missing child evidence path")?);
        let evidence = Evidence {
            cargo_home: std::env::var("CARGO_HOME")?,
            path_prefix: std::env::var("Path")?
                .split(';')
                .next()
                .unwrap_or_default()
                .into(),
        };
        fs::write(output, compat::to_vec_pretty(&evidence)?)?;
        return Ok(());
    }
    let parent = PathBuf::from(
        arguments
            .get(1)
            .context("Pass an absolute scratch parent")?,
    );
    ensure!(parent.is_absolute(), "Scratch parent must be absolute");
    fs::create_dir_all(&parent)?;
    let root = parent.join(format!(
        "environment-probe-{}-{}",
        std::process::id(),
        SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos()
    ));
    fs::create_dir(&root)?;
    let sdk = root.join("sdk");
    fs::create_dir(&sdk)?;
    let parent_cargo = std::env::var_os("CARGO_HOME");
    let parent_path = std::env::var_os("Path");
    #[cfg(windows)]
    let saved_before = (
        envprofile::UserEnvironment.read("CARGO_HOME")?,
        envprofile::UserEnvironment.read("Path")?,
    );
    let profile = Profile {
        version: 1,
        name: "Synthetic child-only environment".into(),
        project: root.to_str().unwrap().into(),
        path_prepend: vec![sdk.to_str().unwrap().into()],
        variables: vec![Variable {
            name: "CARGO_HOME".into(),
            value: Some(sdk.to_str().unwrap().into()),
        }],
    };
    let plan = envprofile::preview_process(&profile)?;
    let output = root.join("child-evidence.json");
    let mut child = envprofile::launch(
        &plan,
        &std::env::current_exe()?,
        &format!("--child \"{}\"", output.display()),
        false,
    )?;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
    loop {
        if let Some(status) = child.try_wait()? {
            ensure!(status.success(), "Child probe failed: {status}");
            break;
        }
        ensure!(
            std::time::Instant::now() < deadline,
            "Child probe still running; inspect PID {} before retrying",
            child.id()
        );
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    let evidence: Evidence = compat::from_slice(&fs::read(&output)?)?;
    ensure!(
        evidence.cargo_home == sdk.to_str().unwrap()
            && evidence.path_prefix == sdk.to_str().unwrap(),
        "Child overlay mismatch"
    );
    ensure!(
        std::env::var_os("CARGO_HOME") == parent_cargo && std::env::var_os("Path") == parent_path,
        "Parent environment changed"
    );
    #[cfg(windows)]
    ensure!(
        (
            envprofile::UserEnvironment.read("CARGO_HOME")?,
            envprofile::UserEnvironment.read("Path")?
        ) == saved_before,
        "Saved user values changed"
    );
    let export = root.join("profile.json");
    envprofile::export(&export, &profile)?;
    ensure!(
        envprofile::import(&export)? == profile,
        "Profile round-trip mismatch"
    );
    println!("PASS: child-only CARGO_HOME/PATH overlay, parent and saved user values unchanged, profile round-trip verified.");
    println!(
        "No persistent environment mutation. Synthetic evidence retained at {}",
        root.display()
    );
    Ok(())
}
