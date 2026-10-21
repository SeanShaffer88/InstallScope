# Readiness checkpoint — 2026-10-21

## Scope and decision

Start from main at `6079cc4`. Preserve existing uncommitted Slowdown Detective
work in the original checkout; perform this batch in a separate git worktree.
Prioritize a reproducible input-handling defect over optional feature expansion.
No dependency, framework, privilege, persistence format, or network changes.

## Completed

- ZIP opening now rejects inputs shorter than the mandatory 22-byte end record
  before indexing the input buffer. Previously an empty input panicked.
- Regression test reproduced that panic before the fix.
- Added coverage for short inputs, every truncated prefix of a generated archive,
  valid empty archives, and a ZIP signature inside an archive comment.
- Format requirement checked against PKWARE APPNOTE 6.3.8, sections 4.3.1 and
  4.3.16, located using Exa:
  https://pkwaredownloads.blob.core.windows.net/pkware-general/Documentation/APPNOTE-6.3.8.TXT

## Verification on Linux x86_64 / Rust 1.99.1

- `cargo test --locked --offline`: 271 library tests and 30 application tests
  passed; one library test and two documentation tests ignored by the suite.
- `cargo clippy --all-targets --locked --offline`: passed with existing warnings;
  warnings were not suppressed.
- `cargo build --locked --offline`: passed.
- Built CLI `--version` and `--help`: passed on Linux.
- Isolated ZIP/Deflate regression suite: 15 passed, one ignored.
- `cargo fmt --all -- --check`: passed after including the existing GUI changes.
- `rustfmt --edition 2021 --check src/zip.rs`: passed.
- `git diff --check`: passed.

## Remaining release gates

This is a correctness batch, not a completed release. Windows is unavailable in
this environment. Actual Windows install/uninstall, recovery, permissions,
accessibility, scaling, native UI, and installer/portable artifact behavior remain
unverified here. Continue with Windows CI and a disposable Windows test machine
before claiming release readiness. No release percentage or ETA is reliable until
the intended release scope and runtime results have been reconciled.

The regular interface remains the usable path while native Win32 migration is
in progress. Remaining roadmap features must retain their planned status until
implemented and verified. At the user’s request to commit all changes, the existing Slowdown Detective GUI
and documentation changes were copied into this branch and tested together.
The original checkout remains unchanged.

## Tool fit across projects

Exa can locate primary technical references for concrete implementation questions.
Render is connected, with no selected workspace; select a user-confirmed workspace
only if hosted infrastructure is actually needed. InstallScope, Meridian, and
DesktopMayhem currently describe local Windows applications, so this inspection
provides no reason to introduce cloud hosting or a Supabase backend. Template
Creator is appropriate when a reusable artifact template is requested, rather than
for ordinary source-code fixes. No hosted resources or templates were created.

GitHub metadata and READMEs were retrieved for Meridian and DesktopMayhem.
`SeanShaffer88/agent-sandbox-diff` returned 404; confirm its current repository
location or access before attempting implementation there.

## Follow-up cleaner review safeguards

The product direction is recorded in ROADMAP.md. Cleaner scans now deduplicate
identical paths across overlapping patterns. Reviewed cleanup checks metadata
without following a replacement link and skips missing, unreadable, non-file,
linked, or size-changed entries. The GUI explains these skips. This is not a
file-identity guarantee: same-size edits, parent-directory links, and changes
between the metadata check and deletion remain limitations. Logical file sizes
are not guaranteed physical disk space reclaimed (hard links, compression, and
sparse files can differ). Recycle Bin and unreviewed scheduled cleaning are
unchanged by this batch.

Verification: `cargo test --locked --offline` passed 274 library and 30
application tests; three tests remain ignored by the suite. Added tests exercise
overlapping patterns including protected items, a size change after scanning,
and a replacement symlink with a surviving target (Unix only). Formatting,
diff whitespace checks, and all-target Clippy passed; existing Clippy warnings
remain. Windows runtime and Windows reparse-point behavior remain unverified.

## Cleanup reporting checkpoint — good stopping point

