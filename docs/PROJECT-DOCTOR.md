# Project Doctor

Available in the regular interface under Developer diagnostics. Expand Project
Doctor, choose an ordinary absolute project folder, select a target and click
Inspect project. Choosing a folder alone never starts work. Results stay in memory;
Copy project report is explicit and includes local paths. It is not a redacted
diagnostic bundle. Inspection never executes scripts/tools/builds/installers,
downloads packages, repairs files or changes the environment.

## Current support

- Read the nearest global.json and Rust toolchain file in up to 32 ancestors.
  Rust's legacy file wins over rust-toolchain.toml in the same folder. Errors
  stop ancestor search rather than substituting an older declaration.
- Read Cargo.toml, package.json, pyproject.toml, .nvmrc, .node-version and
  .python-version only in the chosen folder. No recursive dependency scan.
- UTF-8 inputs: 256 KiB/file, 2 MiB total. Reject linked/reparse paths. Dynamic,
  inherited, malformed or unsupported constraints remain unknown.
- .NET: comments/trailing commas, SDK versions, all nine roll-forward policies
  and allowPrerelease. Inspect up to 16 roots and 256 entries per SDK directory;
  require dotnet.dll presence. Calculate a candidate separately per root without
  claiming effective resolver selection. Detect but do not follow sdk.paths.
  Workloads, custom resolvers, actual SDK contents and Visual Studio/CLI defaults
  remain unverified.
- Rust: package.rust-version minimum and a toolchain-name candidate in rustup's
  bounded local store. Components/targets, custom paths, workspace inheritance
  and effective overrides/aliases remain unverified. No rustup downloads.
- Node: engines.node, devEngines runtime, package-manager declarations and pin
  files. Stable numeric comparator sets, OR, wildcards, caret, tilde and hyphen
  ranges have bounded parsing. Unsupported/prerelease forms remain unknown;
  Cargo's range semantics are not substituted for npm's.
- Python: requires-python and pin files. Stable numeric comparisons, exclusions,
  prefix wildcards and compatible-release ranges. Complex PEP 440 forms, virtual
  environments, backends and dependencies remain unverified.
- Compare up to eight PATH candidates' embedded Windows product metadata, never
  command output. Wrappers/proxies, prereleases, unsupported or mixed evidence
  stay unknown. PE architecture is bounded and separate from the desired build
  target: host tools, cross-compilers and emulation can legitimately differ.
- Preserve observed PATH order and uncertainty. Infer the primary prerequisite
  profile from declarations (Rust, .NET, Node, Python in that order). Mixed
  projects may need additional profiles. No match or empty report establishes
  build readiness. Reports include local observation time.

## Evidence, 2026-10-21

Sixteen core tests cover policy/range semantics, invalid/duplicate declarations,
full TOML syntax, no script interpretation, fixture SDK/toolchain inventory,
ancestor boundaries, size bounds, custom paths, cancellation, PE parsing and
unchanged project contents. Two controls tests cover explicit activation,
accessibility descriptions and keyboard operation without opening a picker.

Full suite: 392 core, 43 regular UI and 47 native tests pass. One core and two
documentation tests remain ignored. Clippy passes with existing warnings.
A development-only example probe inspected SetupScope's local folder, preserved
PATH and Cargo.toml bytes, and left Rust proxy compatibility unknown. This is
not a new shipped CLI command or live GUI verification.

Live page interaction, screen reader/DPI, ARM64 runtime, real SDK/project behavior,
broader ecosystems, WSL/container inventory and dependency completeness remain open.

## Dependencies and references

The saved Windows development catalog's API/build/diagnostics sections were
consulted. Keep Rust/windows-sys; add no framework, native DLL or runtime.
TOML and SemVer reuse exact versions already in Cargo.lock; only direct-dependency
promotion changes the lockfile. Windows x64 builds/tests pass; ARM64 runtime is
not certified. Cached package license texts are retained under licenses/project-doctor.

Official repository metadata rechecked: toml-rs/toml was not archived, last pushed
2026-10-09; dtolnay/semver was not archived, last pushed 2026-06-24. Package sources
include MIT/Apache-2.0 texts. This is not a complete dependency vulnerability or
license audit or a claim that pinned versions are the latest.

- [.NET SDK selection](https://learn.microsoft.com/en-us/dotnet/core/tools/global-json)
- [Rust overrides](https://rust-lang.github.io/rustup/overrides.html)
- [npm declarations](https://docs.npmjs.com/cli/v11/configuring-npm/package-json)
- [TOML](https://github.com/toml-rs/toml)
- [SemVer](https://github.com/dtolnay/semver)
