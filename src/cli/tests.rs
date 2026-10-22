use super::*;

fn v(args: &[&str]) -> Vec<String> {
    args.iter().map(|s| s.to_string()).collect()
}

#[test]
fn every_destructive_command_classifies_destructive() {
    use CmdClass::Destructive as D;
    for args in [
        &["revert", "log1"][..],
        &["uninstall", "Some App"][..],
        &["components", "log1", "--strike"][..],
        &["restore", "backup1"][..],
        &["transfer", "import", "pkg.setupscope"][..],
        &["rewind", "log1", "--to", "state1"][..],
    ] {
        let args = v(args);
        assert_eq!(classify(&args[0], &args), D, "{args:?}");
    }
}

#[test]
fn previews_and_read_only_commands_never_classify_destructive() {
    use CmdClass::ReadOnly as R;
    for args in [
        &["--help"][..],
        &["--version"][..],
        &["logs"][..],
        &["programs", "--json"][..],
        &["verify", "f.exe"][..],
        &["orphans"][..],
        &["check-clean", "log1"][..],
        &["backups"][..],
        &["revert", "log1", "--dry-run"][..],
        &["uninstall", "App", "--dry-run"][..],
        &["components", "log1"][..],
        &["components", "log1", "--json"][..],
        &["transfer", "check", "p.setupscope"][..],
        &["rewind", "log1", "--list"][..],
        &["rewind", "log1", "--to", "s", "--dry-run"][..],
    ] {
        let args = v(args);
        assert_eq!(classify(&args[0], &args), R, "{args:?}");
    }
}

#[test]
fn automation_and_bookkeeping_classify_mutating_not_destructive() {
    use CmdClass::Mutating as M;
    for args in [
        &["snapshot", "a.snap"][..],
        &["diff", "a.snap", "b.snap", "--auto"][..],
        &["monitor", "--", "setup.exe"][..],
        &["watch"][..],
        &["pa-capture", "job.json"][..],
        &["tag", "log1", "--group", "G"][..],
        &["merge", "a", "b"][..],
        &["history"][..],
        &["transfer", "export", "o.setupscope", "App"][..],
        &["rewind", "log1", "--capture"][..],
    ] {
        let args = v(args);
        assert_eq!(classify(&args[0], &args), M, "{args:?}");
    }
}

#[test]
fn gate_decision_covers_the_three_cases() {
    assert_eq!(gate_decision(true, true), Gate::Proceed);
    assert_eq!(gate_decision(true, false), Gate::Proceed);
    assert_eq!(gate_decision(false, true), Gate::Prompt);
    // The non-interactive refusal (P1-23): no terminal, no --yes.
    assert_eq!(gate_decision(false, false), Gate::Refuse);
}

#[test]
fn only_a_plain_yes_confirms() {
    for yes in ["yes", "YES", "Yes", "y", " yes "] {
        assert!(answer_yes(yes), "{yes:?}");
    }
    for no in ["", "no", "n", "ye", "yes please", "ok"] {
        assert!(!answer_yes(no), "{no:?}");
    }
}

#[test]
fn every_command_has_help_that_names_its_confirmation() {
    assert_eq!(COMMAND_HELP.len(), 25);
    for (name, text) in COMMAND_HELP {
        assert!(text.contains("Usage:"), "{name}: no usage line");
        assert!(
            text.contains("Confirmation:"),
            "{name}: no confirmation line"
        );
        assert!(
            text.starts_with(format!("setupscope {name}").as_str()),
            "{name}: heading"
        );
    }
    // Aliases resolve to the same help.
    assert_eq!(help_for("restore-points"), help_for("restore-point"));
    assert_eq!(help_for("silent-package"), help_for("repackage"));
    assert!(help_for("no-such-command").is_none());
}

#[test]
fn help_is_requested_per_command_and_never_after_a_separator() {
    // The P1-24 bug: `revert --help` was parsed as a log id.
    assert!(requested_help(&v(&["revert", "--help"])).is_some());
    assert!(requested_help(&v(&["revert", "log1", "-h"])).is_some());
    assert!(requested_help(&v(&["help", "programs"])).is_some());
    // Flags, ids and installer-side arguments are not help requests.
    assert!(requested_help(&v(&["revert", "log1"])).is_none());
    assert!(requested_help(&v(&["revert", "log1", "--yes"])).is_none());
    assert!(requested_help(&v(&["monitor", "--", "setup.exe", "--help"])).is_none());
    assert!(requested_help(&v(&["nonsense", "--help"])).is_none());
    assert!(requested_help(&v(&["help"])).is_none());
    assert!(requested_help(&v(&[])).is_none());
}

#[test]
fn destructive_help_states_the_yes_requirement() {
    for name in [
        "revert",
        "uninstall",
        "restore",
        "transfer",
        "components",
        "rewind",
    ] {
        let text = help_for(name).expect(name);
        assert!(text.contains("--yes"), "{name}: help must name --yes");
    }
}

#[test]
fn retired_commands_refuse_before_initialization_and_have_no_active_help() {
    for command in [
        "clean",
        "schedule-clean",
        "startup",
        "gatekeeper",
        "slowdown",
        "restore-point",
        "restore-points",
    ] {
        assert!(retired_command(command));
        assert!(help_for(command).is_none());
        assert!(!HELP.contains(&format!("  setupscope {command} ")));
        let error = run_inner(v(&[command, "--help"])).unwrap_err().to_string();
        assert!(
            error.contains("removed from the developer toolkit"),
            "{command}: {error}"
        );
    }
    for command in [
        "revert",
        "uninstall",
        "monitor",
        "restore",
        "check-clean",
        "history",
    ] {
        assert!(!retired_command(command));
    }
}
