//! Command-line interface for scripting and IT use.

use installscope_core::backup::{list_backups, restore_backup, BackupWriter};
use installscope_core::model::{human_size, InstallLog};
use installscope_core::platform::{self, Registry};
use installscope_core::refcount::{annotate_plan, RefLabel, ReferenceIndex};
use installscope_core::revert::{build_plan, execute_plan, ExecOptions};
use installscope_core::revert::{residue, residue_markdown};
use installscope_core::rewind::{
    build_rewind_plan, execute_rewind, list_states, CaptureKind, CaptureScope, ContentStore,
};
use installscope_core::snapshot::{
    detect_program, diff, filter_changes, load_snapshot, save_snapshot, take_snapshot, Progress,
};
use installscope_core::store::Store;
use installscope_core::{programs, report};
use std::io::IsTerminal;
use std::path::PathBuf;

mod help;
use help::{COMMAND_HELP, HELP};

fn arg_value(args: &[String], flag: &str) -> Option<String> {
    args.iter()
        .position(|a| a == flag)
        .and_then(|i| args.get(i + 1).cloned())
}

/// What an invocation does to the PC. Drives two things: the
/// not-elevated warning (read-only commands never warn — warning about a
/// listing trains users to ignore it) and the confirmation discipline
/// for destructive commands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CmdClass {
    /// Prints information only; never writes, never prompts.
    ReadOnly,
    /// Writes InstallScope data, files the user named, settings or
    /// backed-up, one-step-reversible changes. Never prompts.
    Mutating,
    /// Removes, deletes, uninstalls or overwrites system state, or is
    /// hard to undo. Must be confirmed (see `confirm_destructive`).
    Destructive,
}

fn classify(cmd: &str, args: &[String]) -> CmdClass {
    use CmdClass::*;
    let sub = args.get(1).map(String::as_str).unwrap_or("");
    match cmd {
        "--help" | "-h" | "help" | "--version" | "-V" => ReadOnly,
        "logs" | "programs" | "verify" | "orphans" | "check-clean" | "backups" => ReadOnly,
        // Writes snapshots, logs, packages, settings or bookkeeping.
        "snapshot" | "diff" | "monitor" | "tag" | "merge" | "export" | "repackage"
        | "silent-package" | "learn-noise" | "add-program" | "history" | "slowdown" | "watch"
        | "pa-capture" | "portable-build" | "schedule-clean" => Mutating,
        "revert" | "uninstall" => {
            if has(args, "--dry-run") {
                ReadOnly
            } else {
                Destructive
            }
        }
        "clean" => {
            if has(args, "--scheduled") {
                // The Task Scheduler entry point: the schedule the user
                // set is the standing instruction, so it never prompts.
                Mutating
            } else if has(args, "--list") || has(args, "--dry-run") {
                ReadOnly
            } else {
                Destructive
            }
        }
        "components" => {
            if has(args, "--strike") {
                Destructive
            } else {
                ReadOnly
            }
        }
        "restore" => Destructive,
        "transfer" => match sub {
            "check" => ReadOnly,
            "import" => Destructive,
            _ => Mutating,
        },
        "rewind" => {
            if has(args, "--list") {
                ReadOnly
            } else if has(args, "--capture") {
                Mutating
            } else if has(args, "--to") {
                if has(args, "--dry-run") {
                    ReadOnly
                } else {
                    Destructive
                }
            } else {
                Mutating
            }
        }
        "restore-point" | "restore-points" => match sub {
            "delete" | "rollback" => Destructive,
            "create" => Mutating,
            _ => ReadOnly,
        },
        "startup" => match sub {
            // Backed up, and the inverse subcommand undoes it.
            "enable" | "disable" => Mutating,
            _ => ReadOnly,
        },
        "gatekeeper" => match sub {
            "remove" => Destructive,
            "scan" | "allow" | "enable" | "disable" => Mutating,
            _ => ReadOnly,
        },
        _ => Mutating,
    }
}

