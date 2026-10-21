//! Run: cargo run --example preflight --manifest-path crates/portable-builder-kit/Cargo.toml -- PACKAGE
use std::{env, path::PathBuf, process::ExitCode};

fn main() -> ExitCode {
    let args: Vec<_> = env::args_os().skip(1).collect();
    if args.len() != 1 {
        eprintln!("Usage: preflight PACKAGE\nChecks required assets and the compiled launcher; does not run the application.");
        return ExitCode::from(1);
    }
    let package = PathBuf::from(&args[0]);
    let findings = portable_builder_kit::preflight(&package, true);
    if findings.is_empty() {
        println!("Static preflight passed. Runtime portability and full format compliance still require verification.");
        ExitCode::SUCCESS
    } else {
        for finding in findings {
            eprintln!("{}: {}", finding.path.display(), finding.message);
        }
        ExitCode::from(2)
    }
}
