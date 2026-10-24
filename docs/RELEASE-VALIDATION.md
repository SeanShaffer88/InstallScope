# SetupScope 1.0.0.0 Local Release Package

> **Evidence scope (annotated 2026-10-24).** Everything below was recorded for the
> 1.0.0.0 package built from revision `5d6268b`. Two commits landed afterwards:
> PR #59 rewrote the README, and PR #60 changed failure presentation and added
> regression coverage. Neither re-ran the packaging, installer or isolated-host
> checks, so these results stay attached to that revision and are not a
> re-verification of the current tree.

## Scope and environment

The packages target Windows 11 x64 and the five Workflow Board tools plus their integrated workflows. The recorded live host is Windows build 26300, x64. Cargo semantic version is 1.0.0; Windows product/file version is 1.0.0.0. Both portable ZIP and per-user installer are unsigned. ARM64 runtime, Windows 10 and other Windows builds are not certified. Local packaging does not itself publish a GitHub release.

## Verified

- 485 core tests passed; six ignored entries comprise five subprocess fixtures exercised by runner tests and one existing explicit ignore. 43 GUI-entry tests, 3 console-entry tests and 3 CLI integration tests passed, including alternating tool-header clipping regression coverage.
- A real isolated offline Rust build and one project test passed through the reviewed runner; both captured logs were readable by triage. No real vendor installation was performed.
- Clippy completed with existing warnings; optimized Windows x64 build completed.
- Test cases include malformed/truncated inputs, bounded observations, persistence failure paths, guarded recovery, dependency digest vectors/chunk boundaries, unsafe manifests/URLs, held installer files, cancellation and missing review/signature/receipt launch guards.
- Mock installer outcomes distinguish success, reboot requests, cancellation, failed launch and uncertain tracking. No real vendor installation was performed on this PC.
- All six pinned package hashes matched official metadata on 2026-10-22. Real x64 CMake and .NET SDK files passed download/hash/certificate-organization verification, offline export/import and review preparation without execution.
- Live dependency UI verified the cached SDK, exposed its exact review and kept launch disabled without acknowledgment. Five-tool and settings windows expose SetupScope titles and the scoped logo.
- Product metadata, source content, title/icon/banner assets and executable strings were checked for consistent SetupScope branding.
- Existing portable data was retained with file-hash equality; installed-mode data was retained. UI checks made no persistent environment edit, vendor installation or diagnostic upload. Runner tests terminate their isolated fixture process trees; one frozen, data-free QA app was stopped while fixing hidden-parent redraws. Unrelated user processes were not terminated.
- Live checks cover board hiding, continued tool responsiveness/accessibility updates, returning without closing two open tools, content-driven growth and compact initial windows. Windows/WGPU hidden-parent repaint handling is included in the documented local eframe patch.

Current artifact hashes are in SHA256SUMS.txt; packaged console checks are in LOCAL-VALIDATION.json. The localization audit covers all ten owned-interface catalogs: 600 translated strings and 58 technical invariants each. Placeholder/invariant checks pass. Independent native-speaker review is not certified.

## Environment-limited or unverified

Windows Sandbox is unavailable on the current PC. Startup with fresh portable data and a stripped developer PATH is the strongest available local packaging check; it is not a clean Windows VM. A clean-machine run remains a verification limit, not a passed check.

Actual screen-reader behavior, a broad DPI/Windows matrix, ARM64 runtime and real vendor installation/UAC/uninstall/reboot remain unverified. Accessible tree/keyboard/scaling checks are narrower evidence. No signing certificate was used. Do not interpret the version number, test counts or successful build as certification of these scenarios.

On 2026-10-22 the user requested final local 1.0.0.0 packaging with VM checks skipped and testing limits documented. This package records completed local checks and retains every unverified platform, installation and accessibility limit; it is not clean-machine or universal compatibility certification.

## Packaging and licensing

The C runtime is statically linked. The package contains the executable, portable marker, current docs/assets, SHA-256 checksums, collected upstream notices and a CycloneDX inventory generated from Cargo's resolved Windows x64 graph. No test data, developer installer payloads or saved user data is included in a distribution archive. Graph inventory includes build/dev entries and is not a binary reachability/vulnerability certification.

The vendored eframe patch and upstream MIT/Apache notices are retained. Supplemental notices come from exact crate source commits where packaged crates omitted workspace license files. Standard CC0 text for hexf-parse follows its Cargo license declaration and is identified as SPDX-sourced. Font license text is collected from the packaged font directory. PROVENANCE.json files preserve source URLs; upstream attributions are not rewritten.

Only portable packaging is supported in this scope. Local package removal leaves separately installed developer dependencies unchanged; retain saved SetupScopeData or dependency bundles when needed. Automatic installer, online update, repair and publishing flows are not part of this release.

## Latest console, grid and language changes

