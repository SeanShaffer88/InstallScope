//! Command-line interface for scripting and IT use.

use installscope_core::backup::{list_backups, restore_backup, BackupWriter};
use installscope_core::model::{human_size, InstallLog};
use installscope_core::platform::{self, Registry};
use installscope_core::refcount::{annotate_plan, RefLabel, ReferenceIndex};
use installscope_core::revert::{build_plan, execute_plan, ExecOptions};
use installscope_core::revert::{residue, residue_markdown};
use installscope_core::snapshot::{
    detect_program, diff, filter_changes, load_snapshot, save_snapshot, take_snapshot, Progress,
};
use installscope_core::store::Store;
use installscope_core::{programs, report};
use std::path::PathBuf;

const HELP: &str = "\
InstallScope — Windows Install, Uninstall & Developer Tools
Copyright (C) 2026 Sean Shaffer. GPLv3: free software, no warranty.

USAGE:
  installscope                                   Start the graphical interface
  installscope monitor [--name NAME] -- <installer> [args...]
                                            Snapshot, run the installer, wait,
                                            snapshot again, save an install log
  installscope snapshot <file.snap>              Save a snapshot of the system
  installscope diff <before.snap> <after.snap> [--name NAME]
                                            Compare two snapshots, save a log
  installscope logs [--group G] [--tag T] [--json]
                                            List saved install logs
  installscope tag <log-id> [--group G] [--tags a,b]
                                            Set a log's group and tags
  installscope programs [--all] [--trust] [--json]
                                            List installed programs (--trust adds
                                            each program's digital signature)
  installscope verify <file>                     Check a file's digital signature
                                            (exit 2 if unsigned or invalid)
  installscope revert <log-id> [--dry-run] [--remove-unlogged] [--no-backup] [--recycle]
                          [--close-programs] [--restart-explorer] [--keep-log]
                                            Undo an installation precisely
  installscope export <log-id> <html|json|reg|ps1|xml> <output-file>
  installscope repackage <log-id> <folder> [--no-setup]
                                            A silent installer for the recorded
                                            installation: one Setup.exe plus a
                                            folder with install.ps1, install.reg
                                            and uninstall.ps1
  installscope merge <log-id> <log-id>... [--name NAME]
                                            Combine logs into a new one (e.g. a
                                            program and its update)
  installscope learn-noise [--seconds N] [--apply] [--json]
                                            Take two snapshots N seconds apart
                                            (default 60) and list what changed
                                            on its own; --apply adds it to the
                                            exclusions
  installscope transfer export <out.installscope> <program>...
                                            Pack programs (by name) with their files
                                            and settings to move to another PC
  installscope transfer check <package>          Check a package against this PC
  installscope transfer import <package> [--overwrite]
                                            Restore a package (undo from backups)
  installscope export <log-id> portable <launcher.ini>
                                            PortableApps.com Launcher settings
                                            for what the log recorded
  installscope portable-build <installer> --name NAME [--version V] [--publisher P]
                 [--category C] [--silent] [--out DIR] [--here] [--defaults]
                 [--no-prompt] [--no-compile]
                                            Capture an installation in Windows
                                            Sandbox (or on this PC with --here)
                                            and build a PortableApps.com package
                                            with its launcher and .paf.exe
                                            installer
  installscope add-program <folder|program.exe> [--name NAME]
                                            List a program Windows doesn't know
                                            (e.g. portable) so it can be analyzed
                                            and uninstalled
  installscope components <log-id> [--json] [--ask-model] [--strike]
                                            Installer Decrapifier: the services,
                                            tasks and autoruns an installation
                                            added, judged; --strike removes the
                                            bloat (backed up)
  installscope orphans [--json]                  List registrations left by removed
                                            programs (pointing to missing files)
  installscope uninstall <name>... [--silent] [--leftovers safe|none] [--dry-run]
                                            Uninstall programs one after another,
                                            then remove safe leftovers (backed up)
  installscope clean [--categories a,b] [--list] [--dry-run]
                                            Clean junk (default: the safe kinds)
  installscope schedule-clean daily|weekly|monthly|off [--categories a,b]
                                            Clean junk automatically
  installscope startup [--json]                  List startup programs
  installscope startup enable|disable <name>     Turn a startup program on or off
  installscope watch                             Background auto-monitoring (started
                                            at sign-in when turned on in Settings)
  installscope check-clean <log-id>              Check that nothing recorded in a log
                                            is still on the PC (exit 2 if so)
  installscope restore-point [list]              List System Restore points
  installscope restore-point create [DESCRIPTION]
  installscope restore-point delete <number>
  installscope restore-point rollback <number> --yes
                                            Restart Windows into a restore point
  installscope backups                           List backups
  installscope restore <backup-id>               Undo a removal from its backup
  installscope --version | --help

  Any command that takes snapshots accepts --profile NAME to use a saved
  monitoring profile (Settings > Monitoring scope).
  monitor and diff also accept --json to print the saved log's summary.

Exit codes: 0 success, 1 error, 2 finished with failures / findings.

Typical CI use (installer QA):
  installscope monitor --name MyApp --no-prompt --json -- dist\\MyApp-Setup.exe /S
  installscope revert <log-id> --keep-log
  installscope check-clean <log-id>";

fn arg_value(args: &[String], flag: &str) -> Option<String> {
    args.iter()
        .position(|a| a == flag)
        .and_then(|i| args.get(i + 1).cloned())
}

/// Find one program by name: exact (ignoring case) first, then a unique
/// partial match.
fn pick_program<'a>(
    progs: &'a [installscope_core::programs::InstalledProgram],
    name: &str,
) -> anyhow::Result<&'a installscope_core::programs::InstalledProgram> {
    if let Some(p) = progs.iter().find(|p| p.name.eq_ignore_ascii_case(name)) {
        return Ok(p);
    }
    let low = name.to_lowercase();
    let hits: Vec<_> = progs
        .iter()
        .filter(|p| p.name.to_lowercase().contains(&low))
        .collect();
    match hits.len() {
        0 => anyhow::bail!("no installed program matches"),
        1 => Ok(hits[0]),
        _ => anyhow::bail!(
            "several programs match: {}",
            hits.iter()
                .map(|p| p.name.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        ),
    }
}

/// Where scheduled cleaning records what it freed.
fn st_log_path() -> anyhow::Result<PathBuf> {
    Ok(store()?.root.join("cleaning.log"))
}

fn has(args: &[String], flag: &str) -> bool {
    args.iter().any(|a| a == flag)
}

