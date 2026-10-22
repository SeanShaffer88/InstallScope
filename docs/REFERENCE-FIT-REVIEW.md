# Saved Windows references: SetupScope fit review

2026-10-22 expansion: consulted shared useful-dev-resources AGENTS.md and Windows build/build-package-environment categories; fetched origin/main at 765177c. Existing Rust/windows-sys remain the stack. Win32 Job Objects provide child lifetime control, not a sandbox. Reused already locked quick-xml 0.41.0 directly for bounded XML declarations (MIT, Rust minimum 1.79 per crate metadata); no DTD/external loading. Its notice is collected in release metadata. Microsoft's official component/layout documentation supplies setup IDs and limits; see DEPENDENCY-MANAGER.md. No catalog was installed wholesale, revival work or new resource publication was needed. This is a scoped review, not a vulnerability certification.

Diagnostic bundles reuse the project's existing JSON writer and durable no-replace
export code. Saved Rust/tooling references were consulted; no library, archive
engine, account or service was needed. Redaction is a strict data allowlist rather
than a dependency claiming to detect every possible secret.

The WSL/container readiness milestone reuses windows-sys service/status and processor APIs plus guarded local file metadata. No engine SDK, IPC revival or external runtime is needed. Official API guidance was checked; no new dependency was adopted.

## Current reference map: 232 Windows repositories and 361 broader resources

Relevant Windows/native API and platform SDK sections of both local catalogs were
consulted for the ports/file-owner workflow. Existing windows-sys 0.59 bindings
and Windows' built-in IP Helper/Restart Manager APIs fit; only existing-crate
feature flags were enabled. No new crate/runtime/driver or catalog project was
installed, cloned or bundled. This is not a new audit of every catalog entry;
the two catalogs overlap and their counts must not be summed as unique projects.

The shared revival backlog has WR-001 through WR-010. WR-007 is now complete for Windows desktop x86/x64: win-pipe-revival 0.2.0, commit 2e9523e28dee527b03773327338ee2519c5c38c9. That work was not consumed or modified here,
and no shared task status changed in this SetupScope milestone. The earlier
229-entry/WR-001..WR-006 notes below are historical.

Finished-product publication is authorized when appropriate after validation.
SetupScope's current 0.1.0 package is a local development milestone with open
release gates, not a completed release or newly published remote repository.

## Catalog update: 229 repositories

The later 31-entry revival/systems expansion was consulted while implementing
project environment profiles. The initial six-candidate review below concerned
the earlier 198-entry catalog; it is not a full audit of all 229 entries.

New catalog leads relevant to the accepted release/build diagnostics roadmap are
BinSkim (binary security-rule evidence), SizeBench (binary/PDB size regressions)
and vcperf / Build Insights samples (explicit build-performance investigation).
These are discovery leads from the saved catalog, not integrations or freshly
verified maintenance/license/architecture claims. Recheck exact upstream/pins and
assess explicit external-tool handoff versus embedded implementation before use.

Environment profiles need none of the new runtimes, IPC libraries or forks;
existing Rust/windows-sys APIs and the existing durable writer fit this workflow.
No expansion repository was installed, cloned, bundled or executed. Revival
remains eligible where a demonstrated gap justifies scoped ownership; no unique
need to revive a project was established for this milestone.

The shared windows-development-todo.md was also consulted. WR-001 through WR-006
remain queued; no task was selected or changed, so neither shared copy required
a status update. In particular, conditional SharpDX work has no consuming
SharpDX application in this Rust/windows-sys project.

Reviewed 2026-10-21 against the saved 198-repository catalog. Relevant API,
diagnostics, PE/PDB, build, packaging and testing sections were consulted; this
is not a build/security audit of all 198 repositories. The catalog is a source
of candidates. No catalog project was cloned, installed or bundled by this review.

## Selected improvements

