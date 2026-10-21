# Roadmap

Planned for future releases, roughly in priority order. Vote or comment on the
matching GitHub issue to move something up. Features that already exist are
listed in the [README](README.md#features).

## Product direction

SetupSweep is a cohesive local Windows toolkit for reclaiming disk space,
reviewing bloat and telemetry, thorough program removal, install monitoring,
program migration, developer containment, temporary scratch storage, conservative
registry repair, and read-only VSS browsing. Existing tools and planned tools must
remain clearly distinguished.

Prioritize shared review, recovery, actionable diagnostics, and accessible guidance
before adding more surfaces. New and experienced users should follow the same
predictable sequence: scan, explain, review, back up where supported, apply,
verify, and undo where feasible. Irreversible cleanup must say plainly that no
backup or undo is available. Never promise completely safe registry cleanup:
restrict repair to evidence-backed findings, keep uncertain entries by default,
and back up mutations. Avoid implying registry entry counts measure performance.

## Next

The core 1.0 feature set is substantially implemented. The next priorities are
the remaining product goals and a staged UI transition; each should ship as a
bounded, tested workflow rather than a broad rewrite.

**Core product**
- **Storage overview** — the regular interface now has an initial read-only,
  cancellable logical-size survey with bounded top rows and visible partial
  results. Windows client verification remains pending. Later: allocated and
  hard-link-aware accounting, accessible treemap alternatives, drill-down and
  saved growth comparisons. Do not treat logical size as recoverable space.
- **Native Win32 interface** — replace `egui`/`eframe`/`wgpu` in phases while
  preserving the established dark, black, and red design. Migrate and verify
  complete workflows one at a time; keep the existing working UI until each
  replacement is ready. The experimental Programs preview now includes
  browsing, search, details, window-targeted identification, desktop and
  Store-app uninstalls, reviewed leftover cleanup, analysis-only manual entries,
  and sequential batch uninstall with a separate cleanup review per program.
  Restore-point integration and full-flow parity remain.
- **Window-targeted program identification** — a picker is prototyped in the
  native preview (point at a program's window to identify it). Validate it in the full
  production interface; match using a unique executable-path/install-location
  relationship, never a window title, and do not trigger actions automatically.
- **Browser extension inventory** — group extensions by browser and profile,
  with a safe, reversible disable/restore workflow. Preserve profile data and
  make every change explicit.
- **Setup and archive compatibility tiers** — publish a tested support matrix
  for installer and archive formats, then expand coverage by user value.
  “All formats” is an aspiration, not a literal compatibility promise; refuse
  unsupported or malformed inputs safely.
- **Setup Package Creator** — the Logs page can already build a silent
  `Setup.exe` plus a deploy folder from a captured installation. Extend this
  into a guided, reviewable project informed by the capabilities documented by
  [InstallForge](https://docs.installforge.net/getting-started/features/) and
  [Setup Factory](https://www.indigorose.com/setup-factory/features/): project
  identity and target matrix; file and folder selection with destination
  mapping; install types/components; prerequisites; shortcuts, registry and
  other explicit install actions; customizable, localizable wizard pages;
  compression; generated uninstall support; optional code signing; and a
  source-control-friendly project with logged command-line builds.
  **Completeness is a build requirement:** inventory and package every
  application-owned file, resource, library, runtime, and transitive dependency
  found in the selected build inputs and monitored installation. Combine
  dependency inspection with monitored test installs, report unresolved or
  missing inputs, and block builds with unresolved required dependencies rather
  than silently producing incomplete packages. Never copy Windows-owned or
  shared system binaries into an application package or bypass a dependency's
  redistribution terms. For those dependencies, identify the supported OS or
  prerequisite, accept only an authorized and verified redistributable source,
  and make any remaining requirement visible to the user. Keep every inclusion
  and install action reviewable, and make uninstall behavior explicit. Retain
  silent log redeployment as the fast path; do not imply arbitrary installers
  can be safely reconstructed from incomplete logs or that dependencies
  outside the inspected inputs can be discovered with certainty.
- **Disposable Runner for developers** — provide an in-app way to run arbitrary
  test applications and scripts in a disposable environment, distinct from
  the existing Windows Sandbox installer-preview flow. Use the agreed hybrid
  restricted-token plus Job Object containment, a per-run network toggle
  enabled by default, bounded resources, visible containment limits, and
  reliable cleanup. Do not describe it as a VM or security boundary stronger
  than its implementation.
- **VSS Viewer** — browse Volume Shadow Copy snapshots by volume and time,
  inspect their contents read-only, and copy selected files to a chosen
  destination. Keep snapshot deletion and system rollback in the existing
  System Restore manager; never modify a snapshot's contents.
- **Temporary scratch volume** — create and remove an explicitly sized,
  temporary workspace for testing, builds, or extraction. Prefer a bounded,
  dynamically sized temporary VHDX over a RAM disk to avoid consuming
  unpredictable system memory; clearly disclose persistence, capacity,
  elevation, detach failures, and cleanup/recovery behavior. This is storage
  isolation, not application sandboxing.

**Innovation track** — new capabilities vetted against the project's constraints
(offline, local, least-privilege, no new product surfaces). Suggested build order:
reference counting → Canary → Slowdown Detective → Autostart Gatekeeper →
App Rewind. The first four are now implemented at least in part; each bullet
states what has shipped and what is still open.
- **Shared-component reference counting** — shipped in the removal review
  (and `setupsweep revert`): every item is cross-checked against all install
  logs and labeled from real data — *Used only by this program* (safe),
  *Shared with N others — removing may break them* (Careful), or *No install
  log mentions this* (unknown; never rendered as "unused"). Shared and unknown
  items always default to keep, including under *Select all*, and the review
  states how many programs the reference data covers. The build keeps the
  contract that the index is rebuilt from the logs on every review, so adding,
  removing or merging a log can never leave a stale label behind.
- **Canary — post-operation health verification** — shipped as a short,
  read-only pass at the end of every removal plan, appending its verdict to the
  same report the removal shows: a green all-clear line, or specific findings
  (an autostart entry now pointing at a missing file, a shell extension whose
  DLL is gone, an association that no longer resolves, a protected location the
  operation touched). It stays high-precision, checks existence only, loads no
  removed code and uses no network; the removal's backup remains the undo.
  Still open: a **Fix** action on a finding, an "Undo this operation" shortcut
  beside it, and running the same pass after cleaning operations (today it
  runs on removal plans).
- **Slowdown Detective** — history capture, CLI reports and a read-only GUI now
  cover recorded installs, updates, removals, autostart changes, and observed
  service and scheduled-task configuration changes, with inclusive date filters
  and current startup measurements where available. Remaining: before/after measurements and contextual
  Delay/Disable/Uninstall actions through reviewed safety workflows. Timing alone
  must never be presented as proof of causation.
- **Autostart Gatekeeper** — opt-in (Advanced) review of the programs that
  start with Windows. Detection, attribution and the Decrapifier verdicts are
  wired to both the command line (`setupsweep gatekeeper`) and an
  **Autostart Gatekeeper** page, where each new or changed Run/RunOnce entry or
  Startup-folder shortcut waits for an explicit *Keep* or *Remove* (backed up
  first). The weekly, non-modal dashboard digest and optional scan after a
  Monitor-page installation are implemented;
  CLI installs and the separate automatic-monitor service are not covered.
  Remaining work: extending detection to services and scheduled tasks, and
  auto-disabling only high-confidence bloat — everything reversible
  and every attribution labeled with its evidence level.
- **App Rewind — per-program time travel** — for monitored or analyzed
  programs, keep a recorded state history (at install and at updates); rewind
  just that program to an earlier state from a plain-language History timeline,
  with a full review and a backup first, and **Undo rewind** afterwards.
  Requires a content store (today's logs record existence, not content):
  delta-compressed, with a user-visible storage budget and oldest-first
  eviction — Rewind does not ship without the budget. Registry-and-settings
  v1 may come first; never present a partial rewind as complete; exclude
  activation/DRM locations conservatively.

**Product quality**
- **Complete, production-ready tools** — do not ship placeholders as finished
  features. Each tool must cover its end-to-end workflow, input and permission
  checks, progress and cancellation where applicable, actionable failures,
  recovery or cleanup, accessibility, localization, and documented supported
  configurations. Verify normal use and failure paths before enabling it in the
  interface; keep unfinished tools visibly marked as planned.
- **Contextual guidance throughout** — provide concise, useful, localized
  tooltips for interactive controls and settings, explaining their effect,
  prerequisites, safety impact, or limitations as appropriate. Make help
  available by keyboard focus as well as pointer hover; keep critical warnings
  visible without requiring a tooltip.
- **CLI coverage wherever practical** — extend the existing command line to
  cover every workflow that can be operated safely without the GUI. Provide
  documented noninteractive use, machine-readable output, stable exit codes,
  preflight or dry-run support where applicable, and the same backup and
  confirmation safeguards as the GUI. Document and test any justified
  GUI-only workflows rather than implying complete parity prematurely.
- **Safety and recovery first** — keep destructive work previewed, itemized,
  opt-in, and backed up; add failure-injection tests for interrupted removals,
  backup/restore, elevation handoff, and locked files. Explain uncertainty and
  access failures instead of presenting incomplete scans as clean results.
- **A trustworthy compatibility contract** — document Windows versions,
  architectures, privilege requirements, and tested installer/archive tiers.
  Validate destructive workflows on clean, snapshot-isolated Windows 11 x64
  and Arm64 machines, including upgrades and rollback.
- **Useful customization without clutter** — add clear Simple and Advanced
  presets for scan scope, exclusions, review defaults, and automation. Keep
  risky actions opt-in, preserve local-first behavior, and make each setting's
  effect visible before it changes system behavior.
- **Fast launch and responsive operation** — profile cold start and common
  workflows; load expensive data on demand, keep scans cancellable and
  backgrounded, bound concurrency and memory use, and cache only results with
  reliable invalidation. Set measurable startup, idle-resource, and scan-time
  baselines before optimizing.
- **Modern Windows usability** — verify per-monitor DPI and high-contrast
  behavior, keyboard-only navigation and focus, screen-reader names, Unicode
  and long paths, sleep/resume behavior, and consistent dark/red theming.
  Maintain accessible labels and native-speaker reviewed translations.
- **Release confidence** — expand automated regression and Windows client
  coverage, monitor crash/performance reports with privacy-preserving opt-in,
  and keep signed, reproducible release artifacts and clear upgrade/rollback
  guidance.

**Developer workflow additions**
- **Repeatable project setup** — optionally export a reviewed WinGet
  Configuration from recognized installed package IDs and explicit user
  selections, so a project can be bootstrapped on another PC. Never infer or
  apply untrusted configuration silently; WinGet configurations can execute
  machine changes and must be inspected before applying. See Microsoft's
  [WinGet Configuration guidance](https://learn.microsoft.com/en-us/windows/package-manager/configuration/).
- **Per-project environment profiles** — save and switch reversible user-level
  environment-variable and PATH overlays for development tasks, preserving
  prior values and showing the effective environment before launching a tool.
  This addresses a useful workflow also found in
  [PowerToys Environment Variables](https://learn.microsoft.com/en-us/windows/powertoys/environment-variables),
  without making every shell or system variable globally mutable.
- **Dev Drive guidance and workspace setup** — detect supported Dev Drives,
  explain their ReFS/security/performance tradeoffs, and offer a safe link to
  Windows' own creation flow rather than modifying disks behind the user's
  back. Microsoft's [Dev Drive requirements](https://learn.microsoft.com/en-us/windows/dev-drive/)
  include Windows 11 build 22621.2338+, local administrator rights, and
  recommended free-space and memory thresholds.
- **Diagnostic-tool handoff** — when useful, offer to open an already
  installed Microsoft Sysinternals diagnostic such as Process Explorer or
  Process Monitor at the relevant file/process, but do not bundle or download
  third-party executables automatically. SetupSweep's own process, lock, and
  install-monitoring views remain the primary experience. See the
  [Sysinternals tools](https://learn.microsoft.com/en-us/sysinternals/).

## Competitive fit

- **Already covered; avoid duplication:** installed-program analysis, monitored
  installs, leftovers, forced removal, and batch uninstall are workflows
  SetupSweep already has. Keep investment on reliability, review, and the
  native UI migration rather than parallel tools.
- **Window-targeted identification** is prototyped above.
  Browser-extension inventory is a separate, reversible follow-on, not a
  browser-cleaning bundle.
- **Reviewed junk and update cleanup:** extend SetupSweep's existing
  review/restore patterns for the relevant cleanup; defer Compact OS and
  secure deletion, which broaden scope and carry substantially higher
  system/data risk.

## Later

**Related tools: current coverage and gaps**
- **Available:** PortableApps Forge creates PortableApps.com Format
  packages from `.exe`/`.msi` installers, using Windows Sandbox when available
  or monitored host capture otherwise; its portability check, Launcher
  settings, and package creation are already implemented. See the
  [README](README.md#portableapps-forge).
- **Available:** System Restore manager lists, creates, deletes, and rolls
  back Windows restore points and configures System Protection and its disk
  limit. Snapshot comparison is available separately for before/after
  inspection of recorded system changes.
- **Partial, not a substitute for the planned tools:** installer preview and
  PortableApps Forge can use Windows Sandbox on supported editions. This
  does not provide an in-app disposable runner for arbitrary developer tests.
- **Not implemented:** a VSS content browser and a temporary scratch volume;
  both are planned above with read-only or bounded, recoverable behavior.

**Monitoring**
- **Per-value live registry view** — the live activity panel shows which registry areas change; listing each value as it changes would need event tracing (ETW).

**PortableApps Forge**
- **Drive-letter and path fixing** — find absolute paths inside captured settings files and write matching `[FileWrite]` sections, so settings follow the drive.
- **An isolated sandbox for Windows 11 Home** — Home PCs now have *Try it, then undo*, which installs, records and removes again; a virtual machine option (Home has no Hyper-V, so VirtualBox or QEMU) would keep them entirely untouched.

**Release readiness**
- **Code signing certificate** — the release workflow already signs `SetupSweep.exe` (x64 and Arm64) and the installer, with a timestamp, as soon as the `CODESIGN_PFX` and `CODESIGN_PASSWORD` repository secrets are set. What's left is getting a certificate: a free open-source one (SignPath Foundation), Azure Artifact Signing, or an OV certificate.
- **Antivirus false-positive reports** — the release workflow already uploads every download to VirusTotal when the `VIRUSTOTAL_API_KEY` secret is set, warns about any detection and links each report in the release notes. What's left is a free VirusTotal account for the key, and submitting any false positives to Microsoft Defender, Malwarebytes and other vendors before publishing.
- **Windows 11 client validation** — the integration matrix (Windows Installer, NSIS, Inno Setup, Squirrel, WiX Burn, and InstallShield and Wise once the `INSTALLSHIELD_URL` and `WISE_URL` repository variables point to test installers) runs on Windows Server runners; run it on a freshly provisioned, snapshot-isolated Windows 11 virtual machine as well, and add a large MSI suite.
- **Multi-day soak run** — `scripts/soak.ps1` is ready and runs for five hours every week in CI; run it for several days on a test VM before the first release.

**Everything else**
- **More integration tests** — run on Windows 11 client images when GitHub offers them.
- More languages and native-speaker reviews of the existing ones — see [docs/TRANSLATING.md](docs/TRANSLATING.md).

## Considered and not planned
- **Kernel minifilter driver for monitoring** — a driver must be signed by Microsoft, which open-source projects can't easily get, and a driver bug can crash Windows. Snapshots plus Windows' change notifications capture the same changes safely.
- **Rewrite in C#/.NET with WinUI 3** — it would add a runtime dependency and give up the single self-contained exe. The Rust engine is already native and fast.
- **Always require administrator rights in the manifest** — SetupSweep starts as the current user and offers a one-click relaunch as administrator, because much of it (browsing programs, junk cleaning in your profile, the Ask, Snapshots and Right-click menu pages for your account) needs no elevation, and the command line must run in unelevated scripts. Actions that need administrator rights say so and relaunch elevated.
- **Self-contained .NET deployment, trimming and ReadyToRun** — not applicable: SetupSweep is a native Rust program with no runtime to bundle, already built as separate x64 and Arm64 executables with link-time optimization.
- **SQLite index** — the program list is already filtered instantly in memory; the on-disk cache planned above covers the slow parts (sizes, icons, signatures) without a database.

## Known limitations
- Contents of files the installer *modified* or *deleted* can't be restored (only their existence is recorded); SetupSweep lists them so you know.
- A restored scheduled task file is not re-registered with Task Scheduler until it is re-imported.
- Snapshots cover the current user's profile; changes to other users' profiles aren't captured.
