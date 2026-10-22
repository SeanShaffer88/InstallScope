# Developer CLI

Use **SetupScope-cli.exe** for terminal sessions, pipelines and automation. It has
a normal Windows console entry point: PowerShell waits for completion and receives
stdout, stderr and the exit code. The GUI executable accepts the same commands,
but the console executable is preferable for scripting. Neither command entry
point opens a second GUI instance or revives retired maintenance commands.

Run `SetupScope-cli.exe --help` for the full current syntax. Paths with spaces
must be quoted by the calling shell. Project, executable and inspection paths
follow the same local-path checks and bounded evidence rules as the desktop tools.

```powershell
.\SetupScope-cli.exe system --json
.\SetupScope-cli.exe readiness 'C:\Dev\Example' x64
.\SetupScope-cli.exe toolchain cmake x64 cmake 3.28 5.0
.\SetupScope-cli.exe triage 'C:\Dev\Logs\build.log'
.\SetupScope-cli.exe containers
.\SetupScope-cli.exe ports 8080 - 0
.\SetupScope-cli.exe file-owners 'C:\Dev\Example\output.dll' 0
.\SetupScope-cli.exe diagnostics cmake x64 > reviewed-counts.json
```

`toolchain` profiles are `rust`, `dotnet`, `node`, `python`, `cmake`; targets
are `x64`, `arm64`. Optional version bounds are numeric Windows product-version
metadata, minimum inclusive and maximum exclusive. Use `-` to omit a bound.
Proxies/SDK hosts may remain unknown. No tool is executed to discover its version.
Ports include TCP listeners and UDP bound endpoints; page numbers start at zero
with 50 rows per page. Use `-` for an omitted port or PID filter.

## Dependencies

```powershell
.\SetupScope-cli.exe dependencies catalog
.\SetupScope-cli.exe dependencies download 'C:\Dev\Library' dotnet-10.0.402-x64
.\SetupScope-cli.exe dependencies verify 'C:\Dev\Library' dotnet-10.0.402-x64
.\SetupScope-cli.exe dependencies export 'C:\Dev\Library' 'D:\Offline\NewBundle' dotnet-10.0.402-x64 --accept-license
.\SetupScope-cli.exe dependencies import 'C:\Dev\Library' 'D:\Offline\NewBundle'
.\SetupScope-cli.exe dependencies review 'C:\Dev\Library' dotnet-10.0.402-x64
.\SetupScope-cli.exe dependencies install 'C:\Dev\Library' dotnet-10.0.402-x64
.\SetupScope-cli.exe dependencies receipts 'C:\Dev\Library'
```

Download is an explicit network operation. Verify/import/export use the same
compiled IDs, expected bytes/digests and cached publisher checks as the GUI.
An export destination must be new; `--accept-license` means you have reviewed
selected vendors' licensing and redistribution terms. Do not redistribute
payloads just because their download is public. Install displays an exact review,
then requires typing `INSTALL` in an interactive terminal before opening vendor
UI. There is no silent-install or automatic-rollback promise. Receipts may contain
local paths. Native Visual Studio component/layout setup remains a GUI workflow.

## Environments and recovery

```powershell
.\SetupScope-cli.exe environment-preview 'C:\Dev\profile.json'
.\SetupScope-cli.exe environment-user-preview 'C:\Dev\profile.json'
.\SetupScope-cli.exe environment-apply 'C:\Dev\SetupScopeData' 'C:\Dev\profile.json'
.\SetupScope-cli.exe environment-history 'C:\Dev\SetupScopeData'
.\SetupScope-cli.exe environment-receipt 'C:\Dev\SetupScopeData' TRANSACTION_ID
.\SetupScope-cli.exe environment-undo 'C:\Dev\SetupScopeData' TRANSACTION_ID
```

Profiles use the existing bounded JSON format described in
[Project Environments](PROJECT-ENVIRONMENTS.md). Preview never applies changes.
Persistent apply prints the prepared transaction and requires typing `APPLY`.
Undo prints the saved receipt and requires `UNDO`; it retains the existing
identity, prior-state and concurrent-change checks. Only supported developer
variables in HKCU are edited. Existing applications can retain stale values.
Keep recovery data private and preserve it until you no longer need undo.
Environment overlays can be launched through the GUI; there is no CLI overlay
launch command yet.

## Reviewed build/test execution

```powershell
# Preview only; creates no log and starts no process:
.\SetupScope-cli.exe run 'C:\Dev\Example' 'C:\Tools\cargo.exe' 'C:\Dev\Logs' 600 -- test --locked --offline
# Review again and type RUN in the terminal to execute:
.\SetupScope-cli.exe execute 'C:\Dev\Example' 'C:\Tools\cargo.exe' 'C:\Dev\Logs' 600 -- test --locked --offline
```

Everything after `--` is a literal executable argument, including `--json`.
There is no shell or PATH lookup. Timeout is 1–7200 seconds. Execution inherits
your account, credentials, environment and network access; it is not a sandbox.
The existing suspended-process/Job Object containment, bounded pipe drains and
unique logs apply. Timeout stops the contained tree; stopping cannot undo side
effects or external services. Standard input for the child is closed.
Use the GUI when you need its cooperative cancellation button.

No noninteractive confirmation bypass exists for install, execute, apply or undo.
Piped stdin/stderr is rejected before launching or applying. Download, bundle
transfer and verification are explicit named operations and do not prompt.

## Output and exit codes

`--json` before the `--` delimiter returns a schema-1 envelope with `command`,
`ok`, and a **text** `report` or `error`. Findings are not a typed JSON schema yet.
`diagnostics` without an envelope emits the existing redacted JSON bundle directly;
`dependencies catalog` emits the existing catalog JSON. Other reports and receipts
may contain local paths and explicitly selected environment values. Review before
sharing; arbitrary CLI output is not a redacted bundle.

- `0`: command completed. Diagnostic evidence can still be partial, unknown or
  incompatible; successful inspection does not certify a build or installation.
- `1`: operational failure, declined/noninteractive review, failed/timed-out
  execution or log-save failure. Child exit details are in the error/log.
- `2`: malformed invocation: unknown command or subcommand, invalid argument
  count, or a missing `--` delimiter/`--accept-license` flag. Well-formed requests
  that fail validation (for example an unknown package ID) return `1`.

CLI identifiers, diagnostic evidence and help currently remain English for stable
scripting and fidelity. `languages` lists desktop language catalogs. Complete
translated CLI help and typed finding output remain future work.