| Priority | Improvement | Candidate/reference | Decision and acceptance gate |
|---|---|---|---|
| 1 | Discover custom Visual Studio/Build Tools instances, workloads and SDK evidence | [vswhere](https://github.com/microsoft/vswhere) | Improve Project Doctor's standard-folder-only coverage using documented Setup discovery. Keep the default inspection free of external command execution; any installed-tool handoff must be separately explicit. Test custom paths, multiple/incomplete instances and x64/ARM64. |
| 2 | Read-only DLL/import/architecture inspector for build failures and packaging prerequisites | [LIEF](https://github.com/lief-project/LIEF), [pe-parse](https://github.com/trailofbits/pe-parse) | Compare a Rust-compatible integration and native build footprint before adoption. Parse files without loading target DLLs. Bound imports/sections/recursion, handle delay imports/API sets and distinguish candidate resolution from the Windows loader's actual result. Use hostile/truncated fixtures and architecture-specific tests. |
| 3 | Broader installer/archive inspection and reviewed extraction | [libarchive](https://github.com/libarchive/libarchive) | Evaluate pinned feature-limited integration. Preserve path/link/reparse protections, output quotas and explicit support tiers; do not claim every format. Test malformed files, Unicode/long paths, cancellation, unsupported encryption and x64/ARM64 packaging. |
| 4 | Developer package/release preflight for MSIX/AppX | [MSIX SDK](https://github.com/microsoft/msix-packaging) | Use its documented package API contracts as a reference and evaluate built-in Windows packaging APIs before adding a DLL. Validate manifests/block maps/architecture/signature evidence without installing a package. Test bundles, invalid signatures and unsupported versions. |
| 5 | Repeatable keyboard/accessibility acceptance in an isolated test environment | [FlaUI](https://github.com/FlaUI/FlaUI) | Candidate development-only UIA harness, not a shipped .NET dependency. Validate AccessKit and native Win32 behavior separately on supported architectures/DPI. This does not replace screen-reader checks or authorize bypassing a denied automation action. No UI automation was performed by this review. |

These strengthen the accepted installer regression, dependency/provenance,
Project Doctor and local release-preflight work. They remain development tasks;
the reference review alone does not implement or certify them. Keep the existing
Rust/windows-sys product architecture, local processing and explicit review.

## Current upstream evidence

Public repository metadata and README content were read through the connected
GitHub read-only tools. A last-push date is an activity signal, not a maintenance,
security, license or compatibility guarantee.

| Repository | Archived | Last push UTC | Repository license indicator | Integration evidence / open checks |
|---|---|---|---|---|
| lief-project/LIEF | No | 2026-10-18 | Apache-2.0 | README documents PE and Rust APIs. Validate the chosen Rust/native artifact provenance, footprint, feature licensing and Windows x64/ARM64 build/runtime before adoption; no extended/commercial features are assumed. |
| trailofbits/pe-parse | No | 2026-08-05 | MIT | README documents bounded-buffer PE analysis, imports/exports and Windows/MSVC builds. It is C++; Rust FFI/build ownership and ARM64 behavior still need proof. |
| libarchive/libarchive | No | 2026-10-21 | API reports NOASSERTION | Actual COPYING was inspected: a BSD-style base notice plus file-specific exceptions and separately licensed build files. Review selected source/compression dependencies and preserve applicable notices; NOASSERTION is not evidence of no license. |
| microsoft/vswhere | No | 2026-10-01 | MIT | README documents multiple-instance discovery, standalone executable and inclusion with VS Installer from 15.2. This review neither executes nor redistributes it. |
| FlaUI/FlaUI | No | 2026-08-13 | MIT | README documents UIA2/UIA3 and Win32/.NET application automation. Test harness/runtime/architecture compatibility and UIA backend choice require validation. |
| microsoft/msix-packaging | No | 2026-08-24 | MIT | README describes a cross-platform DLL/API subset of Windows appxpackaging.dll. SDK/native runtime/toolchain and architecture-specific requirements remain unverified here. |

For a future adoption, inspect exact pinned source/license files and transitive
dependencies, build from a controlled source, test relevant Windows/CPU targets,
and retain provenance/notices. Current Project Doctor reuses already-locked Rust
parsers; it has not adopted any of the six candidates above.

## Maintenance and revival

Inactive/archived projects remain eligible when a unique benefit justifies the
license/fork-rights, repair, security, compatibility and long-term ownership work.
Compare an existing maintained fork and alternatives, then scope a reproducible
feasibility test. No revival was selected in this review: the immediate gaps have
existing API and candidate paths to evaluate, so there is no demonstrated need
to take ownership of a fork yet.


## Developer dashboard, scratch storage and migration

Consulted the saved Windows catalog's storage/virtual-filesystem references and the general catalog's Windows SDK/build-tool sections. This milestone uses existing Rust filesystem operations and the existing transfer engine; it does not adopt a filesystem driver, new runtime or revival dependency. Ordinary scratch folders are deliberately distinct from the planned capacity-bounded VHDX implementation.

Existing eframe 0.33.3 is locally patched for child-window AccessKit initialization after a live UIA failure. The patch retains upstream source, pinned provenance and MIT/Apache-2.0 licenses. See GUI-DEPENDENCY-PATCH.md and vendor/eframe-0.33.3/SETUPSCOPE-PATCH.md. No dependency-family rewrite or upstream publication occurred.


## Project Readiness integration

Consulted the shared useful-dev-resources AGENTS.md, category index and Windows diagnostics category, alongside the supporting Windows/general catalogs. Existing bounded Project Doctor, developer prerequisite/PATH checks and fixed build-log triage fit the requested composition; no new library, SDK, runtime, command runner or service was adopted. No new external resource was discovered or catalog change required. Existing dependencies and Windows integration remain unchanged.