/// `installscope portable-build`: capture an installation and build a
/// PortableApps.com package.
fn portable_build(args: &[String], reg: &dyn Registry) -> anyhow::Result<i32> {
    use installscope_core::pafbuild::{self, AppDetails, Job};
    let installer = args
        .get(1)
        .filter(|a| !a.starts_with("--"))
        .ok_or_else(|| {
            anyhow::anyhow!("usage: installscope portable-build <installer> --name NAME")
        })?;
    let installer =
        std::fs::canonicalize(installer).map_err(|e| anyhow::anyhow!("{installer}: {e}"))?;
    let installer = installer
        .to_string_lossy()
        .trim_start_matches("\\\\?\\")
        .to_string();
    let st = store()?;
    let settings = load_settings(&st);
    let name = arg_value(args, "--name").ok_or_else(|| anyhow::anyhow!("--name is required"))?;
    let details = AppDetails {
        name,
        version: arg_value(args, "--version").unwrap_or_default(),
        publisher: arg_value(args, "--publisher").unwrap_or_default(),
        category: arg_value(args, "--category").unwrap_or_else(|| "Utilities".into()),
        packager: settings.packager_name.clone(),
        ..Default::default()
    };
    let out_dir = arg_value(args, "--out")
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_default());
    let job = Job {
        details,
        installer,
        silent: has(args, "--silent"),
        out_dir: out_dir.to_string_lossy().to_string(),
        include_defaults: has(args, "--defaults"),
        learn_seconds: 30,
        sandbox: false,
        settings: settings.clone(),
        preview_only: false,
    };
    let here = has(args, "--here") || !pafbuild::sandbox_available();
    let (pkg_dir, app_id) = if here {
        if !has(args, "--here") {
            eprintln!("Windows Sandbox isn't turned on, so the installation runs on this PC.");
        }
        let report = |p: &str| eprintln!("{p}…");
        let no_prompt = has(args, "--no-prompt");
        let ask = || {
            if no_prompt {
                return;
            }
            eprintln!("When the installation has finished, start the program once, set it up, close it, then press Enter.");
            let mut s = String::new();
            let _ = std::io::stdin().read_line(&mut s);
        };
        let (log, pkg) = pafbuild::capture(&job, reg, &report, &ask, &Progress::default())?;
        st.save_log(&log)?;
        eprintln!(
            "The installation was recorded as install log {}; remove it with: installscope revert {}",
            log.id, log.id
        );
        for w in &pkg.warnings {
            eprintln!("note: {w}");
        }
        (PathBuf::from(&pkg.dir), pkg.app_id)
    } else {
        let work = st
            .root
            .join("builder")
            .join(installscope_core::util::new_id());
        pafbuild::start_sandbox(&job, &work)?;
        eprintln!("Windows Sandbox is starting. Follow the steps in the sandbox window.");
        let mut last = String::new();
        let status = loop {
            std::thread::sleep(std::time::Duration::from_secs(1));
            if let Some(s) = pafbuild::read_status(&work) {
                if s.phase != last {
                    eprintln!("{}…", s.phase);
                    last = s.phase.clone();
                }
                if s.done {
                    break s;
                }
            }
        };
        if let Some(e) = status.error {
            anyhow::bail!(e);
        }
        let pkg = status
            .package
            .ok_or_else(|| anyhow::anyhow!("no package was made"))?;
        for w in &pkg.warnings {
            eprintln!("note: {w}");
        }
        if let Ok(b) = std::fs::read(work.join("install-log.json")) {
            if let Ok(log) = installscope_core::json::compat::from_slice::<InstallLog>(&b) {
                let _ = st.save_log(&log);
            }
        }
        (
            pafbuild::collect_sandbox_package(&work, &pkg, &out_dir)?,
            pkg.app_id,
        )
    };
    println!("Package: {}", pkg_dir.display());
    if let Ok(info) =
        std::fs::read_to_string(pkg_dir.join("Other").join("Source").join("Readme.txt"))
    {
        if let Some((_, check)) = info.split_once("Portability check:") {
            eprintln!("Portability check:{}", check.trim_end());
        }
    }
    if has(args, "--no-compile") {
        return Ok(0);
    }
    let tools = pafbuild::find_tools(&settings.portableapps_dir);
    let c = pafbuild::compile(&pkg_dir, &app_id, &tools, &st.root, &|p| eprintln!("{p}…"))?;
    if let Some(l) = c.launcher {
        println!("Launcher: {}", l.display());
    }
    if let Some(p) = c.paf {
        println!("Installer: {}", p.display());
    }
    Ok(0)
}

/// `--profile NAME` (any command): use a saved monitoring profile.
static PROFILE: std::sync::OnceLock<String> = std::sync::OnceLock::new();

fn load_settings(st: &Store) -> installscope_core::settings::Settings {
    let mut s = st.load_settings();
    if let Some(p) = PROFILE.get() {
        if !s.use_profile(p) {
            eprintln!("warning: no profile named {p}; using the current scope");
        }
    }
    s
}

fn store() -> anyhow::Result<Store> {
    Store::open()
}

fn save_new_log(
    store: &Store,
    name: &str,
    installer: &str,
    mut changes: installscope_core::model::ChangeSet,
) -> anyhow::Result<InstallLog> {
    let settings = load_settings(store);
    filter_changes(
        &mut changes,
        &settings.effective_registry_exclusions(),
        &settings.effective_file_exclusions(),
    );
    let mut log = InstallLog {
        id: installscope_core::util::new_id(),
        name: name.to_string(),
        created: installscope_core::util::now_string(),
        installer: installer.to_string(),
        ..Default::default()
    };
    if let Some(d) = detect_program(&changes) {
        if name.is_empty() {
            log.name = d.name;
        }
        log.publisher = d.publisher;
        log.version = d.version;
        log.uninstall_key = d.key;
    }
    if log.name.is_empty() {
        log.name = "Unnamed program".into();
    }
    log.changes = changes;
    store.save_log(&log)?;
    let st = log.changes.stats();
    eprintln!(
        "Saved install log {} for \"{}\": {} files, {} folders, {} keys, {} values added, {} changed ({})",
        log.id,
        log.name,
        st.files_added,
        st.dirs_added,
        st.keys_added,
        st.values_added,
        st.values_modified + st.files_modified,
        human_size(st.bytes_added)
    );
    Ok(log)
}

fn snapshot_now(
    reg: &dyn Registry,
    store: &Store,
) -> anyhow::Result<installscope_core::model::Snapshot> {
    let settings = load_settings(store);
    let p = Progress::default();
    eprintln!("Taking snapshot…");
    let s = take_snapshot(reg, &settings, &p)?;
    eprintln!(
        "  {} registry keys, {} files and folders",
        s.key_count(),
        s.file_count()
    );
    // Anything unreadable is missing from the snapshot, and so from any
    // log made from it: say so.
    if !s.warnings.is_empty() {
        eprintln!("  {} places could not be read:", s.warnings.len());
        for w in s.warnings.iter().take(10) {
            eprintln!("    {w}");
        }
    }
    Ok(s)
}

fn log_summary_json(l: &InstallLog) -> installscope_core::json::Value {
    installscope_core::json!({
        "id": l.id,
        "name": l.name,
        "publisher": l.publisher,
        "version": l.version,
        "created": l.created,
        "uninstall_key": l.uninstall_key,
        "stats": l.changes.stats(),
    })
}