/// The confirmation decision for a destructive invocation, as a pure
/// function so the discipline is testable without a terminal.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Gate {
    /// `--yes` was given: proceed silently.
    Proceed,
    /// Interactive terminal: ask, with a description of what will happen.
    Prompt,
    /// No terminal and no `--yes`: refuse, changing nothing.
    Refuse,
}

fn gate_decision(yes: bool, interactive: bool) -> Gate {
    if yes {
        Gate::Proceed
    } else if interactive {
        Gate::Prompt
    } else {
        Gate::Refuse
    }
}

fn stdin_interactive() -> bool {
    std::io::stdin().is_terminal()
}

/// A typed answer counts as yes only when it plainly says so.
fn answer_yes(line: &str) -> bool {
    let a = line.trim().to_lowercase();
    a == "yes" || a == "y"
}

/// The confirmation discipline for destructive commands, matching the
/// `--yes` convention `rewind` and `restore-point rollback` started:
/// `--yes` proceeds; an interactive terminal is told exactly what will
/// happen (`what`) and must type "yes"; anything else refuses. Returns
/// whether to proceed. Callers return exit code 1 when this is false.
fn confirm_destructive(args: &[String], what: &str) -> bool {
    match gate_decision(has(args, "--yes"), stdin_interactive()) {
        Gate::Proceed => true,
        Gate::Prompt => {
            eprintln!("{what}");
            eprint!("Type \"yes\" to continue: ");
            {
                use std::io::Write;
                let _ = std::io::stderr().flush();
            }
            let mut line = String::new();
            match std::io::stdin().read_line(&mut line) {
                Ok(_) if answer_yes(&line) => true,
                _ => {
                    eprintln!("Cancelled; nothing was changed.");
                    false
                }
            }
        }
        Gate::Refuse => {
            eprintln!(
                "Refusing to run without confirmation: there is no interactive \
                 terminal to ask in. Review what this command does \
                 (`installscope {} --help`), then re-run with --yes if you mean it.",
                args.first().map(String::as_str).unwrap_or("")
            );
            false
        }
    }
}

fn help_for(cmd: &str) -> Option<&'static str> {
    let key = match cmd {
        "restore-points" => "restore-point",
        "silent-package" => "repackage",
        other => other,
    };
    COMMAND_HELP
        .iter()
        .find(|(name, _)| *name == key)
        .map(|(_, text)| *text)
}

/// Help requested by this invocation, if any: `installscope <command>
/// --help` / `-h` (only before a `--` separator, whose right side
/// belongs to another program), or `installscope help <command>`.
/// Checked before anything runs, so help never warns, prompts or
/// touches the system — and a flag can no longer be mistaken for an
/// argument (P1-24: `revert --help` used to be parsed as a log id).
fn requested_help(args: &[String]) -> Option<&'static str> {
    let cmd = args.first().map(String::as_str)?;
    if cmd == "help" {
        return args.get(1).and_then(|topic| help_for(topic));
    }
    help_for(cmd)?;
    let wants = args[1..]
        .iter()
        .take_while(|a| a.as_str() != "--")
        .any(|a| a == "--help" || a == "-h");
    if wants {
        help_for(cmd)
    } else {
        None
    }
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

/// Fold the current install logs, program list and startup list into the
/// Slowdown Detective history and save it. Cheap reads only — no snapshot.
fn capture_history(st: &Store, reg: &dyn Registry) -> installscope_core::history::HistoryStore {
    let mut h = installscope_core::history::HistoryStore::load(&st.root);
    let logs = st.list_logs();
    let programs_now = programs::list_programs(reg, false);
    let autostart = installscope_core::startup::list_startup(reg);
    h.capture(
        &logs,
        &programs_now,
        &autostart,
        installscope_core::time::unix_now(),
    );
    if let Err(e) = h.save(&st.root) {
        eprintln!("warning: could not save the change history: {e}");
    }
    h
}

