//! Help text for the command-line interface: the top-level summary and
//! the per-command entries. Kept apart from `cli.rs` so the file that
//! implements the commands stays a manageable size.

pub(super) const HELP: &str = "\
SetupScope — Windows Install, Uninstall & Developer Tools
Copyright (C) 2026 Sean Shaffer. MIT License: free software, no warranty.

USAGE:
  setupscope                                   Start the graphical interface
  setupscope monitor [--name NAME] -- <installer> [args...]
                                            Snapshot, run the installer, wait,
                                            snapshot again, save an install log
  setupscope snapshot <file.snap>              Save a snapshot of the system
  setupscope diff <before.snap> <after.snap> [--name NAME]
                                            Compare two snapshots, save a log
  setupscope logs [--group G] [--tag T] [--json]
                                            List saved install logs
  setupscope tag <log-id> [--group G] [--tags a,b]
                                            Set a log's group and tags
  setupscope programs [--all] [--trust] [--json]
                                            List installed programs (--trust adds
                                            each program's digital signature)
  setupscope verify <file>                     Check a file's digital signature
                                            (exit 2 if unsigned or invalid)
  setupscope revert <log-id> [--dry-run] [--remove-unlogged] [--no-backup] [--recycle]
                          [--close-programs] [--restart-explorer] [--keep-log] [--yes]
                                            Undo an installation precisely
  setupscope rewind <log-id> --list         App Rewind (early prototype): the
                                            recorded states of one program
  setupscope rewind <log-id> --capture [--before]
                                            Record the program's current file
                                            contents as a state (--before records
                                            them as the log's before-state)
  setupscope rewind <log-id> --to <state> [--dry-run]
                                            Rewind one program to a recorded
                                            state; --dry-run prints the plan,
                                            including anything that cannot be
                                            restored. Without --dry-run the
                                            rewind runs after --yes, takes a
                                            backup of the current state first,
                                            and exits 2 when the state is only
                                            partly restorable
  setupscope export <log-id> <html|json|reg|ps1|xml> <output-file>
  setupscope repackage <log-id> <folder> [--no-setup]
                                            A silent installer for the recorded
                                            installation: one Setup.exe plus a
                                            folder with install.ps1, install.reg
                                            and uninstall.ps1
  setupscope merge <log-id> <log-id>... [--name NAME]
                                            Combine logs into a new one (e.g. a
                                            program and its update)
  setupscope learn-noise [--seconds N] [--apply] [--json]
                                            Take two snapshots N seconds apart
                                            (default 60) and list what changed
                                            on its own; --apply adds it to the
                                            exclusions
  setupscope transfer export <out.setupsweep> <program>...
                                            Pack programs (by name) with their files
                                            and settings to move to another PC
  setupscope transfer check <package>          Check a package against this PC
  setupscope transfer import <package> [--overwrite] [--yes]
                                            Restore a package (undo from backups)
  setupscope export <log-id> portable <launcher.ini>
                                            PortableApps.com Launcher settings
                                            for what the log recorded
  setupscope portable-build <installer> --name NAME [--version V] [--publisher P]
                 [--category C] [--silent] [--out DIR] [--here] [--defaults]
                 [--no-prompt] [--no-compile]
                                            Capture an installation in Windows
                                            Sandbox (or on this PC with --here)
                                            and build a PortableApps.com package
                                            with its launcher and .paf.exe
                                            installer
  setupscope add-program <folder|program.exe> [--name NAME]
                                            List a program Windows doesn't know
                                            (e.g. portable) so it can be analyzed
                                            and uninstalled
  setupscope components <log-id> [--json] [--ask-model] [--strike]
                                            Installer Decrapifier: the services,
                                            tasks and autoruns an installation
                                            added, judged; --strike removes the
                                            bloat (backed up)
  setupscope orphans [--json]                  List registrations left by removed
                                            programs (pointing to missing files)
  setupscope uninstall <name>... [--silent] [--leftovers safe|none] [--dry-run] [--yes]
                                            Uninstall programs one after another,
                                            then remove safe leftovers (backed up)
  setupscope clean [--categories a,b] [--list] [--dry-run] [--yes]
                                            Clean junk (default: the safe kinds)
  setupscope schedule-clean daily|weekly|monthly|off [--categories a,b]
                                            Clean junk automatically
  setupscope startup [--json]                  List startup programs
  setupscope startup enable|disable <name>     Turn a startup program on or off
  setupscope history [--since YYYY-MM-DD] [--json]
                                            Record new changes, then list the
                                            recorded ones (program, startup,
                                            service, task changes)
  setupscope slowdown [--from YYYY-MM-DD] [--to YYYY-MM-DD] [--json]
                                            What changed in the period, ranked
                                            by measured startup impact where
                                            Windows has it
  setupscope gatekeeper [--json]               Review new autostart entries
  setupscope gatekeeper scan                   Run one detection pass
  setupscope gatekeeper allow <id>             Keep an entry, mark it reviewed
  setupscope gatekeeper remove <id> [--yes]   Back up and remove an entry
  setupscope gatekeeper enable|disable         Turn the Gatekeeper on or off
  setupscope watch                             Background auto-monitoring (started
                                            at sign-in when turned on in Settings)
  setupscope check-clean <log-id>              Check that nothing recorded in a log
                                            is still on the PC (exit 2 if so)
  setupscope restore-point [list]              List System Restore points
  setupscope restore-point create [DESCRIPTION]
  setupscope restore-point delete <number> [--yes]
  setupscope restore-point rollback <number> --yes
                                            Restart Windows into a restore point
  setupscope backups                           List backups
  setupscope restore <backup-id> [--yes]       Undo a removal from its backup
  setupscope pa-capture <job.json>             Run a PortableApps Forge job file
                                            (the Sandbox capture runs this itself;
                                            advanced, driven entirely by the file)
  setupscope --version | --help

  Any command that takes snapshots accepts --profile NAME to use a saved
  monitoring profile (Settings > Monitoring scope).
  monitor and diff also accept --json to print the saved log's summary.

  Run `setupscope <command> --help` for one command's flags, what it
  changes, and how it asks before destructive actions.

CONFIRMATION:
  Commands that remove or overwrite things (revert, uninstall, clean,
  components --strike, restore, transfer import, gatekeeper remove,
  restore-point delete and rollback, rewind --to) ask before they act:
  in a terminal they say what will happen and wait for a typed \"yes\";
  without a terminal (scripts) they refuse and exit 1 unless --yes is
  given. Read-only commands and --dry-run previews never ask. The one
  exception is `clean --scheduled`, the Task Scheduler entry point: the
  schedule you set is itself the instruction, so it never asks.

Exit codes: 0 success, 1 error (including \"not confirmed\"), 2 finished
with failures / findings.

Typical CI use (installer QA):
  setupscope monitor --name MyApp --no-prompt --json -- dist\\MyApp-Setup.exe /S
  setupscope revert <log-id> --keep-log --yes
  setupscope check-clean <log-id>";

/// One command's own help: usage, what it changes, its flags, and how it
/// confirms. Printed for `setupscope <command> --help`, which exits 0
/// without doing anything.
pub(super) const COMMAND_HELP: &[(&str, &str)] = &[
    (
        "monitor",
        "\
setupscope monitor — record an installation as an install log

Usage: setupscope monitor [--name NAME] [--no-prompt] [--json] -- <installer> [args...]

Takes a snapshot, runs the installer you name, waits for it to finish,
snapshots again and saves the difference as an install log. Running the
installer changes your PC the way the installer chooses — that is the
point of the command, and the log is what lets you undo it precisely
with `revert`. Without --no-prompt it waits for Enter before the second
snapshot; --json prints the saved log's summary.
Confirmation: none — the installer is run exactly as you invoked it.",
    ),
    (
        "snapshot",
        "\
setupscope snapshot — save a snapshot of the system

Usage: setupscope snapshot <file.snap> [--profile NAME]

Records the monitored registry keys and folders into <file.snap>, to
compare later with `diff`. Changes nothing on the PC.
Confirmation: none (writes only the file you named).",
    ),
    (
        "diff",
        "\
setupscope diff — compare two snapshots and save an install log

Usage: setupscope diff <before.snap> <after.snap> [--name NAME] [--json]

Saves the difference as an install log. (--auto is used by background
monitoring and tags the log as recorded automatically.)
Confirmation: none (writes only the log).",
    ),
    (
        "logs",
        "\
setupscope logs — list saved install logs

Usage: setupscope logs [--group G] [--tag T] [--json]

Read-only. --json prints the list as JSON.
Confirmation: none.",
    ),
    (
        "tag",
        "\
setupscope tag — set an install log's group and tags

Usage: setupscope tag <log-id> [--group G] [--tags a,b]

Edits only the log's own metadata; removes nothing.
Confirmation: none.",
    ),
    (
        "programs",
        "\
setupscope programs — list installed programs

Usage: setupscope programs [--all] [--trust] [--json]

Read-only. --all includes programs Windows lists only per-user or
hidden; --trust adds each program's digital-signature state; --json
prints JSON.
Confirmation: none.",
    ),
    (
        "verify",
        "\
setupscope verify — check a file's digital signature

Usage: setupscope verify <file>

Read-only. Exits 0 when the signature is valid, 2 when the file is
unsigned or its signature is invalid.
Confirmation: none.",
    ),
    (
        "revert",
        "\
setupscope revert — undo an installation precisely

Usage: setupscope revert <log-id> [--dry-run] [--remove-unlogged] [--no-backup]
       [--recycle] [--close-programs] [--restart-explorer] [--keep-log] [--yes]

Removes what the recorded installation added: files, folders, registry
keys and values, services, tasks and shortcuts. Items other programs
also use are never selected automatically (printed as [refs]).
A backup is taken first, so the removal can be undone with
`setupscope restore <backup-id>` — unless --no-backup is given, and
then the removal cannot be undone. The log is deleted afterwards
unless --keep-log. --dry-run prints the plan and changes nothing.
Confirmation: destructive. Asks before removing; scripts must pass
--yes. Exit codes: 0 done, 2 some items failed, 1 error / not confirmed.",
    ),
    (
        "rewind",
        "\
setupscope rewind — App Rewind (early prototype): rewind one program

Usage: setupscope rewind <log-id> --list
       setupscope rewind <log-id> --capture [--before]
       setupscope rewind <log-id> --to <state> [--dry-run] [--yes]

--list shows the program's recorded states (read-only). --capture
records the program's current file contents as a new state in the
content store. --to rewinds the program to a recorded state: the plan
is printed first, including anything that cannot be restored; a
partial rewind is reported as partial and exits 2.
Confirmation: --to is destructive. A backup of the current state is
taken first, so the rewind itself can be undone; it asks before
running, and scripts must pass --yes.",
    ),
    (
        "export",
        "\
setupscope export — export an install log

Usage: setupscope export <log-id> <html|json|reg|ps1|xml> <output-file>
       setupscope export <log-id> portable <launcher.ini>

Writes the log in the chosen format. Note the reg and ps1 forms are
undo scripts: running them changes the PC the way running any script
does. This command only writes the file.
Confirmation: none.",
    ),
    (
        "repackage",
        "\
setupscope repackage — build a silent installer from an install log

Usage: setupscope repackage <log-id> <folder> [--no-setup]

Writes a folder holding the recorded installation's files plus
install.ps1, install.reg and uninstall.ps1, and (unless --no-setup)
a combined Setup.exe. Changes nothing on this PC.
Confirmation: none.",
    ),
    (
        "merge",
        "\
setupscope merge — combine install logs into a new one

Usage: setupscope merge <log-id> <log-id>... [--name NAME]

The originals are kept. The merged log is printed (its id) on stdout.
Confirmation: none.",
    ),
    (
        "learn-noise",
        "\
setupscope learn-noise — learn what changes on its own

Usage: setupscope learn-noise [--seconds N] [--apply] [--json]

Takes two snapshots N seconds apart (default 60) and lists what
changed by itself. --apply adds those places to the monitoring
exclusions in Settings. Without --apply nothing is changed.
Confirmation: none.",
    ),
    (
        "transfer",
        "\
setupscope transfer — move programs to another PC

Usage: setupscope transfer export <out.setupsweep> <program>...
       setupscope transfer check <package>
       setupscope transfer import <package> [--overwrite] [--yes]

export packs the named programs with their files and settings.
check compares a package against this PC without changing anything.
import writes the package onto this PC: files and registry settings
are restored, replacing what is there only with --overwrite; a backup
is taken first, so the import can be undone with
`setupscope restore <backup-id>`.
Confirmation: import is destructive. It asks first; scripts must pass
--yes.",
    ),
    (
        "portable-build",
        "\
setupscope portable-build — build a PortableApps.com package

Usage: setupscope portable-build <installer> --name NAME [--version V]
       [--publisher P] [--category C] [--silent] [--out DIR] [--here]
       [--defaults] [--no-prompt] [--no-compile]

Captures an installation in Windows Sandbox and builds a
PortableApps.com package with launcher and .paf.exe installer. With
--here (or when Sandbox is unavailable) the installer instead runs on
this PC and is recorded as an install log, so it can be removed
precisely with `revert`. --no-compile stops after the package folder.
Confirmation: none of its own — on-this-PC capture runs the installer
you named, exactly as invoked.",
    ),
    (
        "add-program",
        "\
setupscope add-program — list a program Windows doesn't know

Usage: setupscope add-program <folder|program.exe> [--name NAME]

Adds a hand-made entry (e.g. a portable program) to SetupScope's
data so it can be analyzed and uninstalled. Changes no system state.
Confirmation: none.",
    ),
    (
        "components",
        "\
setupscope components — Installer Decrapifier for one install log

Usage: setupscope components <log-id> [--json] [--ask-model] [--strike] [--yes]

Lists the services, tasks and autoruns the installation added, each
judged with its reasons. --ask-model asks the local model configured
in Settings about the ones the rules can't judge (this PC only).
--strike removes the components judged bloat: a backup is taken
first, so they can be restored with `setupscope restore <backup-id>`,
and the log is updated.
Confirmation: --strike is destructive. It asks first; scripts must
pass --yes. Without --strike the command is read-only.",
    ),
    (
        "orphans",
        "\
setupscope orphans — registrations left by removed programs

Usage: setupscope orphans [--json]

Read-only: lists registrations pointing at files that no longer
exist. Exits 2 when any are found. Remove them in the app, where they
are reviewed and backed up first.
Confirmation: none.",
    ),
    (
        "uninstall",
        "\
setupscope uninstall — uninstall programs, then remove safe leftovers

Usage: setupscope uninstall <name>... [--silent] [--leftovers safe|none]
       [--dry-run] [--yes]

Runs each program's own uninstaller one after another (--silent uses
its silent mode). What a program's own uninstaller changes is NOT
backed up and SetupScope cannot undo it. Afterwards, safe leftovers
are removed with a backup (skip with --leftovers none). --dry-run
prints the commands and changes nothing.
Confirmation: destructive. Asks before starting; scripts must pass
--yes. Exit codes: 0 done, 2 some programs failed, 1 error / not
confirmed.",
    ),
    (
        "clean",
        "\
setupscope clean — clean junk files

Usage: setupscope clean [--categories a,b] [--list] [--dry-run] [--yes]
       setupscope clean --scheduled [--categories a,b]

Deletes junk in the chosen categories (default: the safe kinds);
--list shows each category's size and changes nothing. Cleaned files
are deleted permanently: they are NOT backed up and SetupScope
cannot restore them. Paths on the Cleaner page's Protected list are
always kept. --dry-run reports sizes and deletes nothing.
--scheduled is the Task Scheduler entry point (see schedule-clean):
the schedule itself is the instruction, so it never asks and never
takes --yes.
Confirmation: destructive (except --list, --dry-run and --scheduled).
Asks before deleting; scripts must pass --yes.",
    ),
    (
        "schedule-clean",
        "\
setupscope schedule-clean — clean junk automatically

Usage: setupscope schedule-clean daily|weekly|monthly|off [--categories a,b]

Creates (or, with off, removes) the Windows Task Scheduler task that
runs `setupscope clean --scheduled`. What each run freed is recorded
in cleaning.log in the data folder.
Confirmation: none — the schedule is set or removed exactly as asked;
each scheduled run is the standing instruction.",
    ),
    (
        "startup",
        "\
setupscope startup — list and toggle startup programs

Usage: setupscope startup [--json]
       setupscope startup enable|disable <name>

Lists the programs that start with Windows. enable/disable turns one
on or off: the change is backed up first and can be undone with
`setupscope restore <backup-id>` or the inverse command. Entries
Windows, security software or hardware need cannot be disabled.
Confirmation: none (backed up and reversible); listing is read-only.",
    ),
    (
        "history",
        "\
setupscope history — the recorded change history

Usage: setupscope history [--since YYYY-MM-DD] [--json]

First records observed program, startup, service and scheduled-task changes
into history.json, then lists them, newest first. Inventory omissions can
reflect access limits; an observed change does not establish its cause.
Confirmation: none.",
    ),
    (
        "slowdown",
        "\
setupscope slowdown — Slowdown Detective report

Usage: setupscope slowdown [--from YYYY-MM-DD] [--to YYYY-MM-DD] [--json]

What changed in the period (default: the last 90 days), ranked by
measured startup impact where Windows has a measurement; unmeasured
changes are listed unranked, never as zero impact.
Confirmation: none.",
    ),
    (
        "gatekeeper",
        "\
setupscope gatekeeper — Autostart Gatekeeper review queue

Usage: setupscope gatekeeper [--json]
       setupscope gatekeeper scan
       setupscope gatekeeper allow <id>
       setupscope gatekeeper remove <id> [--yes]
       setupscope gatekeeper enable|disable

New autostart entries wait in a review queue; nothing is ever blocked
live or removed without your decision. allow keeps an entry; remove
deletes it, backed up first (undo with `setupscope restore
<backup-id>`). scan runs one detection pass (when the Gatekeeper is
off, it scans nothing). enable/disable turn the Gatekeeper on or off.
Confirmation: remove is destructive. It asks first; scripts must
pass --yes. The rest never ask.",
    ),
    (
        "watch",
        "\
setupscope watch — background auto-monitoring

Usage: setupscope watch

Keeps a snapshot and, when a new program registers itself, records an
install log tagged \"auto\" once the installation settles. Runs until
stopped; started at sign-in when turned on in Settings. Records logs
only — it never removes anything.
Confirmation: none.",
    ),
    (
        "check-clean",
        "\
setupscope check-clean — was an installation fully removed?

Usage: setupscope check-clean <log-id>

Read-only. Exits 0 when nothing the log recorded is still on the PC,
2 when something was left behind (the leftovers are printed).
Confirmation: none.",
    ),
    (
        "restore-point",
        "\
setupscope restore-point — System Restore points

Usage: setupscope restore-point [list]
       setupscope restore-point create [DESCRIPTION]
       setupscope restore-point delete <number> [--yes]
       setupscope restore-point rollback <number> [--yes]

list is read-only. create makes a restore point. delete removes one
permanently — that rollback point is gone for good. rollback restarts
Windows right away and rolls system files and settings back to the
chosen point; programs installed after it may stop working, and
System Restore does not touch your documents.
Confirmation: delete and rollback are destructive. They ask first;
scripts must pass --yes.",
    ),
    (
        "backups",
        "\
setupscope backups — list backups

Usage: setupscope backups

Read-only. Backups left by interrupted operations are marked, with
the command that undoes them.
Confirmation: none.",
    ),
    (
        "restore",
        "\
setupscope restore — undo a removal from its backup

Usage: setupscope restore <backup-id> [--yes]

Puts back the files and registry settings the backup holds,
replacing what is on the PC now. (Find ids with `setupscope
backups`.) The restore itself takes no new backup.
Confirmation: destructive. Asks before restoring; scripts must pass
--yes. Exit codes: 0 done, 2 some items failed, 1 error / not
confirmed.",
    ),
    (
        "pa-capture",
        "\
setupscope pa-capture — run a PortableApps Forge job file

Usage: setupscope pa-capture <job.json>

Runs the capture job described by a job file written by PortableApps
Forge (the Windows Sandbox capture runs this itself). Advanced:
the job file is the whole instruction, so there is no confirmation
and no --yes.
Confirmation: none (driven entirely by the job file).",
    ),
];
