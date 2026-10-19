# Roadmap

Planned for future releases, roughly in priority order. Vote or comment on the
matching GitHub issue to move something up. Features that already exist are
listed in the [README](README.md#features).

## Next

The core 1.0 feature set is substantially implemented. The next priorities are
the remaining product goals and a staged UI transition; each should ship as a
bounded, tested workflow rather than a broad rewrite.

**Core product**
- **Native Win32 interface** — replace `egui`/`eframe`/`wgpu` in phases while
  preserving the established dark, black, and red design. Migrate and verify
  complete workflows one at a time; keep the existing working UI until each
  replacement is ready. The experimental Programs preview now includes
  browsing, search, details, window-targeted identification, desktop and
  Store-app uninstalls, and reviewed leftover cleanup. Manual-entry removal,
  batch actions, restore points, and full-flow parity remain.
- **Window-targeted program identification** — a Revo Hunter Mode-inspired
  picker is prototyped in the native preview. Validate it in the full
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
  into a guided, reviewable project only where it adds value: package identity,
  destination mapping, shortcuts, prerequisites, install scope, and a clear
  uninstall/rollback plan. Keep silent redeployment as the fast path; do not
  imply arbitrary installers can be safely reconstructed from incomplete logs.
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

**Product quality**
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
  third-party executables automatically. Expunge's own process, lock, and
  install-monitoring views remain the primary experience. See the
  [Sysinternals tools](https://learn.microsoft.com/en-us/sysinternals/).

## Competitive fit

- **Already covered; avoid duplication:** Total Uninstall and Revo emphasize
  installed-program analysis, monitored installs, leftovers, forced removal,
  and batch uninstall—workflows Expunge already has. Keep investment on
  reliability, review, and the native UI migration rather than parallel tools.
- **Distinct Revo feature:** window-targeted identification is prototyped above.
  Browser-extension inventory is a separate, reversible follow-on, not a
  browser-cleaning bundle.
- **Auslogics Windows Slimmer:** its official
  [feature page](https://www.auslogics.com/en/software/windows-slimmer/)
  describes reviewed junk and update cleanup, Rescue Center restore, and
  Compact OS. Prefer extending Expunge's existing review/restore patterns for
  relevant cleanup; defer Compact OS and secure deletion, which broaden scope
  and carry substantially higher system/data risk.

## Later

**Related tools: current coverage and gaps**
- **Available:** Portable App Builder creates PortableApps.com Format
  packages from `.exe`/`.msi` installers, using Windows Sandbox when available
  or monitored host capture otherwise; its portability check, Launcher
  settings, and package creation are already implemented. See the
  [README](README.md#portable-app-builder).
- **Available:** System Restore manager lists, creates, deletes, and rolls
  back Windows restore points and configures System Protection and its disk
  limit. Snapshot comparison is available separately for before/after
  inspection of recorded system changes.
- **Partial, not a substitute for the planned tools:** installer preview and
  Portable App Builder can use Windows Sandbox on supported editions. This
  does not provide an in-app disposable runner for arbitrary developer tests.
- **Not implemented:** a VSS content browser and a temporary scratch volume;
  both are planned above with read-only or bounded, recoverable behavior.

**Monitoring**
- **Per-value live registry view** — the live activity panel shows which registry areas change; listing each value as it changes would need event tracing (ETW).

**Portable App Builder**
- **Drive-letter and path fixing** — find absolute paths inside captured settings files and write matching `[FileWrite]` sections, so settings follow the drive.
- **An isolated sandbox for Windows 11 Home** — Home PCs now have *Try it, then undo*, which installs, records and removes again; a virtual machine option (Home has no Hyper-V, so VirtualBox or QEMU) would keep them entirely untouched.

**Release readiness**
- **Code signing certificate** — the release workflow already signs `Expunge.exe` (x64 and Arm64) and the installer, with a timestamp, as soon as the `CODESIGN_PFX` and `CODESIGN_PASSWORD` repository secrets are set. What's left is getting a certificate: a free open-source one (SignPath Foundation), Azure Artifact Signing, or an OV certificate.
- **Antivirus false-positive reports** — the release workflow already uploads every download to VirusTotal when the `VIRUSTOTAL_API_KEY` secret is set, warns about any detection and links each report in the release notes. What's left is a free VirusTotal account for the key, and submitting any false positives to Microsoft Defender, Malwarebytes and other vendors before publishing.
- **Windows 11 client validation** — the integration matrix (Windows Installer, NSIS, Inno Setup, Squirrel, WiX Burn, and InstallShield and Wise once the `INSTALLSHIELD_URL` and `WISE_URL` repository variables point to test installers) runs on Windows Server runners; run it on a freshly provisioned, snapshot-isolated Windows 11 virtual machine as well, and add a large MSI suite.
- **Multi-day soak run** — `scripts/soak.ps1` is ready and runs for five hours every week in CI; run it for several days on a test VM before the first release.

**Everything else**
- **More integration tests** — run on Windows 11 client images when GitHub offers them.
- More languages and native-speaker reviews of the existing ones — see [docs/TRANSLATING.md](docs/TRANSLATING.md).

## Considered and not planned
- **Kernel minifilter driver for monitoring** — a driver must be signed by Microsoft, which open-source projects can't easily get, and a driver bug can crash Windows. Snapshots plus Windows' change notifications capture the same changes safely.
- **Rewrite in C#/.NET with WinUI 3** — it would add a runtime dependency and give up the single self-contained exe. The Rust engine is already native and fast.
- **Always require administrator rights in the manifest** — Expunge starts as the current user and offers a one-click relaunch as administrator, because much of it (browsing programs, junk cleaning in your profile, the Ask, Snapshots and Right-click menu pages for your account) needs no elevation, and the command line must run in unelevated scripts. Actions that need administrator rights say so and relaunch elevated.
- **Self-contained .NET deployment, trimming and ReadyToRun** — not applicable: Expunge is a native Rust program with no runtime to bundle, already built as separate x64 and Arm64 executables with link-time optimization.
- **SQLite index** — the program list is already filtered instantly in memory; the on-disk cache planned above covers the slow parts (sizes, icons, signatures) without a database.

## Known limitations
- Contents of files the installer *modified* or *deleted* can't be restored (only their existence is recorded); Expunge lists them so you know.
- A restored scheduled task file is not re-registered with Task Scheduler until it is re-imported.
- Snapshots cover the current user's profile; changes to other users' profiles aren't captured.
