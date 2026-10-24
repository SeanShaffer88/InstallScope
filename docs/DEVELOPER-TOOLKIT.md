# Developer toolkit delivery plan

> **Superseded scope (updated 2026-10-24).** This 2026-10-21 delivery plan is a
> historical decision record. Its "Retained and retired" list still names
> programs/uninstalls, browser extensions, install monitoring, storage
> inspection, debloat and privacy controls as retained; the confirmed scope in
> `AGENTS.md` and the README now keeps exactly five developer tools plus the
> integrated dependency, diagnostic-export and recovery workflows. Nothing in
> that retired list is compiled today. Read this page for provenance, not as a
> description of current capability.

Confirmed scope, 2026-10-21. All processing stays local by default. No paid
hosting, accounts, automatic downloads, telemetry or new background service.
These are requested development tasks, not claims of existing functionality.
The completion goal includes implementing and verifying these accepted additions;
passing tests for the older feature set does not finish the expanded goal.

## Retained and retired

Retain general programs/uninstalls and browser-extension management, install
monitoring, install logs, snapshots, backups/rollback, portable/setup packaging,
read-only storage inspection, developer diagnostics, debloat and privacy /
anti-telemetry controls. Privacy controls must describe their documented limits;
no claim of eliminating all Windows network traffic.

Remove general junk/browser-data cleanup, startup/task manager, Autostart
Gatekeeper, Slowdown Detective, System Restore manager and context-menu editor
from the product UI and CLI. Keep internal startup/service/task/shell capture and
reviewed application-leftover removal because installers can create these items.
Keep pre-uninstall restore-point safeguards and the install-monitor Explorer verb.
Retire legacy cleanup scheduling entry points without deleting existing Windows
tasks or old saved data. Old executable copies may still run their old behavior;
replacing this portable executable does not update every installed copy.

## Accepted additions, in implementation order

1. **Project Doctor (initial read-only workflow implemented; expansion/verification open):** explicitly choose a project; inspect bounded known
   manifest/version files, declared SDK/compiler/runtime requirements, installed
   candidates, PATH precedence, architecture and Windows prerequisites. Start
   with .NET global.json, Rust toolchain files, Node/Python version declarations
   and existing diagnostics evidence. Never run project scripts during diagnosis.
   Report missing, incompatible, unknown and unsupported constraints separately.
   Respect each ecosystem's actual resolution rules; numeric file metadata alone
   is insufficient proof of a working or compatible toolchain.
   Current support and evidence: [Project Doctor](PROJECT-DOCTOR.md).
2. **Installer regression workspace (initial workflow implemented; live acceptance open):** guided before/install/upgrade/uninstall
   comparisons, reproducible fixture plans, versioned local evidence and reviewed
   leftover reports. Include failures, cancellation, reboot-required outcomes and
   explicit incomplete states. Vendor operations require review and appropriate
   isolation; no automatic testing on the owner's installed software.
   Snapshot prerequisites now reject expanded input above 512 MiB (callers may
   choose a smaller limit), preserve gzip integrity checks, and refuse success
   after a file worker fails. Optional cooperative capture item limits distinguish
   exhausted scope from user cancellation; they are not hard memory caps. The
   guided regular-interface workspace now saves versioned scope/attempt receipts,
   handles pending/failed captures, reported vendor outcomes and reviewed paged
   comparisons. No vendor operations run. [Workflow, limits and open gates](INSTALLER-REGRESSION.md).
3. **Project environment profiles (initial workflow implemented; live acceptance open):** preview/export/import PATH and environment
   diffs; process-local launch environments by default. Explicit review, conflict
   detection, exact prior-state backup and undo for any persistent user-level
   change. Do not include secrets or silently edit machine-wide settings.
   Explicit developer path schema, child-only launch and HKCU recovery receipts
   are implemented; no inherited environment dump or automatic activation.
   [Support, limits and evidence](PROJECT-ENVIRONMENTS.md).
4. **Ports and locks inspector (initial workflow implemented; live acceptance open):** read-only local listening-port/process evidence
   and an explicitly selected file's lock owners where Windows exposes them.
   Show access failures, PID reuse/refresh uncertainty and unsupported cases.
   Do not kill processes, unlock files, alter firewall rules or connect to services.
   TCP/optional UDP IPv4/IPv6 tables, checked single-file Restart Manager reports,
   PID/start-time evidence and paged explicit copying are implemented.
   [Support, limits and verification](PORTS-AND-LOCKS.md).
5. **Diagnostic bundles (initial workflow implemented; live UI acceptance open):** bounded local reports with preview, selectable sections,
   secret/path redaction and no-replace export. Include evidence dates and unknowns;
   exclude raw environment values, credentials and arbitrary project contents.
   Export is explicit; uploading or messaging a bundle is a separate user action. Readiness summaries, fixed tool counts and runtime sections use allowlist redaction, exact preview review and bounded no-replace JSON export. [Support and limits](DIAGNOSTIC-BUNDLES.md).

## Further accepted developer enhancements

- WSL/container host readiness initial workflow is implemented in Developer diagnostics: fixed CLI-file candidates, query-only service states, processor hints and override-name presence. Runtime/guest health and optional features remain unverified. [Limits and verification](WSL-CONTAINERS.md). Do not install,
  start distributions/engines or change virtualization/security settings on scan.
- Local build-log triage (initial workflow implemented; broader acceptance open): 2 MiB saved UTF-8/BOM UTF-16 input, bounded lines, grouped static hints/source line numbers and raw-line exclusion. [Limits and verification](BUILD-LOG-TRIAGE.md). Identify common errors and link
  to applicable evidence without executing suggested commands automatically.
- Reviewed project launch/build recipes: show exact command, working directory,
  executable resolution and environment before running; capture exit/cancellation
  and distinguish restricted-token containment from a security sandbox.
- Dev Drive/workspace guidance: read-only capability and filesystem inspection;
  user-controlled handoff to Windows settings, no automatic reformat/security change.
- Dependency/provenance inventory from supported lockfiles: local metadata and
  missing/unsupported evidence, not a vulnerability or license-compliance guarantee.
- Developer storage analysis: attribute project outputs/caches without deletion;
  preserve the removal of general PC cleanup tools.
- Local release preflight: architecture/version/signature/manifest/package checks,
  artifact hashes and explicit unverified platform/installer/runtime gates.

## Acceptance for every delivered slice

The [saved-reference fit review](REFERENCE-FIT-REVIEW.md) adds concrete work for
Visual Studio discovery, DLL dependency inspection, broader archive support,
MSIX preflight and isolated UIA regression coverage. Candidate dependencies are
not adopted merely by appearing in the catalog; validate each chosen integration.

Provide a complete reachable workflow, bounded inputs, actionable errors,
background work/cancellation where appropriate, keyboard and accessibility
semantics, stable local formats, preservation of existing data and honest limits.
Use meaningful fixture and Windows tests; test every consequential mutation's
failure/recovery path. Build and package locally. Record actual runtime evidence
separately from component tests. Live accessibility, platform compatibility and
isolated end-to-end gates remain open until demonstrated; do not claim “ultimate”
or 100% from feature count or green unit tests alone.

## Primary references for upcoming implementation

- [.NET global.json SDK search and roll-forward rules](https://learn.microsoft.com/en-us/dotnet/core/tools/global-json).
- [Windows Dev Drive support and configuration](https://learn.microsoft.com/en-us/windows/dev-drive/).

Consult current official ecosystem/platform specifications before implementing
resolution or mutating integrations. These links establish design inputs, not
verification that SetupScope has implemented the corresponding behavior.