/// One history event as a single display line.
fn event_line(e: &installscope_core::history::HistoryEvent) -> String {
    use installscope_core::history::HistoryEventKind as K;
    let mut s = format!("{}  {:<17} {}", e.date, e.kind.label(), e.subject);
    match e.kind {
        K::ProgramUpdated => s.push_str(&format!("  {} → {}", e.from_version, e.to_version)),
        K::ProgramInstalled if !e.to_version.is_empty() => {
            s.push_str(&format!("  {}", e.to_version))
        }
        K::ProgramRemoved if !e.from_version.is_empty() => {
            s.push_str(&format!("  (was {})", e.from_version))
        }
        _ => {}
    }
    if !e.target.is_empty() {
        s.push_str(&format!("  {}", e.target));
    }
    if !e.detail.is_empty() {
        s.push_str(&format!("  — {}", e.detail));
    }
    s
}

fn event_json(
    e: &installscope_core::history::HistoryEvent,
    measured_ms: Option<u64>,
) -> installscope_core::json::Value {
    installscope_core::json!({
        "date": e.date, "kind": format!("{:?}", e.kind), "subject": e.subject,
        "from_version": e.from_version, "to_version": e.to_version,
        "target": e.target, "detail": e.detail, "measured_ms": measured_ms,
    })
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
    // Per-command help comes before everything else: it must not warn,
    // prompt or touch the system, and it exits 0.
    if let Some(help) = requested_help(&args) {
        println!("{help}");
        return Ok(0);
    }
    let cmd = args.first().map(String::as_str).unwrap_or("--help");
    if let Some(p) = arg_value(&args, "--profile") {
        let _ = PROFILE.set(p);
    }
    let class = classify(cmd, &args);
    // The not-elevated warning only helps where something will actually
    // be written; on read-only commands it just trains users to ignore
    // it (audit P2-2).
    if class != CmdClass::ReadOnly && !platform::is_elevated() {
        eprintln!("warning: not running as Administrator; system-wide changes may fail.");
    }
    // A destructive invocation with no terminal to ask in and no --yes
    // refuses up front, before anything is loaded or changed.
    if class == CmdClass::Destructive
        && gate_decision(has(&args, "--yes"), stdin_interactive()) == Gate::Refuse
    {
        eprintln!(
            "`installscope {cmd}` changes this PC and cannot ask for confirmation \
             without an interactive terminal.\nNothing was changed. Run it in a \
             terminal to be asked, or add --yes in a script once you have reviewed \
             what it will do (see `installscope {cmd} --help`)."
        );
        return Ok(1);
    }
    let reg = platform::system_registry();
    match cmd {
        "--help" | "-h" | "help" => println!("{HELP}"),
        "--version" | "-V" => println!("InstallScope {}", installscope_core::APP_VERSION),
        "snapshot" => {
            let out = args
                .get(1)
                .ok_or_else(|| anyhow::anyhow!("usage: installscope snapshot <file.snap>"))?;
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
                    let out = args.get(2).ok_or_else(|| {
                        anyhow::anyhow!(
                            "usage: installscope transfer export <out.installscope> <program>..."
                        )
                    })?;
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
                    let pkg = PathBuf::from(args.get(2).ok_or_else(|| {
                        anyhow::anyhow!("usage: installscope transfer check|import <package>")
                    })?);
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
                    let what = format!(
                        "This imports {} onto this PC: {} program(s) are written with \
                         their files and settings{}. A backup is taken first, so the \
                         import can be undone with `installscope restore <backup-id>`.",
                        pkg.display(),
                        names.len(),
                        if has(&args, "--overwrite") {
                            ", replacing files that already exist"
                        } else {
                            ""
                        }
                    );
                    if !confirm_destructive(&args, &what) {
                        return Ok(1);
                    }
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
            if !dry {
                let what = format!(
                    "This uninstalls: {}. Each program's own uninstaller runs — what \
                     an uninstaller itself changes is not backed up and cannot be \
                     undone. Safe leftovers found afterwards are removed with a \
                     backup (undo with `installscope restore <backup-id>`).",
                    names.join(", ")
                );
                if !confirm_destructive(&args, &what) {
                    return Ok(1);
                }
            }
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
            // The Cleaner page's protected list applies here too, so a
            // protected path survives manual and scheduled cleans alike.
            let protect = load_settings(&store()?).cleaner_protect;
            if !has(&args, "--dry-run") && !has(&args, "--scheduled") {
                let what = format!(
                    "This permanently deletes junk files in these categories: {}. \
                     Cleaned files are not backed up and InstallScope cannot \
                     restore them. Paths on the Cleaner page's Protected list \
                     are kept.",
                    wanted.join(", ")
                );
                if !confirm_destructive(&args, &what) {
                    return Ok(1);
                }
            }
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
                let scan = installscope_core::cleaner::scan_detailed(c, &protect);
                let kept = scan.kept_totals();
                if kept.files > 0 && !has(&args, "--scheduled") {
                    println!(
                        "{:<20} {:>10}  {} protected file(s) kept",
                        id,
                        human_size(kept.bytes),
                        kept.files
                    );
                }
                let r = if has(&args, "--dry-run") {
                    scan.totals
                } else {
                    installscope_core::cleaner::clean_files(c, &scan.files)
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
        "gatekeeper" => {
            // Autostart Gatekeeper: opt-in review queue for new autostart
            // entries. Nothing here can block an installer or remove an
            // entry the user has not explicitly decided on.
            use installscope_core::gatekeeper::{self as gk, Attribution, ItemState, Queue};
            let st = store()?;
            let mut settings = load_settings(&st);
            let sub = args.get(1).map(|s| s.as_str());
            match sub {
                None | Some("show") | Some("--json") => {
                    let q = Queue::load(&st)?;
                    if has(&args, "--json") {
                        println!(
                            "{}",
                            installscope_core::json::compat::to_string_pretty(&q.items)?
                        );
                        return Ok(0);
                    }
                    println!(
                        "Autostart Gatekeeper is {}.",
                        if settings.gatekeeper_enabled {
                            "on"
                        } else {
                            "off"
                        }
                    );
                    if q.items.is_empty() {
                        println!("The review queue is empty.");
                    } else {
                        for i in &q.items {
                            let verdict = i
                                .verdict
                                .map(|v| format!(" — {}", v.label()))
                                .unwrap_or_default();
                            println!(
                                "[{}] {} ({}) — {}{}",
                                match i.state {
                                    ItemState::Pending => " ? ",
                                    ItemState::Allowed => " ok",
                                    ItemState::Removed => "gone",
                                },
                                i.entry.name,
                                i.entry.command,
                                i.attribution.label(),
                                verdict
                            );
                            println!(
                                "      id {} · first seen {} · {}",
                                i.id,
                                i.first_seen,
                                i.state.label()
                            );
                            for r in &i.verdict_reasons {
                                println!("      {r}");
                            }
                            if let Attribution::Log { evidence, .. } = &i.attribution {
                                println!("      evidence: {evidence}");
                            }
                            if !i.note.is_empty() {
                                println!("      note: {}", i.note);
                            }
                        }
                    }
                    let n = q.pending().len();
                    if n > 0 {
                        println!(
                            "{n} entr{} waiting for your decision: \
                             `installscope gatekeeper allow <id>` keeps one, \
                             `installscope gatekeeper remove <id>` removes it (backed up first).",
                            if n == 1 { "y" } else { "ies" }
                        );
                    }
                }
                Some("scan") => {
                    if !settings.gatekeeper_enabled {
                        println!(
                            "Autostart Gatekeeper is off, so nothing was scanned. \
                             Turn it on with `installscope gatekeeper enable` or in Settings."
                        );
                        return Ok(0);
                    }
                    let mut q = Queue::load(&st)?;
                    let current: Vec<gk::WatchedEntry> =
                        installscope_core::startup::list_startup(reg.as_ref())
                            .iter()
                            .map(gk::WatchedEntry::from)
                            .collect();
                    let logs = st.list_logs();
                    let summary = gk::run_scan(&mut q, &current, &logs);
                    q.save(&st)?;
                    println!(
                        "Scan complete: {} new, {} changed, {} waiting for your decision.",
                        summary.added.len(),
                        summary.changed.len(),
                        q.pending().len()
                    );
                }
                Some("allow") => {
                    let id = args.get(2).ok_or_else(|| {
                        anyhow::anyhow!("usage: installscope gatekeeper allow <id>")
                    })?;
                    let mut q = Queue::load(&st)?;
                    let item = q.allow(id)?;
                    q.save(&st)?;
                    println!(
                        "Kept \"{}\": it stays as it is, and future scans will not ask about it again.",
                        item.entry.name
                    );
                }
                Some("remove") => {
                    let id = args.get(2).ok_or_else(|| {
                        anyhow::anyhow!("usage: installscope gatekeeper remove <id>")
                    })?;
                    let mut q = Queue::load(&st)?;
                    let label = q
                        .items
                        .iter()
                        .find(|i| i.id.as_str() == id.as_str())
                        .map(|i| i.entry.name.clone())
                        .unwrap_or_else(|| id.clone());
                    let what = format!(
                        "This removes the autostart entry \u{201c}{label}\u{201d}. \
                         A backup is taken first, so it can be restored with \
                         `installscope restore <backup-id>` (the id is printed \
                         when it finishes)."
                    );
                    if !confirm_destructive(&args, &what) {
                        return Ok(1);
                    }
                    let (item, backup_id) =
                        gk::remove_entry(&mut q, id, reg.as_ref(), &st.backups_dir())?;
                    q.save(&st)?;
                    println!(
                        "Removed \"{}\". Backup: {} (undo with: installscope restore {})",
                        item.entry.name, backup_id, backup_id
                    );
                }
                Some("enable") => {
                    settings.gatekeeper_enabled = true;
                    st.save_settings(&settings)?;
                    println!(
                        "Autostart Gatekeeper is on. New autostart entries will be \
                         held in the review queue (`installscope gatekeeper`) instead \
                         of going unnoticed. Nothing is blocked or removed without \
                         your decision."
                    );
                }
                Some("disable") => {
                    settings.gatekeeper_enabled = false;
                    st.save_settings(&settings)?;
                    println!(
                        "Autostart Gatekeeper is off. The queue and baseline were kept; \
                         turn it back on any time to resume reviewing."
                    );
                }
                Some(other) => {
                    anyhow::bail!(
                        "unknown gatekeeper command \"{other}\"; use: \
                         gatekeeper [show|--json] | scan | allow <id> | remove <id> | enable | disable"
                    );
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
                    let what = format!(
                        "This strikes {} component(s) of {}: the services, tasks and \
                         autoruns judged as bloat are removed. A backup is taken \
                         first, so they can be restored with \
                         `installscope restore <backup-id>`.",
                        chosen.len(),
                        log.name
                    );
                    if !confirm_destructive(&args, &what) {
                        return Ok(1);
                    }
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
        "history" => {
            let st = store()?;
            let h = capture_history(&st, reg.as_ref());
            let since = arg_value(&args, "--since");
            if let Some(s) = &since {
                if installscope_core::time::Date::parse_ymd(s).is_none() {
                    anyhow::bail!("--since wants a date as YYYY-MM-DD, got {s}");
                }
            }
            let mut events: Vec<_> = h
                .events
                .iter()
                .filter(|e| since.as_deref().is_none_or(|s| e.date.as_str() >= s))
                .collect();
            events.reverse(); // newest first
            if has(&args, "--json") {
                let items: Vec<_> = events.iter().map(|e| event_json(e, None)).collect();
                println!(
                    "{}",
                    installscope_core::json::compat::to_string_pretty(&items)?
                );
            } else if events.is_empty() {
                match h.started_date() {
                    Some(d) => println!("No changes recorded yet. Recording began {d}; installs, updates, removals and autostart changes will appear here as they happen."),
                    None => println!("No changes recorded yet."),
                }
            } else {
                for e in &events {
                    println!("{}", event_line(e));
                }
            }
        }
        "slowdown" => {
            use installscope_core::history::HistoryEventKind as K;
            use installscope_core::time::Date;
            let st = store()?;
            let h = capture_history(&st, reg.as_ref());
            let today = Date::today_local();
            let to = match arg_value(&args, "--to") {
                Some(s) => Date::parse_ymd(&s)
                    .ok_or_else(|| anyhow::anyhow!("--to wants a date as YYYY-MM-DD, got {s}"))?,
                None => today,
            };
            let from = match arg_value(&args, "--from") {
                Some(s) => Date::parse_ymd(&s)
                    .ok_or_else(|| anyhow::anyhow!("--from wants a date as YYYY-MM-DD, got {s}"))?,
                None => today.add_days(-90),
            };
            if from > to {
                anyhow::bail!("--from {from} is after --to {to}");
            }
            let rep = installscope_core::history::report(&h, from, to);

            // Measurements, where Windows has them: boot diagnostics for
            // autostart entries, the usage tracker's startup figures for
            // programs still installed. Anything unmeasured stays unranked —
            // it is never reported as zero impact.
            let boot = installscope_core::boot::read();
            let usage = installscope_core::usage::UsageData::gather(reg.as_ref(), &st.root);
            let programs_now = programs::list_programs(reg.as_ref(), false);
            let measured = |e: &installscope_core::history::HistoryEvent| -> Option<u64> {
                if let Some(ms) = installscope_core::history::boot_impact_ms(e, boot.as_ref()) {
                    return Some(ms);
                }
                if e.kind.is_program() {
                    let p = programs_now.iter().find(|p| {
                        installscope_core::model::fold(&p.name)
                            == installscope_core::model::fold(&e.subject)
                    })?;
                    return usage.for_program(p).startup_ms;
                }
                None
            };

            if has(&args, "--json") {
                let doc = installscope_core::json!({
                    "from": from.to_string(), "to": to.to_string(),
                    "history_started": rep.history_started,
                    "total_events": rep.total_events,
                    "measurements_available": boot.is_some(),
                    "events": rep.events.iter().map(|e| event_json(e, measured(e))).collect::<Vec<_>>(),
                });
                println!(
                    "{}",
                    installscope_core::json::compat::to_string_pretty(&doc)?
                );
            } else {
                println!("Slowdown Detective — changes from {from} to {to}");
                match &rep.history_started {
                    Some(d) => println!(
                        "Recording since {d} · {} events recorded in all.",
                        rep.total_events
                    ),
                    None => println!("Recording has not begun."),
                }
                if let Some(ms) = boot.as_ref().and_then(|b| b.last_boot_ms) {
                    println!(
                        "Last startup took {:.0} s (Windows boot diagnostics).",
                        ms as f64 / 1000.0
                    );
                }
                if rep.is_empty() {
                    if rep.total_events == 0 {
                        println!(
                            "\nNot enough history yet: InstallScope has only just begun \
                             recording, so there is no earlier state to compare against. \
                             Installs, updates, removals and autostart changes from now on \
                             will build the history this report needs."
                        );
                    } else {
                        println!(
                            "\nNothing changed between {from} and {to}. \
                             {} events are recorded outside that window; widen it with --from/--to.",
                            rep.total_events
                        );
                    }
                } else {
                    let mut ranked: Vec<(&installscope_core::history::HistoryEvent, u64)> =
                        Vec::new();
                    let mut rest: Vec<&installscope_core::history::HistoryEvent> = Vec::new();
                    for e in &rep.events {
                        match measured(e) {
                            Some(ms) => ranked.push((e, ms)),
                            None => rest.push(e),
                        }
                    }
                    ranked.sort_by(|a, b| b.1.cmp(&a.1).then(b.0.date.cmp(&a.0.date)));
                    if !ranked.is_empty() {
                        println!(
                            "\nRanked by measured startup impact (averages Windows measured):"
                        );
                        for (e, ms) in &ranked {
                            println!("  [{:>6} ms] {}", ms, event_line(e));
                        }
                    } else if boot.is_none() {
                        println!(
                            "\nStartup-impact measurements are unavailable (Windows boot \
                             diagnostics need administrator rights), so changes are ranked \
                             by recency and count instead."
                        );
                    }
                    if !rest.is_empty() {
                        if !ranked.is_empty() {
                            println!(
                                "\nEverything else — no impact measurement exists for these, newest first:"
                            );
                        }
                        for kind in [
                            K::ProgramInstalled,
                            K::ProgramUpdated,
                            K::ProgramRemoved,
                            K::AutostartAdded,
                            K::AutostartRemoved,
                            K::AutostartChanged,
                        ] {
                            let mut group: Vec<_> =
                                rest.iter().filter(|e| e.kind == kind).collect();
                            if group.is_empty() {
                                continue;
                            }
                            group.reverse(); // newest first within the group
                            println!("\n{} ({}):", kind.label(), group.len());
                            for e in group {
                                println!("  {}", event_line(e));
                            }
                        }
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
                        anyhow::anyhow!("usage: installscope restore-point delete <number> [--yes]")
                    })?;
                    let what = format!(
                        "This deletes System Restore point {n} for good; Windows \
                         can no longer roll back to it."
                    );
                    if !confirm_destructive(&args, &what) {
                        return Ok(1);
                    }
                    restore::delete(n)?;
                    println!("Deleted restore point {n}");
                }
                "rollback" => {
                    let n: u32 = args.get(2).and_then(|x| x.parse().ok()).ok_or_else(|| {
                        anyhow::anyhow!("usage: installscope restore-point rollback <number> --yes")
                    })?;
                    let what = format!(
                        "Windows restarts right away and rolls system files and \
                         settings back to restore point {n}. Programs installed \
                         after that point may stop working; System Restore does \
                         not touch your documents."
                    );
                    if !confirm_destructive(&args, &what) {
                        return Ok(1);
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
            let file = args
                .get(1)
                .ok_or_else(|| anyhow::anyhow!("usage: installscope verify <file>"))?;
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
            let id = args.get(1).ok_or_else(|| {
                anyhow::anyhow!(
                    "usage: installscope revert <log-id> [--dry-run] [--yes] \
                     (see `installscope revert --help`)"
                )
            })?;
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
            if !opts.dry_run {
                let what = format!(
                    "This removes everything the installation of \u{201c}{}\u{201d} \
                     added: {} item(s) from install log {id}.{}",
                    log.name,
                    plan.selected(),
                    if has(&args, "--no-backup") {
                        " No backup will be taken (--no-backup), so this cannot \
                         be undone."
                    } else {
                        " A backup is taken first, so it can be undone with \
                         `installscope restore <backup-id>` (the id is printed \
                         when it finishes)."
                    }
                );
                if !confirm_destructive(&args, &what) {
                    return Ok(1);
                }
            }
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
        "rewind" => {
            let id = args.get(1).ok_or_else(|| {
                anyhow::anyhow!(
                    "usage: installscope rewind <log-id> --list | --capture | --to <state>"
                )
            })?;
            let st = store()?;
            let budget = load_settings(&st).rewind_budget_bytes;
            let log = st.load_log(id)?;
            let mut cstore = ContentStore::open(&st.root)?;
            if has(&args, "--list") {
                println!("Recorded states of \u{201c}{}\u{201d}:", log.name);
                for s in list_states(&cstore, &log) {
                    println!(
                        "  {:<20} {}{}",
                        s.id,
                        s.label,
                        if s.files_without_content > 0 {
                            format!(
                                "  [{} file(s) have recorded contents; {} without]",
                                s.files_with_content, s.files_without_content
                            )
                        } else {
                            format!(
                                "  [{} file(s) have recorded contents]",
                                s.files_with_content
                            )
                        }
                    );
                }
                println!(
                    "Content store: {} used of {} budget",
                    human_size(cstore.referenced_bytes()),
                    human_size(budget)
                );
                for e in cstore
                    .index()
                    .evictions
                    .iter()
                    .filter(|e| e.log_id == log.id)
                {
                    println!(
                        "  [evicted] {} ({} file(s), {}) to stay within the budget",
                        e.set_id,
                        e.files,
                        human_size(e.bytes)
                    );
                }
                if cstore.referenced_bytes() > budget {
                    eprintln!(
                        "warning: the content store is over budget; only before-states and \
                         newest states remain, which are protected from eviction."
                    );
                }
                return Ok(0);
            }
            if has(&args, "--capture") {
                // Explicit capture entry point until the monitoring pipeline
                // captures before-contents automatically (see src/rewind.rs).
                let kind = if has(&args, "--before") {
                    CaptureKind::Before
                } else {
                    CaptureKind::Snapshot
                };
                if kind == CaptureKind::Before {
                    eprintln!(
                        "warning: this records the current file contents as the log's \
                         before-state; it is only correct if the files have not \
                         changed since the installation was recorded."
                    );
                }
                let report = if kind == CaptureKind::Before {
                    cstore.capture_before_images(&log, budget)?
                } else {
                    cstore.capture(&log, kind, CaptureScope::WholeLog, budget)?
                };
                let set = report.set.expect("capture always returns a set");
                println!(
                    "Recorded state {} for \u{201c}{}\u{201d}: {} file(s), {}",
                    set.id,
                    log.name,
                    set.files.len(),
                    human_size(set.bytes())
                );
                for s in &report.skipped {
                    println!("  [skipped] {s}");
                }
                for e in &report.evicted {
                    println!(
                        "  [evicted] {} ({} file(s)) to stay within the budget",
                        e.set_id, e.files
                    );
                }
                return Ok(0);
            }
            let Some(state) = arg_value(&args, "--to") else {
                anyhow::bail!(
                    "usage: installscope rewind <log-id> --list | --capture [--before] | --to <state> [--dry-run] [--yes]"
                );
            };
            let dry = has(&args, "--dry-run");
            let plan = build_rewind_plan(&cstore, &log, &state, reg.as_ref())?;
            for line in plan.describe_lines() {
                println!("{line}");
            }
            if dry {
                return Ok(if plan.complete() { 0 } else { 2 });
            }
            if !confirm_destructive(
                &args,
                &format!(
                    "This rewinds \u{201c}{}\u{201d} to the recorded state {state}: \
                     files and registry settings are put back as recorded (the \
                     plan above shows anything that cannot be restored). A backup \
                     of the current state is taken first, so the rewind itself \
                     can be undone.",
                    log.name
                ),
            ) {
                return Ok(1);
            }
            // Always backed up: a rewind is itself undoable, never destructive.
            let outcome = execute_rewind(
                &mut cstore,
                &plan,
                &log,
                reg.as_ref(),
                &st.backups_dir(),
                budget,
                &ExecOptions::default(),
            )?;
            for r in &outcome.results {
                println!(
                    "[{}] {} \u{2014} {}",
                    if r.ok { " OK " } else { "FAIL" },
                    r.description,
                    r.message
                );
            }
            for e in &outcome.evicted {
                println!(
                    "  [evicted] {} ({} file(s)) to stay within the budget",
                    e.set_id, e.files
                );
            }
            if let Some(set) = &outcome.pre_rewind_set {
                println!(
                    "Pre-rewind state recorded as {} (rewind to it to go back).",
                    set.id
                );
            }
            if let Some(b) = &outcome.backup_id {
                println!(
                    "Backup: {} (undo the rewind with: installscope restore {})",
                    b, b
                );
            }
            if !plan.complete() {
                println!(
                    "This rewind was partial: {} item(s) had no recorded before-state.",
                    plan.gaps.len()
                );
            }
            let failed = outcome.failed();
            return Ok(if failed > 0 || !plan.complete() { 2 } else { 0 });
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
            let id = args.get(1).ok_or_else(|| {
                anyhow::anyhow!("usage: installscope restore <backup-id> [--yes]")
            })?;
            let what = format!(
                "This restores backup {id}: the files and registry settings it \
                 holds are put back, replacing what is on this PC now. The \
                 restore itself takes no new backup."
            );
            if !confirm_destructive(&args, &what) {
                return Ok(1);
            }
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

#[cfg(test)]
mod tests;
