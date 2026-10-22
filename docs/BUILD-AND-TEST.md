# Reviewed Build And Test

Open **Build Troubleshooting → Reviewed Build / Test**. Execution is separate from read-only log inspection. Nothing executes on project inspection, preset selection or review preparation.

1. Select an existing absolute local project folder and the trusted tool's actual `.exe`. No PATH lookup or shell is used. For MSVC, start SetupScope in the intended developer environment or configure the tool's INCLUDE/LIB and other required paths.
2. Choose a preset and adjust arguments: one literal argument per line, without shell quotes. Spaces stay within an argument; empty interior lines represent empty arguments. SetupScope does not evaluate shell substitutions, but the executable interprets its own arguments.
3. Choose an existing log folder and timeout (1–7200 seconds). Rust defaults use `--locked --offline`; .NET uses `--no-restore`. These do not restrict network access by project code. CMake build/test defaults assume an existing `build` directory; configure separately or select actual paths/presets. MSBuild needs a suitable project or explicit project argument.
4. Prepare the review, inspect executable/arguments/paths/risks, acknowledge it, then Run. Editing inputs clears approval. Settings are session-only; project declarations never authorize execution.
5. Inspect completion/failure/cancellation/timeout. Use Captured Log For Triage selects the log; inspection is another explicit action. Pattern hints and exit zero do not establish correctness.

## Safety And Recovery

Build hooks, dependencies, scripts and tests are arbitrary code with SetupScope's current account, environment, credentials and network access. No elevation request or sandbox is provided. An elevated parent produces elevated children; prefer a normal account. Files, package downloads and remote effects cannot be automatically undone.

Windows creates the process suspended, restricts inherited handles to stdin/stdout/stderr, and assigns a Job Object before allowing execution. Failed containment blocks execution; stdin reads EOF. Cancellation/timeout ends the contained tree, and remaining descendants also stop on root exit. This is for finite builds/tests, not persistent servers/watchers. Already-existing services/processes contacted by code are outside the tree. Lifetime control is not a security sandbox.

A unique log is created and its prepared header flushed before launch. Without final status, outcome is unknown; inspect project state before retrying. Each stream retains up to 1,015,808 bytes, drains/discards excess and replaces invalid UTF-8. Combined bounded output/header/footer stays below 2 MiB. Stdout/stderr are separate sections, not chronological interleaving. Triage may skip long/excess lines. Post-execution log-save failures are explicit; logs may be incomplete. Logs can contain secrets, arguments and paths, stay local and are excluded from automatic diagnostic export. Review/redact before sharing; remove unwanted logs manually.

Limits: 64 arguments, 4096 bytes each, 16000 bytes combined; local paths under 240 characters; no network/device/alternate-stream/reparse paths. File/path holds reduce races, not attacks by same-user processes, administrators or kernel attackers. Development executables need not be signed; the user chooses/trusts them.

## Verification

Windows fixture tests exercise capture/triage, nonzero exits, consent/pre-cancellation, running cancellation, timeout, output floods, invalid UTF-8 and detached-child cleanup. They execute isolated test code, not vendor installers, and do not verify every project/toolchain combination.
