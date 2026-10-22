# Developer-toolkit expansion proposal (post-1.0)

Status: **proposal for selection — nothing here is implemented or authorized.**
Per AGENTS.md, items that add tools, network use, privileges or persistence need
an explicit choice before work starts. Effort is rough engineering time
including Windows CI tests; it excludes clean-VM and assistive-technology
acceptance, which need a Windows test machine.

Goal: cover the whole Windows desktop software lifecycle (set up → build →
debug → make accessible → package → sign → ship) with the same reviewed,
evidence-first, least-privilege design as 1.0. Today SetupScope is strongest at
*set up* and *build*; the largest gaps are **after a successful build**.

## Tier 1 — highest value, fits current tools (no new board card)

| # | Addition | Where | Why it matters | Effort |
|---|---|---|---|---|
| 1 | **Built-artifact inspector**: PE architecture/subsystem, embedded manifest (DPI awareness, `longPathAware`, UAC level, supportedOS), imported DLLs flagged as *not on a clean Windows 11* (VC++ runtime, debug CRT, UCRT debug), Authenticode status | Project Readiness | The most common "works on my machine" desktop release failure. SetupScope already has PE and Authenticode code. | 1–1.5 wk |
| 2 | **Locked-output fix path**: when triage sees `LNK1104`/`MSB3027`/`error: failed to remove`, offer the existing read-only File Owners view on the exact file | Build Troubleshooting ↔ Ports And Processes | Turns a triage finding into the answer in one step; reuses existing code. | 2–3 d |
| 3 | **Expanded triage knowledge base**: NETSDK, NU1xxx, MSB4xxx, CMake configure, vcpkg, WinUI/XAML compiler (WMC), MIDL, C++ module and link errors; each with a fix hint and a doc link | Build Troubleshooting | Higher hit rate on real logs; pure data plus tests. | 1 wk |
| 4 | **Build log comparison**: diff two saved runs (new/resolved findings, duration, exit code) | Build Troubleshooting | "What changed since it last built?" | 3–4 d |
| 5 | **Typed JSON + SARIF CLI output** for readiness and triage, with a GitHub Actions annotation mode | CLI | Makes SetupScope usable as a CI gate; already listed as future work in CLI.md. | 1 wk |
| 6 | **Desktop framework detection**: WinUI 3/Windows App SDK, WPF, WinForms, Win32/MFC, Rust (windows-rs/egui), Electron/Tauri; checks required SDK/workload versions | Project Readiness | Readiness answers become framework-specific instead of generic. | 1–1.5 wk |

## Tier 2 — new capability; needs your authorization (scope, privileges or a new card)

| # | Addition | Decision needed | Effort |
|---|---|---|---|
| 7 | **Clean-machine test via Windows Sandbox**: generate a reviewed `.wsb` that maps the built output read-only, with networking off by default, then launch it | Launching Sandbox is a new changing action. It requires an optional Windows feature (Pro/Enterprise). | 1 wk |
| 8 | **Accessibility checker for your app**: inspect a running app's UI Automation tree (missing names, unfocusable controls, keyboard traps, tiny targets) and export a report | Read-only, but inspects another process's UI. Likely a new sixth tool, so the board layout changes. | 2–3 wk |
| 9 | **Packaging and signing readiness**: detect MSIX/WiX/NSIS/Inno projects, lint MSIX manifests, check signing-certificate availability and timestamping configuration (never handles private keys) | Read-only, but a new lifecycle stage. Release & Ship card? | 2 wk |
| 10 | **Crash evidence**: read Application Error/WER events and existing LocalDumps configuration for a selected executable, with a reviewed per-user dump-folder suggestion | Reads event logs. Enabling dumps writes HKLM and needs admin, so it would be guidance only unless you authorize it. | 1–1.5 wk |
| 11 | **More pinned dependencies**: Windows App SDK runtime, Git, Node LTS, Python, Ninja, WiX, vcpkg bootstrap | Each needs a verified source, integrity data, license review and recovery behavior per catalog policy. | 2–3 d each |
| 12 | **Dev Drive / Defender performance advisor** (read-only): detect Dev Drive, ReFS and Defender performance mode, and estimate build-folder exposure | Read-only. Recommendation wording must avoid weakening security. | 3–4 d |

## Tier 3 — distribution and trust of SetupScope itself

| # | Addition | Note |
|---|---|---|
| 13 | **Signed releases** (for example SignPath Foundation for OSS, or Azure Trusted Signing) | Removes SmartScreen friction. Needs an account and possibly cost; your decision. |
| 14 | **CI-built release artifacts** from the tag (ZIP, installer, SBOM, checksums, provenance attestation) | Today's 1.0.0.0 binaries were built locally. CI builds would make artifacts traceable to reviewed source. ~3–4 d. |
| 15 | **winget manifest** | Discoverability. Better done after signing. |
| 16 | **Native ARM64 release** and **Windows 10 22H2** support | ARM64 already compiles in CI, but runtime is unverified. Windows 10 is blocked at startup today. Both need test hardware. |
| 17 | **Explicit, opt-in update check** (shows the newer release; never auto-installs) | First product network call outside dependency downloads. Needs your approval. |

## Recommended sequence

1. **Ship 1.0.0.1** from `claude/release-1.0-hardening` (CLI exit-code fix), ideally with #14 so the patch release is CI-built.
2. Tier 1 in order **1 → 2 → 3 → 6 → 5 → 4**. Each is independent, testable in CI, and adds no new privilege or card. This is about 5–6 weeks for one developer, a credible **1.1**.
3. Then pick from Tier 2. **#7 (Sandbox)** and **#8 (accessibility checker)** are the strongest differentiators, and #7 also closes SetupScope's own clean-machine testing gap.
4. Run Tier 3 signing (#13) in parallel whenever you're ready to take on the account and cost.

## Deliberately excluded

AI code generation, cloud accounts, telemetry, automatic repairs and
general PC-maintenance features. These conflict with the product's
explicit, local and least-privilege principles, and AGENTS.md retired them.