pub fn run(args: Vec<String>) -> i32 {
    match run_inner(args) {
        Ok(code) => code,
        Err(e) => {
            eprintln!("error: {e:#}");
            1
        }
    }
}

fn run_inner(args: Vec<String>) -> anyhow::Result<i32> {
    let cmd = args.first().map(String::as_str).unwrap_or("--help");
    if !matches!(cmd, "--help" | "-h" | "help" | "--version" | "-V") && !platform::is_elevated() {
        eprintln!("warning: not running as Administrator; system-wide changes may fail.");
    }
    if let Some(p) = arg_value(&args, "--profile") {
        let _ = PROFILE.set(p);
    }
    let reg = platform::system_registry();
    match cmd {
        "--help" | "-h" | "help" => println!("{HELP}"),
        "--version" | "-V" => println!("InstallScope {}", installscope_core::APP_VERSION),
        "snapshot" => {
            let out = args
                .get(1)
                .ok_or_else(|| anyhow::anyhow!("missing output file"))?;
            let st = store()?;
            let s = snapshot_now(reg.as_ref(), &st)?;
            save_snapshot(&PathBuf::from(out), &s)?;
            println!("Saved {out}");
        }
        "diff" => {
            let (a, b) = (args.get(1), args.get(2));
            let (Some(a), Some(b)) = (a, b) else {
                anyhow::bail!("usage: installscope diff <before> <after>")
            };
            let before = load_snapshot(&PathBuf::from(a))?;
            let after = load_snapshot(&PathBuf::from(b))?;
            let name = arg_value(&args, "--name").unwrap_or_default();
            let st = store()?;
            let mut log = save_new_log(&st, &name, "", diff(&before, &after))?;
            // Used by background monitoring (`watch`), which records here.
            if has(&args, "--auto") {
                log.tags = vec![installscope_core::autowatch::TAG.to_string()];
                log.notes = "Recorded automatically by background monitoring.".into();
                st.save_log(&log)?;
                eprintln!("Recorded {}", log.name);
            }
            if has(&args, "--json") {
                println!("{}", log_summary_json(&log));
            }
        }
        "monitor" => {
            let sep = args.iter().position(|a| a == "--").ok_or_else(|| {
                anyhow::anyhow!("usage: installscope monitor [--name N] -- <installer> [args]")
            })?;
            let installer: Vec<String> = args[sep + 1..].to_vec();
            if installer.is_empty() {
                anyhow::bail!("no installer given");
            }
            let name = arg_value(&args[..sep], "--name").unwrap_or_default();
            let st = store()?;
            if let Some(p) = st.pending() {
                anyhow::bail!(
                    "monitoring of {} is already in progress; finish it in InstallScope first",
                    if p.name.is_empty() {
                        "an installation".to_string()
                    } else {
                        p.name
                    }
                );
            }
            let before = snapshot_now(reg.as_ref(), &st)?;
            // Marks the installation as monitored, so background monitoring
            // leaves it alone; cleared however this command ends.
            st.save_pending(
                &installscope_core::store::PendingSession {
                    name: name.clone(),
                    started: installscope_core::util::now_string(),
                    installer: installer[0].clone(),
                    trial: false,
                },
                &before,
            )?;
            struct ClearPending<'a>(&'a Store);
            impl Drop for ClearPending<'_> {
                fn drop(&mut self) {
                    self.0.clear_pending();
                }
            }
            let _pending = ClearPending(&st);
            let cmdline = installer
                .iter()
                .map(|a| {
                    if a.contains(' ') {
                        format!("\"{a}\"")
                    } else {
                        a.clone()
                    }
                })
                .collect::<Vec<_>>()
                .join(" ");
            eprintln!("Running {cmdline}");
            let child = platform::spawn_command_line(&cmdline)?;
            let code = child.wait();
            eprintln!("Installer exited with code {code}.");
            // Bootstrappers (WiX Burn and others) can exit while Windows
            // Installer is still working for them; wait until it's done.
            let settle = std::time::Instant::now();
            if platform::installer_busy() {
                eprintln!("Waiting for Windows Installer to finish…");
            }
            while platform::installer_busy() && settle.elapsed().as_secs() < 1800 {
                std::thread::sleep(std::time::Duration::from_secs(2));
            }
            if !has(&args, "--no-prompt") {
                eprintln!("If the installer is still working in another window, finish it, then press Enter.");
                let mut s = String::new();
                let _ = std::io::stdin().read_line(&mut s);
            }
            let after = snapshot_now(reg.as_ref(), &st)?;
            let log = save_new_log(&st, &name, &installer[0], diff(&before, &after))?;
            if has(&args, "--json") {
                println!("{}", log_summary_json(&log));
            }
        }
        "logs" => {
            let group = arg_value(&args, "--group");
            let tag = arg_value(&args, "--tag");
            let logs: Vec<_> = store()?
                .list_logs()
                .into_iter()
                .filter(|l| {
                    group
                        .as_ref()
                        .is_none_or(|g| l.group.eq_ignore_ascii_case(g))
                })
                .filter(|l| {
                    tag.as_ref()
                        .is_none_or(|t| l.tags.iter().any(|x| x.eq_ignore_ascii_case(t)))
                })
                .collect();
            if has(&args, "--json") {
                let items: Vec<_> = logs
                    .iter()
                    .map(|l| {
                        let st = l.changes.stats();
                        installscope_core::json!({
                            "id": l.id, "name": l.name, "publisher": l.publisher,
                            "version": l.version, "created": l.created,
                            "group": l.group, "tags": l.tags,
                            "files_added": st.files_added, "bytes_added": st.bytes_added,
                        })
                    })
                    .collect();
                println!(
                    "{}",
                    installscope_core::json::compat::to_string_pretty(&items)?
                );
            } else {
                for l in &logs {
                    let st = l.changes.stats();
                    let mut extra = String::new();
                    if !l.group.is_empty() {
                        extra.push_str(&format!("  [{}]", l.group));
                    }
                    if !l.tags.is_empty() {
                        extra.push_str(&format!("  #{}", l.tags.join(" #")));
                    }
                    println!(
                        "{}  {}  {}  ({} files, {}){extra}",
                        l.id,
                        l.created,
                        l.name,
                        st.files_added,
                        human_size(st.bytes_added)
                    );
                }
            }
        }
        "tag" => {
            let id = args.get(1).ok_or_else(|| {
                anyhow::anyhow!("usage: installscope tag <log-id> [--group G] [--tags a,b]")
            })?;
            let st = store()?;
            let mut log = st.load_log(id)?;
            if let Some(g) = arg_value(&args, "--group") {
                log.group = g.trim().to_string();
            }
            if let Some(t) = arg_value(&args, "--tags") {
                log.tags = installscope_core::model::parse_tags(&t);
            }
            st.save_log(&log)?;
            println!(
                "{}: group \"{}\", tags [{}]",
                log.name,
                log.group,
                log.tags.join(", ")
            );
        }
        "transfer" => {
            let sub = args.get(1).map(String::as_str).unwrap_or("");
            let st = store()?;
            match sub {
                "export" => {
                    let out = args
                        .get(2)
                        .ok_or_else(|| anyhow::anyhow!("missing output file"))?;
                    let wanted: Vec<String> = args
                        .iter()
                        .skip(3)
                        .filter(|a| !a.starts_with("--"))
                        .map(|a| a.to_lowercase())
                        .collect();
                    if wanted.is_empty() {
                        anyhow::bail!("name at least one program");
                    }
                    let all = programs::list_programs(reg.as_ref(), false);
                    let chosen: Vec<_> = all
                        .into_iter()
                        .filter(|p| wanted.iter().any(|w| p.name.to_lowercase().contains(w)))
                        .collect();
                    if chosen.is_empty() {
                        anyhow::bail!("no installed program matches");
                    }
                    let logs = st.list_logs();
                    let changes: Vec<_> = chosen
                        .iter()
                        .map(|p| programs::log_for(&logs, p).map(|l| l.changes.clone()))
                        .collect();
                    let c = installscope_core::transfer::collect(
                        &chosen,
                        &changes,
                        reg.as_ref(),
                        &|s| eprintln!("  {s}"),
                    );
                    let m = installscope_core::transfer::export(
                        &c,
                        reg.as_ref(),
                        &PathBuf::from(out),
                        &|_| {},
                    )?;
                    for w in &m.warnings {
                        eprintln!("warning: {w}");
                    }
                    println!(
                        "Saved {out}: {} programs, {} files ({}), {} registry entries",
                        m.programs.len(),
                        m.files.len(),
                        human_size(m.bytes()),
                        m.keys.len() + m.values.len()
                    );
                }
                "check" | "import" => {
                    let pkg = PathBuf::from(
                        args.get(2)
                            .ok_or_else(|| anyhow::anyhow!("missing package"))?,
                    );
                    let m = installscope_core::transfer::read_manifest(&pkg)?;
                    let c = installscope_core::transfer::check(
                        &m,
                        reg.as_ref(),
                        &installscope_core::transfer::Vars::current(),
                    );
                    println!("From {} ({}), {}", m.computer, m.user, m.created);
                    for p in &m.programs {
                        println!("  {} {}", p.name, p.version);
                    }
                    for p in &c.problems {
                        println!("problem: {p}");
                    }
                    for w in &c.warnings {
                        println!("warning: {w}");
                    }
                    if !c.conflicts.is_empty() {
                        println!(
                            "{} files already exist (use --overwrite to replace them)",
                            c.conflicts.len()
                        );
                    }
                    if sub == "check" || !c.problems.is_empty() {
                        return Ok(if c.problems.is_empty() { 0 } else { 2 });
                    }
                    let names: Vec<String> = m.programs.iter().map(|p| p.name.clone()).collect();
                    let mut bw = BackupWriter::new(
                        &st.backups_dir(),
                        &format!("Transfer: {}", names.join(", ")),
                    )?;
                    let res = installscope_core::transfer::restore(
                        &pkg,
                        reg.as_ref(),
                        &mut bw,
                        has(&args, "--overwrite"),
                        &|s| eprintln!("  {s}"),
                    )?;
                    let b = bw.finish()?;
                    let failed = res.iter().filter(|r| !r.ok).count();
                    for r in res.iter().filter(|r| !r.ok) {
                        eprintln!("failed: {} — {}", r.description, r.message);
                    }
                    println!("Restored. Undo with: installscope restore {}", b.id);
                    return Ok(if failed == 0 { 0 } else { 2 });
                }
                _ => anyhow::bail!("usage: installscope transfer export|check|import …"),
            }
        }
        "uninstall" => {
            let names: Vec<&String> = args[1..]
                .iter()
                .filter(|a| !a.starts_with("--"))
                .collect::<Vec<_>>();
            let names: Vec<String> = {
                // Drop values that belong to flags.
                let mut out = Vec::new();
                let mut skip = false;
                for a in &args[1..] {
                    if skip {
                        skip = false;
                        continue;
                    }
                    if a == "--leftovers" {
                        skip = true;
                        continue;
                    }
                    if !a.starts_with("--") {
                        out.push(a.clone());
                    }
                }
                let _ = names;
                out
            };
            if names.is_empty() {
                anyhow::bail!("name at least one program (see `installscope programs`)");
            }
            let silent_mode = has(&args, "--silent");
            let dry = has(&args, "--dry-run");
            let leftovers = arg_value(&args, "--leftovers").unwrap_or_else(|| "safe".into());
            let st = store()?;
            let mut failures = 0;
            let mut reboot = false;
            for name in &names {
                let mut progs = programs::list_programs(reg.as_ref(), false);
                if let Ok(st) = store() {
                    progs.extend(programs::load_manual(&st.root));
                }
                let p = match pick_program(&progs, name) {
                    Ok(p) => p.clone(),
                    Err(e) => {
                        eprintln!("{name}: {e}");
                        failures += 1;
                        continue;
                    }
                };
                let engine = installscope_core::silent::detect(&p);
                let cmd = if silent_mode {
                    installscope_core::silent::silent_command(&p, engine)
                } else {
                    p.uninstall_command(false)
                };
                let Some(cmd) = cmd else {
                    eprintln!(
                        "{}: no {}uninstaller available",
                        p.name,
                        if silent_mode { "silent " } else { "" }
                    );
                    failures += 1;
                    continue;
                };
                println!("Uninstalling {} ({})…", p.name, engine.label());
                if dry {
                    println!("  would run: {cmd}");
                    continue;
                }
                while platform::installer_busy() {
                    std::thread::sleep(std::time::Duration::from_secs(5));
                }
                let code = platform::spawn_command_line(&cmd)?.wait();
                match installscope_core::silent::exit_meaning(engine, code) {
                    installscope_core::silent::ExitMeaning::RebootRequired => reboot = true,
                    installscope_core::silent::ExitMeaning::Success => {}
                    m => {
                        eprintln!("  the uninstaller reported {m:?} (exit code {code})");
                    }
                }
                std::thread::sleep(std::time::Duration::from_secs(2));
                if reg.exists(&p.key) {
                    eprintln!(
                        "  {} is still installed; skipping the leftover scan",
                        p.name
                    );
                    failures += 1;
                    continue;
                }
                if leftovers == "none" {
                    continue;
                }
                let mut plan = installscope_core::leftovers::find_leftovers(
                    &p,
                    reg.as_ref(),
                    &installscope_core::leftovers::LeftoverScope::default(),
                );
                // Unattended: only items labelled Safe.
                for i in &mut plan.items {
                    if installscope_core::revert::risk(i) != installscope_core::revert::Risk::Safe {
                        i.selected = false;
                    }
                }
                if plan.selected() == 0 {
                    println!("  no leftovers found");
                    continue;
                }
                let mut w =
                    BackupWriter::new(&st.backups_dir(), &format!("Leftovers of {}", p.name))?;
                let results =
                    execute_plan(&plan, reg.as_ref(), Some(&mut w), &ExecOptions::default());
                let b = w.finish()?;
                let ok = results.iter().filter(|r| r.ok).count();
                println!(
                    "  removed {ok} leftovers (backup {}; restore with: installscope restore {})",
                    b.id, b.id
                );
                if ok < results.len() {
                    failures += 1;
                }
            }
            if reboot {
                println!("Restart Windows to finish removing everything.");
            }
            return Ok(if failures > 0 { 2 } else { 0 });
        }
        "clean" => {
            let cats = installscope_core::cleaner::categories();
            if has(&args, "--list") {
                for c in &cats {
                    let r = installscope_core::cleaner::scan(c);
                    println!(
                        "{:<20} {:>10}  {}{}",
                        c.id,
                        human_size(r.bytes),
                        if c.default_on && !installscope_core::cleaner::is_advanced(c.id) {
                            "default"
                        } else {
                            ""
                        },
                        if c.needs_admin {
                            " (administrator)"
                        } else {
                            ""
                        }
                    );
                }
                return Ok(0);
            }
            let wanted: Vec<String> = match arg_value(&args, "--categories") {
                Some(v) => v
                    .split(',')
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .collect(),
                None => cats
                    .iter()
                    .filter(|c| c.default_on && !installscope_core::cleaner::is_advanced(c.id))
                    .map(|c| c.id.to_string())
                    .collect(),
            };
            let elevated = platform::is_elevated();
            let mut total = 0u64;
            for id in &wanted {
                let Some(c) = cats.iter().find(|c| c.id == id) else {
                    eprintln!("unknown category: {id} (see `installscope clean --list`)");
                    continue;
                };
                if c.needs_admin && !elevated {
                    eprintln!("{id}: skipped (needs administrator rights)");
                    continue;
                }
                let r = if has(&args, "--dry-run") {
                    installscope_core::cleaner::scan(c)
                } else {
                    installscope_core::cleaner::clean(c)
                };
                total += r.bytes;
                if !has(&args, "--scheduled") {
                    println!("{:<20} {:>10}  {} files", id, human_size(r.bytes), r.files);
                }
            }
            if has(&args, "--scheduled") {
                // Leave a trace for the user in the data folder.
                let line = format!(
                    "{}  cleaned {}\n",
                    installscope_core::util::now_string(),
                    human_size(total)
                );
                let path = st_log_path()?;
                let mut old = std::fs::read_to_string(&path).unwrap_or_default();
                old.push_str(&line);
                let keep: Vec<&str> = old.lines().rev().take(100).collect();
                let text: String = keep.into_iter().rev().map(|l| format!("{l}\n")).collect();
                let _ = std::fs::write(&path, text);
            } else {
                println!("Total: {}", human_size(total));
            }
        }
        "schedule-clean" => {
            let what = args.get(1).map(|s| s.as_str()).unwrap_or("");
            if what == "off" {
                installscope_core::schedule::remove()?;
                println!("Automatic cleaning turned off.");
                return Ok(0);
            }
            let freq = match what {
                "daily" => installscope_core::schedule::Frequency::Daily,
                "weekly" => installscope_core::schedule::Frequency::Weekly,
                "monthly" => installscope_core::schedule::Frequency::Monthly,
                _ => anyhow::bail!(
                    "usage: installscope schedule-clean daily|weekly|monthly|off [--categories a,b]"
                ),
            };
            let cats: Vec<String> = match arg_value(&args, "--categories") {
                Some(v) => v
                    .split(',')
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .collect(),
                None => installscope_core::cleaner::categories()
                    .iter()
                    .filter(|c| c.default_on && !installscope_core::cleaner::is_advanced(c.id))
                    .map(|c| c.id.to_string())
                    .collect(),
            };
            let refs: Vec<&str> = cats.iter().map(|s| s.as_str()).collect();
            installscope_core::schedule::set(&refs, freq, platform::is_elevated())?;
            println!("Junk will be cleaned {} ({}).", what, cats.join(", "));
        }
        "startup" => {
            let entries = installscope_core::startup::list_startup(reg.as_ref());
            match args.get(1).map(|s| s.as_str()) {
                Some(op @ ("enable" | "disable")) => {
                    let name = args
                        .get(2)
                        .ok_or_else(|| anyhow::anyhow!("name the startup program"))?;
                    let e = entries
                        .iter()
                        .find(|e| e.name.eq_ignore_ascii_case(name))
                        .ok_or_else(|| anyhow::anyhow!("no startup program called {name}"))?;
                    if op == "disable" && installscope_core::startup::is_essential(e) {
                        anyhow::bail!("{} is needed by Windows, security software or hardware and can't be disabled", e.name);
                    }
                    let st = store()?;
                    let mut w = BackupWriter::new(
                        &st.backups_dir(),
                        &format!("Startup: {} {}", op, e.name),
                    )?;
                    installscope_core::startup::set_enabled(
                        reg.as_ref(),
                        e,
                        op == "enable",
                        Some(&mut w),
                    )?;
                    w.finish()?;
                    println!("{} {}d.", e.name, op);
                }
                _ => {
                    if has(&args, "--json") {
                        let items: Vec<_> = entries
                            .iter()
                            .map(|e| installscope_core::json!({
                                "name": e.name, "command": e.command, "location": e.location,
                                "enabled": e.enabled, "all_users": e.all_users, "missing": e.missing,
                            }))
                            .collect();
                        println!(
                            "{}",
                            installscope_core::json::compat::to_string_pretty(&items)?
                        );
                    } else {
                        let boot = installscope_core::boot::read();
                        if let Some(ms) = boot.as_ref().and_then(|b| b.last_boot_ms) {
                            println!("Last startup took {:.0} s.", ms as f64 / 1000.0);
                        }
                        for e in &entries {
                            let impact = boot
                                .as_ref()
                                .and_then(|b| b.impact_of(&e.command))
                                .map(|i| {
                                    format!("  [impact: {}, {} ms]", i.level().label(), i.avg_ms)
                                })
                                .unwrap_or_default();
                            println!(
                                "[{}] {}{}{impact}  {}",
                                if e.enabled { "on " } else { "off" },
                                e.name,
                                if e.missing { " (missing)" } else { "" },
                                e.command
                            );
                        }
                    }
                }
            }
        }
        "watch" => {
            #[cfg(windows)]
            platform::windows::background_priority();
            let st = store()?;
            let base_path = st.snapshots_dir().join("auto-baseline.snap");
            let after_path = st.snapshots_dir().join("auto-after.snap");
            let poll = std::time::Duration::from_secs(30);
            // Snapshots and comparisons run in a short-lived copy of InstallScope:
            // building a snapshot of every monitored place can take a
            // gigabyte or more, and this process stays running for days.
            // The memory goes back to Windows when the copy exits.
            let exe = std::env::current_exe()?.to_string_lossy().into_owned();
            let in_child = |what: &str, argv: &[&str]| -> anyhow::Result<()> {
                let (code, out) = platform::run_hidden(&exe, argv)?;
                if code != 0 {
                    let last = out
                        .lines()
                        .rev()
                        .find(|l| !l.trim().is_empty())
                        .unwrap_or("");
                    anyhow::bail!("{what} failed ({code}): {last}");
                }
                Ok(())
            };
            let base_arg = base_path.to_string_lossy().into_owned();
            let after_arg = after_path.to_string_lossy().into_owned();
            let take_baseline = |_st: &Store| -> anyhow::Result<()> {
                in_child("baseline snapshot", &["snapshot", &base_arg])
            };
            while platform::installer_busy() {
                std::thread::sleep(poll);
            }
            take_baseline(&st)?;
            // The kernel signals changes to these keys; nothing runs between
            // them except the five-minute resource-use sample.
            const UNINSTALL_ROOTS: [&str; 3] = [
                "HKLM\\SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\Uninstall",
                "HKLM\\SOFTWARE\\WOW6432Node\\Microsoft\\Windows\\CurrentVersion\\Uninstall",
                "HKCU\\Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall",
            ];
            let watch = platform::RegistryWatch::new(&UNINSTALL_ROOTS)?;
            let usage_every = std::time::Duration::from_secs(300);
            let rebaseline_every = std::time::Duration::from_secs(24 * 3600);
            let mut next_usage = std::time::Instant::now() + usage_every;
            let mut keys = installscope_core::autowatch::uninstall_keys(reg.as_ref());
            let mut base_time = std::time::Instant::now();
            let mut stale = false;
            loop {
                // Short checks only while something is in progress (manual
                // monitoring, or a baseline waiting for an installer).
                let until_usage = next_usage.saturating_duration_since(std::time::Instant::now());
                let timeout = if stale || st.pending().is_some() {
                    until_usage.min(poll)
                } else {
                    until_usage
                };
                let event = watch.wait(Some(timeout))?;
                if std::time::Instant::now() >= next_usage {
                    installscope_core::usage::sample_and_save(&st.root, 290);
                    next_usage = std::time::Instant::now() + usage_every;
                }
                // Manual monitoring records this installation itself.
                if st.pending().is_some() {
                    stale = true;
                    keys = installscope_core::autowatch::uninstall_keys(reg.as_ref());
                    continue;
                }
                if event == platform::WatchEvent::Timeout
                    && !stale
                    && base_time.elapsed() < rebaseline_every
                {
                    continue;
                }
                let now = installscope_core::autowatch::uninstall_keys(reg.as_ref());
                let new = installscope_core::autowatch::added(&keys, &now);
                if new.is_empty() {
                    if now != keys {
                        // Something was uninstalled: start from a fresh baseline.
                        keys = now;
                        stale = true;
                    }
                    if (stale || base_time.elapsed() > rebaseline_every)
                        && !platform::installer_busy()
                    {
                        take_baseline(&st)?;
                        keys = installscope_core::autowatch::uninstall_keys(reg.as_ref());
                        base_time = std::time::Instant::now();
                        stale = false;
                    }
                    continue;
                }
                // Wait for the installation to settle: no installer running
                // and no change to the program list for 20 seconds (max. 30 min).
                let started = std::time::Instant::now();
                let mut last = now;
                loop {
                    let ev = watch.wait(Some(std::time::Duration::from_secs(20)))?;
                    let again = installscope_core::autowatch::uninstall_keys(reg.as_ref());
                    let settled = ev == platform::WatchEvent::Timeout
                        && again == last
                        && !platform::installer_busy();
                    last = again;
                    if settled || started.elapsed() > std::time::Duration::from_secs(1800) {
                        break;
                    }
                }
                in_child("snapshot", &["snapshot", &after_arg])?;
                // Monitoring may have started while it settled.
                if st.pending().is_none() {
                    in_child("comparison", &["diff", &base_arg, &after_arg, "--auto"])?;
                }
                std::fs::rename(&after_path, &base_path)?;
                keys = last;
                base_time = std::time::Instant::now();
                stale = false;
            }
        }
        "components" => {
            // Installer Decrapifier on the command line.
            let id = args.get(1).ok_or_else(|| {
                anyhow::anyhow!(
                    "usage: installscope components <log-id> [--json] [--ask-model] [--strike]"
                )
            })?;
            let st = store()?;
            let mut log = st.load_log(id)?;
            let ctx = installscope_core::decrap::context_for(&log);
            let mut comps = installscope_core::decrap::components(
                &log.changes,
                &ctx,
                &installscope_core::decrap::read_task_command,
            );
            if has(&args, "--ask-model") {
                let s = load_settings(&st);
                if s.local_model_url.is_empty() {
                    anyhow::bail!("no local model is set (Settings > Installer Decrapifier)");
                }
                let n = installscope_core::llm::judge_unknowns(
                    &s.local_model_url,
                    &s.local_model_name,
                    &log.name,
                    &log.publisher,
                    &mut comps,
                )?;
                eprintln!("The local model judged {n} component(s).");
            }
            if has(&args, "--json") {
                println!(
                    "{}",
                    installscope_core::json::compat::to_string_pretty(&comps)?
                );
            } else {
                println!("{}", installscope_core::decrap::summary(&log.name, &comps));
                for c in &comps {
                    println!(
                        "[{}] {:<24} {} — {}",
                        if c.verdict.strike_by_default() {
                            "x"
                        } else {
                            " "
                        },
                        c.kind.label(),
                        c.name,
                        c.verdict.label()
                    );
                    for r in &c.reasons {
                        println!("      {r}");
                    }
                }
            }
            if has(&args, "--strike") {
                let chosen: Vec<&installscope_core::decrap::Component> = comps
                    .iter()
                    .filter(|c| c.verdict.strike_by_default())
                    .collect();
                if chosen.is_empty() {
                    println!("Nothing to strike.");
                } else {
                    let mut w = BackupWriter::new(
                        &st.backups_dir(),
                        &format!("Strike background items of {}", log.name),
                    )?;
                    let results = installscope_core::decrap::strike(
                        &mut log,
                        &chosen,
                        reg.as_ref(),
                        Some(&mut w),
                    );
                    let b = w.finish()?;
                    for r in &results {
                        println!(
                            "[{}] {} — {}",
                            if r.ok { " OK " } else { "FAIL" },
                            r.description,
                            r.message
                        );
                    }
                    st.save_log(&log)?;
                    println!(
                        "Backup: {} (restore with: installscope restore {})",
                        b.id, b.id
                    );
                    if results.iter().any(|r| !r.ok) {
                        return Ok(2);
                    }
                }
            }
        }
        "check-clean" => {
            let id = args
                .get(1)
                .ok_or_else(|| anyhow::anyhow!("usage: installscope check-clean <log-id>"))?;
            let log = store()?.load_log(id)?;
            let left = residue(&log.changes, reg.as_ref());
            if left.items.is_empty() {
                println!("Nothing from {} is left on this PC.", log.name);
                return Ok(0);
            }
            println!("{}", residue_markdown(&log.name, &left));
            return Ok(2);
        }
        "orphans" => {
            let plan = installscope_core::orphans::scan_system(reg.as_ref());
            if has(&args, "--json") {
                let items: Vec<_> = plan
                    .items
                    .iter()
                    .map(|i| {
                        installscope_core::json!({
                            "category": i.action.category(),
                            "kind": i.note,
                            "action": i.action.describe(),
                            "risk": format!("{:?}", installscope_core::revert::risk(i)),
                        })
                    })
                    .collect();
                println!("{}", installscope_core::json::Value::Array(items));
            } else if plan.items.is_empty() {
                println!("No leftovers of removed programs found.");
            } else {
                for i in &plan.items {
                    println!("[{}] {}", i.note, i.action.describe());
                }
                eprintln!(
                    "{} items. Remove them from the app (Tools → Find leftovers of removed programs), where everything is backed up first.",
                    plan.items.len()
                );
            }
            return Ok(if plan.items.is_empty() { 0 } else { 2 });
        }
        "repackage" | "silent-package" => {
            let (Some(id), Some(dir)) = (args.get(1), args.get(2)) else {
                anyhow::bail!("usage: installscope repackage <log-id> <folder> [--no-setup]")
            };
            let st = store()?;
            let log = st.load_log(id)?;
            let s = installscope_core::repackage::repackage(
                &log,
                reg.as_ref(),
                &PathBuf::from(dir),
                &|_| {},
            )?;
            println!(
                "Saved {}: {} files ({}), {} keys, {} values",
                s.folder.display(),
                s.files,
                human_size(s.bytes),
                s.keys,
                s.values
            );
            if !has(&args, "--no-setup") {
                let tc = installscope_core::patools::ensure(&st.root, &|p| eprintln!("{p}…"))?;
                let setup = installscope_core::patools::build_silent_setup(
                    &tc,
                    &s.folder,
                    &log.name,
                    &log.version,
                    &st.root.join("tools").join("work"),
                )?;
                println!("Silent setup: {}", setup.display());
            }
        }
        "merge" => {
            let ids: Vec<&String> = args
                .iter()
                .skip(1)
                .take_while(|a| !a.starts_with("--"))
                .collect();
            if ids.len() < 2 {
                anyhow::bail!("usage: installscope merge <log-id> <log-id>... [--name NAME]");
            }
            let st = store()?;
            let logs = ids
                .iter()
                .map(|id| st.load_log(id))
                .collect::<anyhow::Result<Vec<_>>>()?;
            let refs: Vec<&InstallLog> = logs.iter().collect();
            let merged = installscope_core::merge::merge_logs(
                &refs,
                &arg_value(&args, "--name").unwrap_or_default(),
            );
            st.save_log(&merged)?;
            println!("{}", merged.id);
            eprintln!(
                "Merged {} logs into {} ({})",
                logs.len(),
                merged.id,
                merged.name
            );
        }
        "restore-point" | "restore-points" => {
            use installscope_core::restore;
            match args.get(1).map(String::as_str).unwrap_or("list") {
                "list" => {
                    let p = restore::protection();
                    println!(
                        "System Protection: {}",
                        if p.enabled { "on" } else { "off" }
                    );
                    for r in restore::list()? {
                        println!(
                            "{:>5}  {}  {:<18}  {}",
                            r.sequence,
                            r.created,
                            r.kind.label(),
                            r.description
                        );
                    }
                }
                "create" => {
                    let d = args
                        .get(2)
                        .cloned()
                        .unwrap_or_else(|| "InstallScope manual restore point".into());
                    restore::create_restore_point(&d)?;
                    println!("Created restore point: {d}");
                }
                "delete" => {
                    let n: u32 = args.get(2).and_then(|x| x.parse().ok()).ok_or_else(|| {
                        anyhow::anyhow!("usage: installscope restore-point delete <number>")
                    })?;
                    restore::delete(n)?;
                    println!("Deleted restore point {n}");
                }
                "rollback" => {
                    let n: u32 = args.get(2).and_then(|x| x.parse().ok()).ok_or_else(|| {
                        anyhow::anyhow!("usage: installscope restore-point rollback <number> --yes")
                    })?;
                    if !has(&args, "--yes") {
                        anyhow::bail!("Windows restarts immediately to restore point {n}; add --yes to confirm");
                    }
                    restore::restore_to(n)?;
                }
                other => anyhow::bail!(
                    "unknown restore-point action {other}: use list, create, delete or rollback"
                ),
            }
        }
        "pa-capture" => {
            let job = args
                .get(1)
                .ok_or_else(|| anyhow::anyhow!("usage: installscope pa-capture <job.json>"))?;
            installscope_core::pafbuild::run_job_file(std::path::Path::new(job))?;
        }
        "portable-build" => return portable_build(&args, reg.as_ref()),
        "learn-noise" => {
            let secs: u64 = arg_value(&args, "--seconds")
                .and_then(|s| s.parse().ok())
                .unwrap_or(60);
            let st = store()?;
            let before = snapshot_now(reg.as_ref(), &st)?;
            eprintln!("Waiting {secs} s; don't install or change anything…");
            std::thread::sleep(std::time::Duration::from_secs(secs));
            let after = snapshot_now(reg.as_ref(), &st)?;
            let mut settings = load_settings(&st);
            let roots: Vec<String> = settings.file_roots.iter().map(|r| r.path.clone()).collect();
            let l = installscope_core::noise::learn(
                &diff(&before, &after),
                &roots,
                &settings.registry_roots,
                &settings.effective_file_exclusions(),
                &settings.effective_registry_exclusions(),
            );
            if has(&args, "--json") {
                println!(
                    "{}",
                    installscope_core::json!({ "registry": l.registry, "files": l.files })
                );
            } else if l.is_empty() {
                println!("Nothing changed on its own.");
            } else {
                for p in l.files.iter().chain(&l.registry) {
                    println!("{p}");
                }
            }
            if has(&args, "--apply") && !l.is_empty() {
                let date = installscope_core::time::DateTime::now_local().ymd();
                installscope_core::noise::append_learned(
                    &mut settings.registry_exclusions,
                    &l.registry,
                    &date,
                );
                installscope_core::noise::append_learned(
                    &mut settings.file_exclusions,
                    &l.files,
                    &date,
                );
                st.save_settings(&settings)?;
                eprintln!("Added to the exclusions.");
            }
        }
        "verify" => {
            let file = args.get(1).ok_or_else(|| anyhow::anyhow!("missing file"))?;
            let tr = installscope_core::trust::check_file(file);
            match &tr {
                installscope_core::trust::Trust::Signed { signer } => {
                    println!("Signed by {signer} (valid)")
                }
                installscope_core::trust::Trust::Invalid { signer, reason } => {
                    println!("Signature problem: {reason} {signer}")
                }
                other => println!("{}", other.label()),
            }
            return Ok(
                if matches!(tr, installscope_core::trust::Trust::Signed { .. }) {
                    0
                } else {
                    2
                },
            );
        }
        "programs" if has(&args, "--json") => {
            let list = programs::list_programs(reg.as_ref(), has(&args, "--all"));
            if has(&args, "--trust") {
                let v: Vec<installscope_core::json::Value> = list
                    .iter()
                    .map(|p| {
                        use installscope_core::json::ToJson;
                        let mut j = p.to_json();
                        j.set(
                            "signature",
                            installscope_core::trust::check_program(p, reg.as_ref()).to_json(),
                        );
                        j
                    })
                    .collect();
                println!("{}", installscope_core::json::compat::to_string_pretty(&v)?);
            } else {
                println!(
                    "{}",
                    installscope_core::json::compat::to_string_pretty(&list)?
                );
            }
        }
        "add-program" => {
            let path = args.get(1).ok_or_else(|| {
                anyhow::anyhow!(
                    "usage: installscope add-program <folder|program.exe> [--name NAME]"
                )
            })?;
            let p = programs::add_manual(
                &store()?.root,
                path,
                &arg_value(&args, "--name").unwrap_or_default(),
            )?;
            println!(
                "Added {} ({}); uninstall it with: installscope uninstall \"{}\"",
                p.name, p.install_location, p.name
            );
        }
        "programs" => {
            let with_trust = has(&args, "--trust");
            let mut all = programs::list_programs(reg.as_ref(), has(&args, "--all"));
            if let Ok(st) = store() {
                all.extend(programs::load_manual(&st.root));
            }
            for p in all {
                if with_trust {
                    let tr = installscope_core::trust::check_program(&p, reg.as_ref());
                    let sig = match &tr {
                        installscope_core::trust::Trust::Signed { signer } => {
                            format!("signed: {signer}")
                        }
                        installscope_core::trust::Trust::Invalid { reason, .. } => {
                            format!("INVALID: {reason}")
                        }
                        other => other.label().to_lowercase(),
                    };
                    println!("{}\t{}\t{}\t{}", p.name, p.version, p.publisher, sig);
                } else {
                    println!("{}\t{}\t{}\t{}", p.name, p.version, p.publisher, p.key);
                }
            }
        }
        "revert" => {
            let id = args
                .get(1)
                .ok_or_else(|| anyhow::anyhow!("missing log id"))?;
            let st = store()?;
            let log = st.load_log(id)?;
            let mut plan = build_plan(&log.changes, reg.as_ref());
            // Cross-program reference counting: items other programs'
            // logs also claim, or no log mentions, are not auto-selected.
            let index = ReferenceIndex::build(&st.list_logs());
            let removing: std::collections::BTreeSet<String> =
                [log.id.clone()].into_iter().collect();
            let labels = annotate_plan(&mut plan, &index, &removing);
            if let Some(line) =
                index.coverage_line(programs::list_programs(reg.as_ref(), false).len())
            {
                println!("{line}");
            }
            for (item, label) in plan.items.iter().zip(&labels) {
                if !matches!(label, RefLabel::OnlyThisProgram) {
                    println!(
                        "[refs] not selected: {} ({})",
                        item.action.describe(),
                        label.text()
                    );
                }
            }
            let opts = ExecOptions {
                remove_unlogged: has(&args, "--remove-unlogged"),
                dry_run: has(&args, "--dry-run"),
                use_recycle_bin: has(&args, "--recycle"),
                restart_explorer: has(&args, "--restart-explorer"),
                close_programs: has(&args, "--close-programs"),
            };
            let mut writer = if has(&args, "--no-backup") || opts.dry_run {
                None
            } else {
                Some(BackupWriter::new(
                    &st.backups_dir(),
                    &format!("Uninstall {}", log.name),
                )?)
            };
            let results = execute_plan(&plan, reg.as_ref(), writer.as_mut(), &opts);
            for r in &results {
                println!(
                    "[{}] {} — {}",
                    if r.ok { " OK " } else { "FAIL" },
                    r.description,
                    r.message
                );
            }
            for i in &plan.info {
                println!("[note] {i}");
            }
            if let Some(w) = writer {
                let b = w.finish()?;
                println!(
                    "Backup: {} (restore with: installscope restore {})",
                    b.id, b.id
                );
            }
            let failed = results.iter().filter(|r| !r.ok).count();
            if !opts.dry_run && failed == 0 && !has(&args, "--keep-log") {
                st.delete_log(id)?;
            }
            return Ok(if failed > 0 { 2 } else { 0 });
        }
        "export" => {
            let (Some(id), Some(fmt), Some(out)) = (args.get(1), args.get(2), args.get(3)) else {
                anyhow::bail!(
                    "usage: installscope export <log-id> <html|json|reg|ps1|xml|portable> <output-file>"
                )
            };
            let log = store()?.load_log(id)?;
            let content = match fmt.as_str() {
                "html" => report::to_html(&log),
                "reg" => report::to_reg_undo(&log),
                "ps1" => report::to_ps1(&log),
                "json" => installscope_core::json::compat::to_string_pretty(&log)?,
                "xml" => report::to_xml(&log),
                "portable" | "ini" => {
                    installscope_core::portable::launcher_ini(
                        &log.changes,
                        &log.name,
                        &installscope_core::portable::Folders::current(),
                    )
                    .text
                }
                f => anyhow::bail!("unknown format {f}"),
            };
            if fmt == "reg" {
                let mut bytes = vec![0xFF, 0xFE];
                content
                    .encode_utf16()
                    .for_each(|u| bytes.extend_from_slice(&u.to_le_bytes()));
                std::fs::write(out, bytes)?;
            } else {
                std::fs::write(out, content)?;
            }
            println!("Saved {out}");
        }
        "backups" => {
            let list = list_backups(&store()?.backups_dir());
            let cut: Vec<String> = installscope_core::backup::interrupted(&list)
                .iter()
                .map(|b| b.id.clone())
                .collect();
            for b in &list {
                println!(
                    "{}  {}  {}{}{}  ({})",
                    b.id,
                    b.created,
                    b.title,
                    if b.restored { " [restored]" } else { "" },
                    if cut.contains(&b.id) {
                        " [INTERRUPTED]"
                    } else {
                        ""
                    },
                    b.summary()
                );
            }
            if !cut.is_empty() {
                eprintln!(
                    "{} operation(s) were interrupted before finishing. Undo with `installscope restore <backup-id>`.",
                    cut.len()
                );
            }
        }
        "restore" => {
            let id = args
                .get(1)
                .ok_or_else(|| anyhow::anyhow!("missing backup id"))?;
            let results = restore_backup(&store()?.backups_dir(), id, reg.as_ref())?;
            let failed = results.iter().filter(|r| !r.ok).count();
            for r in &results {
                println!(
                    "[{}] {} — {}",
                    if r.ok { " OK " } else { "FAIL" },
                    r.description,
                    r.message
                );
            }
            return Ok(if failed > 0 { 2 } else { 0 });
        }
        other => {
            eprintln!("unknown command: {other}\n\n{HELP}");
            return Ok(1);
        }
    }
    Ok(0)
}
