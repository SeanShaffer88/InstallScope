//! Supported developer CLI; deliberately independent of retired maintenance commands.
use anyhow::{bail, ensure, Context, Result};
use setupscope_core::{
    build_log, build_run, container_readiness, dependencies, devcheck, devinspect,
    diagnostic_bundle, envprofile, json, project_readiness, snapshot::Progress,
};
use std::{
    io::{IsTerminal, Write},
    path::PathBuf,
};

const HELP: &str = "SetupScope — Windows Software Development Toolkit
Usage: SetupScope <command> [arguments] [--json]
  system                         Local OS, architecture, CPU and memory snapshot
  readiness PROJECT [x64|arm64]   Bounded project/toolchain/PATH/version evidence
  toolchain PROFILE TARGET COMMAND [MIN|-] [MAX|-]  Prerequisites, PATH and numeric metadata
  triage LOG                     Bounded saved build-log hints; excludes raw log
  containers                     Passive WSL/container/virtualization evidence
  ports [PORT|-] [PID|-] [PAGE]   TCP/UDP snapshot; pages are zero-based (50 rows)
  file-owners FILE [PAGE]         Selected-file owners; never terminates processes
  environment-preview PROFILE    Validate JSON profile and preview process changes
  environment-user-preview PROFILE  Preview persistent HKCU environment changes
  environment-apply DATA PROFILE Review, confirm APPLY, then save recovery and apply
  environment-history DATA       List recovery transaction IDs
  environment-receipt DATA ID    Print an existing recovery receipt
  environment-undo DATA ID       Review receipt, confirm UNDO, restore if conflict-free
  dependencies catalog           Pinned package IDs, hashes and official URLs
  dependencies verify ROOT ID... Verify selected cached installers
  dependencies download ROOT ID... Explicit official network downloads to cache
  dependencies import ROOT BUNDLE Verify/import an offline bundle
  dependencies export ROOT DEST ID... --accept-license  Create selected bundle
  dependencies review ROOT ID    Inspect installer review; no installation
  dependencies receipts ROOT     Saved vendor-operation receipts (may contain paths)
  dependencies install ROOT ID   Review, then interactive INSTALL confirmation
  diagnostics [PROFILE TARGET]   Redacted runtime and optional toolchain count bundle
  run PROJECT EXE LOGDIR TIMEOUT -- ARG...  Preview exact build/test invocation
  execute PROJECT EXE LOGDIR TIMEOUT -- ARG...  Review, then interactive RUN confirmation
  languages                      Available interface language codes
  --help | --version
Global --json returns a schema-1 envelope with a text report (not structured findings).
Exit codes: 0 command completed (does not certify readiness/build success),
1 operational failure (including nonzero/timeout execution), 2 invalid command or count.
Reports may contain local paths. PROFILE: rust, dotnet, node, python, cmake.
Diagnostics do not execute project code. No implicit writes, uploads or repairs.
Install/execute require an interactive terminal; no unattended confirmation bypass.
Launch without arguments for the desktop Workflow Board.";

