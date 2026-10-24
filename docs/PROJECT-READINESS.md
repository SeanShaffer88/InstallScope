# Project Readiness and Workflow Board

The compact Workflow Board opens five independent tools. Project Readiness combines bounded diagnostics with separately reviewed dependencies; selecting a tool never starts operations.

Desktop declarations include CMakePresets.json/CMakeUserPresets.json, .vsconfig, vcpkg.json, Directory.Build.props and root-level .vcxproj/.csproj/.fsproj/.vbproj files. CMakeLists.txt presence identifies a CMake project without script evaluation. Discovery stops at 128 root entries, never recurses and shares the 256 KiB/file and 2 MiB read budget. XML is UTF-8, at most 64 elements deep; DTDs/entity-based property values are rejected. Up to 64 XML declarations/components or 32 presets per collection are reported. Imports/includes, conditions, macros, inheritance and effective resolver selection remain unverified. The checklist links to Dependencies and separately reviewed Build Troubleshooting; diagnostics never automatically build/install anything.

## Setup and troubleshooting

1. Open Project Readiness and select an existing absolute local project folder.
2. Choose the intended x64 or ARM64 build target. Choose a fallback prerequisite profile for projects without a recognized stack. Project Doctor chooses one profile from supported declarations; mixed stacks can need separate additional scans.
3. Optionally attach one saved build log. The user associates this file with the project; SetupScope does not prove its origin, target, freshness or build outcome.
4. Select Inspect Project Readiness. The existing bounded Project Doctor inspection reads supported declarations and gathers prerequisite, PATH and version evidence. Optional log triage then uses the existing fixed offline rules. No project scripts, builds, package-manager commands, installations, repairs, environment changes or uploads run.
5. Review incomplete evidence, declaration/check counts, original observation time and optional hint groups/source line numbers. No overall ready, compatible or successful-build verdict is issued.
6. Follow Guided Next Steps. Open This Evidence reuses observations in Requirements, Toolchain / PATH or saved-log triage without rescanning. A running diagnostic is never replaced. Dependencies, Environment Profiles and Ports And Processes require their own explicit inputs/review. Prepare Build / Test passes only the project folder into Build Troubleshooting; choose and review executable/arguments before execution.

A possible linker/access failure offers file-owner investigation; it does not assert a lock. A possible storage failure offers storage inspection; it does not identify a full volume. Guidance does not execute suggested fixes or install missing packages.

## Freshness, failures and cancellation

Reports remain in memory for the application session. Changing the project, target, fallback profile or selected log invalidates linked handoff until another inspection finishes. Unchanged inputs can also become stale when files, SDKs, environment or IDE state changes; reinspect after changes.

Failed workflow refreshes retain previous evidence with an explicit warning and disable handoff. A failed optional log read preserves the new project evidence and marks the workflow incomplete; it does not import a stale log report. Cancellation is cooperative at existing checkpoints; operating-system reads must return first. Cancelled evidence cannot be handed off as a current observation. Closing the tool preserves its session and pending work; reopening returns to it. The main board polls completion even when the tool window is closed.

## Scope and privacy

Existing limits remain: Project Doctor reads at most 256 KiB per supported UTF-8 manifest, 2 MiB in total, and relevant declarations in up to 32 ancestors. SDK/tool discovery remains bounded. Logs are one regular local file, at most 2 MiB, 20,000 lines and 8 KiB per line, with supported UTF-8/BOM UTF-16 decoding. Oversized logs are rejected; skipped/partial evidence stays explicit. No recursive repository scan is added.

Copy Readiness Summary is an explicit clipboard action. It contains project/log paths, dates, bounded counts and fixed guidance, so review it before sharing. It excludes raw log lines, environment values and manifest values. Detailed evidence remains available in its individual diagnostic tools. Nothing is saved or uploaded automatically; these summaries are not automatically added to redacted diagnostic bundles.

## Validation boundaries

Automated coverage includes non-execution/non-write behavior, raw-log exclusion, missing log failure preservation, cancellation/input validation, accessible explicit controls, keyboard scan requests, stale-input/failed-refresh invalidation and refusal to replace busy diagnostic evidence. Windows visual checks, packaged executable smoke and suite results are recorded in LOCAL-STATUS.txt. Full screen-reader/DPI, broad OS/architecture and vendor recovery acceptance remain separate release gates.

The Workflow Board fills the dashboard without a left sidebar: six cards in three columns and two rows (the five tools plus the read-only System And OS card), with Settings at the far right of the header. There is no card search. Each card opens its tool in its own window, and the tool's top-right Workflow Board button restores the board without closing any tool or stopping its work.
