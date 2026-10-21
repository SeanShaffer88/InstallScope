# Portable Builder Kit — Rust core

Embed reliable PortableApps.com package checks and build-result handling in Rust
software without a Python runtime. MIT, no runtime dependencies, Rust 1.85+.
This is an independent project; no PortableApps.com endorsement is implied.

This native core succeeds the initial Python reference implementation for
InstallScope integration. It deliberately reuses the host's cryptography,
compiler provisioning and process runner. It does not duplicate the existing
capture engine or claim to turn every installed application into a portable app.

## Integrate

```toml
[dependencies]
portable-builder-kit = { path = "path/to/portable-builder-kit", version = "0.2.0" }
```

```rust
use portable_builder_kit::{package_app_id, preflight};
use std::path::Path;
let root = Path::new("build/ExamplePortable");
// Call from a background worker in a desktop application.
match package_app_id(root) {
    Ok(id) => println!("Package: {id}"),
    Err(error) => eprintln!("Fix package metadata: {error}"),
}
for finding in preflight(root, true) {
    eprintln!("{}: {}", finding.path.display(), finding.message);
}
```

`preflight(root, false)` requires the Launcher INI before compilation;
`preflight(root, true)` checks the compiled launcher's DOS/PE signatures instead.
The finding type separates path and explanation so hosts can display selectable,
keyboard-accessible diagnostics, rather than encoding meaning through color.
All calls are synchronous. The host owns threading, accessible UI, progress and
cancellation. No GUI accessibility testing is implied by these data structures.

## Verify an installer result

1. Call `ArtifactWatch::capture` with the package parent and (if the builder uses
   it) the package directory, plus the AppID and a complete-file SHA-256 callback.
2. Run the official installer builder with the absolute package directory as one
   argument through your application's supervised process runner.
3. Call `finish(exit_code, hash)` after the process ends successfully.
4. Retain the returned `BuildArtifact.path` and SHA-256 in your build report.

The SDK rejects nonzero exits, unreadable files, linked/reparse output files,
unchanged output, ambiguous multiple outputs and malformed DOS/PE signatures.
Candidates must start with `<AppID>_` and end with `.paf.exe` (case insensitive).
A new unrelated application installer is ignored. Directory errors propagate.
At most 10,000 directory entries per output directory are inspected.

Use a fresh, private parent directory per job. A byte-identical rebuild is accepted when its modification time changes. If
both bytes and modification time remain unchanged, there is no evidence the
compiler replaced the artifact, so the build is rejected. Renaming or deleting existing release files
is the host's policy, not an automatic side effect of the SDK.

The SHA-256 callback must use your established crypto implementation and hash the
entire file. A checksum returned here records bytes; it does not establish trust
in a downloaded compiler or prove runtime portability. Tool acquisition, complete
tool-distribution verification, licensing, signing and source provenance remain
host responsibilities. Never hash a freshly downloaded untrusted EXE and call
that independent verification.

## Native developer utility

```sh
cargo run --manifest-path crates/portable-builder-kit/Cargo.toml --example preflight -- build/ExamplePortable
```

The utility prints every missing/invalid required asset. Exit codes: 0 static
checks passed, 1 usage error, 2 findings. Paths are separate from shell arguments;
no shell is invoked by this utility. No network or system changes occur.

## Verification and limits

```sh
cargo test --manifest-path crates/portable-builder-kit/Cargo.toml --locked
cargo clippy --manifest-path crates/portable-builder-kit/Cargo.toml --all-targets --locked -- -D warnings
```

Contract tests exercise stale and ambiguous outputs, process failures, malformed
metadata, UTF-8/UTF-16 input, filename aliases, path traversal, PE offsets, missing
assets and hash errors. Unix additionally tests symlink output. Windows CI is
configured separately; configuration is not evidence of execution.

The metadata reader is bounded to 64 KiB. It rejects ambiguous duplicate keys and
sections. It checks format type, AppID and Control Start; it is not a full format
schema validator. Image presence checks do not validate ICO/PNG dimensions or
contents. PE checks validate DOS and PE signature bounds, not a complete loader
or Authenticode signature. Ancestor junctions, concurrent file replacement and
hostile races require OS isolation; this SDK is not a sandbox. Inspect only
trusted build directories. The current InstallScope adapter still inherits its
existing process runner's cancellation/timeout limitations.

Actual official installer compilation, clean Windows launches, relocation,
upgrades preserving Data and crash recovery remain release gates. Do not label
this a complete all-in-one production toolchain until those checks pass.

Author / Principal Architect: Sean Shaffer  
Security: seanshaffer88@proton.me  
Support: https://buymeacoffee.com/seanspawn88