Reviewed cleaning now retains a structured in-memory result for each supplied
file: removed, already gone, size changed, no longer a regular file, or failed
with the OS error. The GUI shows counts, selectable full paths, and an explicitly
bounded 1,000-row list; Copy full cleanup report includes all outcomes. Its button
and disclosure have hover, keyboard-focus, and accessibility descriptions through
the existing helper. Reports survive automatic/manual rescans and are replaced
by the next cleanup or lost at exit. Recycle Bin results are aggregate only;
empty-directory tidy-up remains best effort and is not itemized. No persistence
format changed, and the summary-only API remains available to existing callers.

Verification: 276 library and 30 application tests passed on Linux; three tests
remain ignored by the suite. New tests exercise mixed per-file results and an
invalid path producing an explicit error. Full formatting, whitespace checks,
and all-target Clippy pass with existing warnings. Windows interaction, clipboard,
screen-reader runtime, locked-file failure and Recycle Bin operation are not
verified here. This is a completed implementation batch, not release approval.
Pause after publishing this batch; next validate the PR on Windows and exercise
review → clean → inspect report → copy report → rescan on disposable data.

## Opt-in saved cleanup report

Added a version-1 display-only receipt at cleanup-report.json in the data folder.
Save and Delete always confirm with Cancel focused; Load replaces only displayed
results. All I/O runs through existing background jobs. Reports are never saved
automatically, uploaded, or treated as executable actions. One file and a 16 MiB
limit bound retention. Full paths and OS error text are local, unencrypted data.
Loading reports is explicit; new cleanup results do not overwrite a saved report.
Invalid/future versions, unreadable or oversized data and overflowing byte totals
are reported, not silently discarded or overwritten. Staged writes are synced
before rename, and failed staging attempts remove the current temporary file.
Multi-instance/external modification races and full power-loss durability are not
verified; an abrupt process death may leave a uniquely named temporary file.
This is report persistence, not an operation recovery journal or undo engine.

Prior PR head e5cf685 passed GitHub Windows tests/build, Arm64 build and Linux
core checks; the scheduled performance check was skipped. This does not verify
Windows accessibility or runtime behavior of the new saved-report controls.
Current local checks: 281 library tests and 30 application tests passed; three
suite tests ignored. New tests cover all outcomes, replacement, delete-without-
acting-on-targets, corrupt/future files, v1 fixture compatibility, oversized
reads, failed saves and integer-overflow rejection. Formatting and all-target
Clippy pass (pre-existing warnings retained). Windows client, low-disk/crash,
clipboard and assistive-tech interaction remain pending for this new batch.

The final Linux debug build also passed, along with changed-file whitespace
checks against the preserved source baseline. New report controls use translation
keys with English fallback; translated strings and actual Windows UI behavior
still require review. The source baseline was compared by Git blob hash against
the remote PR tree before publication; no unrelated remote file was replaced.

## Read-only storage overview

Added storage.rs and the regular-interface Storage overview page. Scans are
explicit, backgrounded and cancellable at filesystem checkpoints. Retains the
largest 200 immediate entries and first 100 error details, while totals/counts
include measured entries beyond those lists. The iterative subtree traversal
bounds open directory frames at 256 levels; observed links and Windows reparse
entries are skipped. Errors, skips, depth limits and cancellation are visible
partial-result states. Existing results remain during a replacement scan. Paths
are selectable; controls use existing tooltip/focus/assistive-text helpers.

This first slice measures logical file lengths only: no allocated-space or
reclaim guarantee, hard-link deduplication, treemap, deletion, automatic scan,
saved history, or content hashing. It is not a filesystem snapshot/security
boundary. Ancestor links may resolve the selected root, and concurrent changes
can race metadata checks. Blocked filesystem calls cannot be interrupted by the
cancel flag. Progress counts entries rather than claiming a known percentage.

Validation: 286 library and 30 application tests passed locally (three suite
checks ignored); formatting and all-target Clippy passed with existing warnings.
Five new scanner tests cover nested sizes/ranking and unchanged file contents,
invalid roots/cancellation, bounded rows with full totals, Unix link skipping,
and Unix depth-limit partial results. Windows reparse/ACL/long-path, actual
screen-reader/DPI and UI interactions remain pending. New strings use English
fallback pending translation review. Release progress/ETA cannot yet be reliably
estimated across all remaining tools.