Headless layout tests cover six cards in three columns/two rows at 740/960
logical pixels, five singleton tools and far-right return buttons. CLI black-box
tests verify console exit codes/JSON, the six-package catalog, no raw-secret echo
or file mutation during triage, and rejection of piped execution confirmation
before any log/process is created. Production vendor installation and persistent
user environment changes were not exercised by these new tests.

Language checks require all 658 owned inventory entries in each embedded catalog,
including tooltips and safety guidance. Exact placeholder occurrences and the 58
technical invariants pass. Original evidence, commands/arguments, vendor reviews,
JSON and CLI help retain their wording. Isolated subprocesses render all eleven
language choices at preferred/minimum tool widths with configuration sections
expanded. Long headings wrap; diagnostic previews grow independently of the old
compact viewport. Live German/Chinese/Japanese boards and German/Chinese/Korean/
Japanese settings showed readable text and CJK system-font fallback. Saved Japanese
selection was verified in isolated settings. These checks do not certify native
linguistic quality, every font installation or actual screen-reader operation.

Packaged console verification passed 18 functional checks, including passive readiness over an untrusted synthetic build script, literal child-argument preservation, JSON errors and all eleven language choices. Saved portable data hashes were unchanged. Both PE subsystem types (GUI and console) and SetupScope branding were checked. No vendor installer or persistent environment change was executed.

## Desktop pass before final wording review

The earlier local release build (GUI SHA256 8a89490fb9bc0851df8434f0f0528f64bdc2554dd4c7e51fee8ed75ae03d9399) was used for all five tool workflows and the refreshed focused screenshots. The final independent review changed catalog wording; the revised build passed the full tests and isolated multilingual layout checks. A fresh full interactive pass after those wording changes was not completed because Computer Use was stopped. Passive ports/host checks, synthetic project declaration handoff, immutable redacted JSON preview, build-review preparation/discard and synthetic saved-log triage were exercised. The build remained disabled without acknowledgment; no code was executed through UI automation. Five tools stayed open while the board was restored. A second GUI launch reused the existing process/board. Reopening a tool reused its window and retained evidence. Screenshot examples include explicit synthetic files; no vendor installer was launched.

One idle sample with five tools open recorded 248684544 working-set bytes, 614203392 private bytes and 0.0625 CPU seconds over 2.004 seconds. This is one local sample, not a benchmark or performance guarantee.

Independent second-agent review on 2026-10-22 checked all 6,000 owned translations, corrected 80 wording issues, and preserved placeholders/technical identifiers. See LOCALIZATION-REVIEW.json for exact changes. This does not substitute for human native-speaker review.

## Independent review and clean-host follow-up

A second set of agents reviewed all 6,000 owned translations and corrected 80 wording issues. Exact changes are in LOCALIZATION-REVIEW.json; human native-speaker certification remains unavailable. Catalog integrity and expanded compact layout tests were rerun after these changes.

The PowerShell 5.1 clean-host harness passed 12 consistency/loader/CLI/diagnostic checks with fresh portable data, System32-only child PATH and isolated profile/AppData. An empty inherited PSModulePath exposed a built-in command-loading issue; the harness now explicitly imports the executing runtime’s own Microsoft modules. Valid-package tests pass with empty PSModulePath, and tampered executable checksums are rejected before execution. This is still the current host OS/account, not a clean Windows guest. No VM runtime or guest image was available in the checked locations. See CLEAN-MACHINE.md for the reproducible clean-PC procedure.

## Final local package

Local 1.0.0.0 packaging was requested with VM checks skipped. The current package passed 18 packaged functional checks and 12 isolated-host checks; saved portable user data remained byte-identical. Direct PE imports reference Windows DLLs/contracts, with no direct VC/.NET/Python/Node runtime imports (PE-DEPENDENCY-CHECK.json). This static check does not establish transitive driver or clean-OS compatibility. LOCAL-HOST-CHECK.json records the exact tested executable hashes. Existing compiler/Clippy warnings remain documented by the local check logs; no clean-machine, human native-speaker, broad DPI or screen-reader certification is claimed.

## Publication follow-up

GitHub Linux CI exposed a test fixture using a Windows-only absolute path; the fixture now uses the host temporary directory. Windows CI exposed the 200 ms durable-write lock wait expiring under disk contention. The wait is now bounded at ten seconds while keeping atomic publication and conflict handling. The updated binaries passed the full 534 local tests, Clippy, release build and the 12 isolated-host checks again. Earlier interactive screenshots predate this persistence-only correction; no new interactive GUI certification is claimed.

The per-user NSIS installer uses the verified portable binaries, omits the portable marker, registers Windows Installed Apps, and offers optional shortcuts. It rejects a running GUI, portable destinations and unrelated nonempty folders. Uninstall uses explicit file lists and preserves user data and unknown files. Silent install, same-version upgrade, loader, running-app guard and uninstall were checked on the current host. The reproducible `scripts/Test-Installer.ps1` also verifies installed checksums and destination guards. The release's installer-validation JSON records the exact final installer digest and check results. Clean-VM, interactive installer-page, reboot and signed-package acceptance remain unverified.