fn count(a: &[String], min: usize, max: usize) -> Result<()> {
    ensure!(
        (min..=max).contains(&a.len()),
        "Invalid argument count; use --help"
    );
    Ok(())
}
fn confirm(word: &str) -> Result<()> {
    ensure!(
        std::io::stdin().is_terminal() && std::io::stderr().is_terminal(),
        "Interactive terminal required; nothing launched"
    );
    eprint!("Type {word} to approve this exact review: ");
    std::io::stderr().flush()?;
    let mut answer = String::new();
    std::io::stdin().read_line(&mut answer)?;
    ensure!(answer.trim() == word, "Not confirmed; nothing launched");
    Ok(())
}
fn page(a: Option<&String>) -> Result<usize> {
    a.map(|s| s.parse().context("PAGE must be a nonnegative integer"))
        .unwrap_or(Ok(0))
}
fn profile(s: &str) -> Result<devcheck::Profile> {
    Ok(match s {
        "rust" => devcheck::Profile::RustMsvc,
        "dotnet" => devcheck::Profile::DotNet,
        "node" => devcheck::Profile::Node,
        "python" => devcheck::Profile::Python,
        "cmake" => devcheck::Profile::Cmake,
        _ => bail!("PROFILE must be rust, dotnet, node, python or cmake"),
    })
}
fn target(s: &str) -> Result<devcheck::Target> {
    Ok(match s {
        "x64" => devcheck::Target::X64,
        "arm64" => devcheck::Target::Arm64,
        _ => bail!("TARGET must be x64 or arm64"),
    })
}
fn report(a: &[String]) -> Result<String> {
    let p = Progress::default();
    match a[0].as_str() {
        "--help" | "-h" | "help" => {
            count(a, 1, 1)?;
            Ok(HELP.into())
        }
        "--version" => {
            count(a, 1, 1)?;
            Ok(format!("SetupScope {}.0", env!("CARGO_PKG_VERSION")))
        }
        "system" => {
            count(a, 1, 1)?;
            Ok(crate::system_info::summary())
        }
        "languages" => {
            count(a, 1, 1)?;
            Ok(setupscope_core::i18n::languages()
                .iter()
                .map(|l| format!("{}\t{}", l.code, l.name))
                .collect::<Vec<_>>()
                .join("\n"))
        }
        "readiness" => {
            count(a, 2, 3)?;
            let target = match a.get(2).map(String::as_str).unwrap_or("x64") {
                "x64" => devcheck::Target::X64,
                "arm64" => devcheck::Target::Arm64,
                _ => bail!("Target must be x64 or arm64"),
            };
            let r = project_readiness::inspect(
                &project_readiness::Selection {
                    project: PathBuf::from(&a[1]),
                    target,
                    fallback_profile: devcheck::Profile::RustMsvc,
                    log: None,
                },
                &p,
            )?;
            Ok(format!("{}\n{}", r.summary(), r.project.text()))
        }
        "triage" => {
            count(a, 2, 2)?;
            Ok(build_log::inspect(&PathBuf::from(&a[1]), &p)?.text())
        }
        "toolchain" => {
            count(a, 4, 6)?;
            let mut request =
                devcheck::Request::current(profile(&a[1])?, target(&a[2])?, a[3].clone())?;
            request.minimum_version = a
                .get(4)
                .filter(|s| s.as_str() != "-")
                .cloned()
                .unwrap_or_default();
            request.maximum_version = a
                .get(5)
                .filter(|s| s.as_str() != "-")
                .cloned()
                .unwrap_or_default();
            let r = devcheck::inspect(&request, &p)?;
            let mut text=format!("Read-only toolchain / PATH / version metadata. No executable is run.\nProfile: {} · Target: {} · Cancelled: {} · PATH partial: {}\nShell/IDE selection and build success remain unverified.\n",r.profile.label(),r.target.label(),r.cancelled,r.path_partial);
            for c in &r.checks {
                text.push_str(&format!(
                    "\n{}: {}\n{}\n{}\n",
                    c.name,
                    c.status.label(),
                    c.guidance,
                    c.evidence.join("\n")
                ));
            }
            for row in &r.versions {
                text.push_str(&format!(
                    "\n{}: {}\n{}\n",
                    row.path.display(),
                    row.compatibility.label(),
                    row.detail
                ));
            }
            for note in r.notes {
                text.push_str(&format!("\n{note}\n"));
            }
            Ok(text)
        }
        "containers" => {
            count(a, 1, 1)?;
            Ok(container_readiness::inspect(&p).text())
        }
        "ports" => {
            count(a, 1, 4)?;
            let value = |n| {
                a.get(n)
                    .map(String::as_str)
                    .filter(|s| *s != "-")
                    .unwrap_or("")
            };
            let f = devinspect::Filter::parse(value(1), value(2))?;
            Ok(devinspect::inspect_ports(true, &p)?
                .page(f, page(a.get(3))?, false)
                .text)
        }
        "file-owners" => {
            count(a, 2, 3)?;
            Ok(devinspect::inspect_file(&PathBuf::from(&a[1]), &p)?
                .page(page(a.get(2))?, false)
                .text)
        }
        "environment-preview" => {
            count(a, 2, 2)?;
            let r = envprofile::preview_process(&envprofile::import(&PathBuf::from(&a[1]))?)?;
            Ok(format!("Read-only process environment preview. Values may contain secrets; review before sharing.\n{}",json::to_json_string_pretty(&r.changes)))
        }
        "environment-history" => {
            count(a, 2, 2)?;
            Ok(json::to_json_string_pretty(&envprofile::list_transactions(
                &PathBuf::from(&a[1]),
            )?))
        }
        "environment-receipt" => {
            count(a, 3, 3)?;
            Ok(json::to_json_string_pretty(&envprofile::load_transaction(
                &PathBuf::from(&a[1]),
                &a[2],
            )?))
        }
        "environment-user-preview" | "environment-apply" | "environment-undo" => {
            #[cfg(not(windows))]
            bail!("Persistent user environment support requires Windows");
            #[cfg(windows)]
            {
                let environment = envprofile::UserEnvironment;
                if a[0] == "environment-user-preview" {
                    count(a, 2, 2)?;
                    let profile = envprofile::import(&PathBuf::from(&a[1]))?;
                    return Ok(json::to_json_string_pretty(&envprofile::preview(
                        &profile,
                        &environment,
                    )?));
                }
                count(a, 3, 3)?;
                let data = PathBuf::from(&a[1]);
                eprintln!("Only HKCU developer environment variables are changed. Existing applications can retain older values. Machine values are unchanged. Receipts contain prior values/local paths; protect the data directory. Review this exact transaction. Undo requires matching identity and conflict checks; interrupted writes may require recovery inspection.");
                if a[0] == "environment-apply" {
                    let transaction = envprofile::prepare_user(
                        &envprofile::import(&PathBuf::from(&a[2]))?,
                        &environment,
                    )?;
                    eprintln!("{}", json::to_json_string_pretty(&transaction));
                    confirm("APPLY")?;
                    Ok(json::to_json_string_pretty(&envprofile::apply_user(
                        &data,
                        &transaction,
                        &environment,
                    )?))
                } else {
                    let transaction = envprofile::load_transaction(&data, &a[2])?;
                    eprintln!("{}", json::to_json_string_pretty(&transaction));
                    confirm("UNDO")?;
                    Ok(json::to_json_string_pretty(&envprofile::undo_user(
                        &data,
                        &transaction,
                        &environment,
                    )?))
                }
            }
        }
        "diagnostics" => {
            ensure!(
                a.len() == 1 || a.len() == 3,
                "Use diagnostics [PROFILE TARGET]"
            );
            let report = if a.len() == 3 {
                Some(devcheck::inspect(
                    &devcheck::Request::current(profile(&a[1])?, target(&a[2])?, String::new())?,
                    &p,
                )?)
            } else {
                None
            };
            diagnostic_bundle::preview(
                diagnostic_bundle::Sections {
                    runtime: true,
                    readiness: report.is_some(),
                    tools: report.is_some(),
                },
                report.as_ref(),
            )
        }
        "dependencies" => {
            ensure!(a.len() > 1, "Specify a dependencies subcommand");
            if a[1] == "catalog" {
                count(a, 2, 2)?;
                return Ok(json::to_json_string_pretty(&dependencies::catalog()?));
            }
            if a[1] == "receipts" {
                count(a, 3, 3)?;
                return Ok(json::to_json_string_pretty(&dependencies::recent_receipts(
                    &PathBuf::from(&a[2]),
                )?));
            }
            ensure!(a.len() >= 4, "Specify ROOT and package IDs or bundle path");
            let root = PathBuf::from(&a[2]);
            match a[1].as_str() {
                "verify" => dependencies::verify(&root, &a[3..], &p),
                "download" => dependencies::download(&root, &a[3..], &p),
                "import" => {
                    count(a, 4, 4)?;
                    dependencies::import_bundle(&root, &PathBuf::from(&a[3]), &p)
                }
                "export" => {
                    ensure!(a.len()>=6 && a.last().is_some_and(|s|s=="--accept-license"),"Export requires DEST, IDs and --accept-license after reviewing vendor redistribution terms");
                    dependencies::export_bundle(
                        &root,
                        &a[4..a.len() - 1],
                        &PathBuf::from(&a[3]),
                        true,
                        &p,
                    )
                }
                "review" | "install" => {
                    count(a, 4, 4)?;
                    let review = dependencies::review_install(&root, &a[3], false, &p)?;
                    if a[1] == "review" {
                        Ok(review.text().into())
                    } else {
                        eprintln!("{}", review.text());
                        confirm("INSTALL")?;
                        dependencies::install(&root, review, true, &p)
                    }
                }
                _ => bail!("Unknown dependencies subcommand"),
            }
        }
        "run" | "execute" => {
            ensure!(
                a.len() >= 6 && a[5] == "--",
                "Expected PROJECT EXE LOGDIR TIMEOUT -- literal arguments"
            );
            let review = build_run::review(build_run::Request {
                project: PathBuf::from(&a[1]),
                executable: PathBuf::from(&a[2]),
                logs: PathBuf::from(&a[3]),
                timeout_seconds: a[4].parse().context("Invalid timeout")?,
                arguments: a[6..].to_vec(),
            })?;
            if a[0] == "run" {
                Ok(review.text().into())
            } else {
                eprintln!("{}", review.text());
                confirm("RUN")?;
                let outcome = build_run::run(review, true, &p)?;
                ensure!(outcome.successful, "{}", outcome.summary);
                Ok(outcome.summary)
            }
        }
        _ => bail!("Unknown command; use --help. No action performed"),
    }
}
pub fn dispatch(args: &[String]) -> i32 {
    let mut args = args.to_vec();
    // Only consume an envelope flag before the literal executable argument delimiter.
    let end = args.iter().position(|s| s == "--").unwrap_or(args.len());
    let json_mode = args[..end].iter().any(|s| s == "--json");
    if json_mode {
        let mut i = 0;
        args.retain(|s| {
            let keep = !(i < end && s == "--json");
            i += 1;
            keep
        });
    }
    if args.is_empty() {
        eprintln!("Specify a command; use --help");
        return 2;
    }
    match report(&args) {
        Ok(text) => {
            if json_mode {
                use json::ToJson;
                println!(
                    "{}",
                    json::to_string(&json::Value::Object(vec![
                        ("schema".into(), 1u32.to_json()),
                        ("command".into(), args[0].to_json()),
                        ("ok".into(), true.to_json()),
                        ("report".into(), text.to_json())
                    ]))
                );
            } else {
                println!("{text}");
            }
            0
        }
        Err(e) => {
            eprintln!("{e:#}");
            if json_mode {
                use json::ToJson;
                println!(
                    "{}",
                    json::to_string(&json::Value::Object(vec![
                        ("schema".into(), 1u32.to_json()),
                        ("command".into(), args[0].to_json()),
                        ("ok".into(), false.to_json()),
                        ("error".into(), format!("{e:#}").to_json())
                    ]))
                );
            }
            if e.to_string().contains("Invalid argument count")
                || e.to_string().starts_with("Unknown")
            {
                2
            } else {
                1
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_legacy_and_extra_arguments() {
        assert!(report(&["programs".into()]).is_err());
        assert!(report(&["system".into(), "extra".into()]).is_err());
    }
    #[test]
    fn help_lists_only_supported_surfaces() {
        let h = report(&["--help".into()]).unwrap();
        for c in [
            "readiness",
            "triage",
            "environment-preview",
            "dependencies",
            "execute",
            "file-owners",
        ] {
            assert!(h.contains(c));
        }
    }
    #[test]
    fn invalid_execution_never_launches() {
        assert!(report(&["execute".into(), "relative".into()]).is_err());
    }
}
