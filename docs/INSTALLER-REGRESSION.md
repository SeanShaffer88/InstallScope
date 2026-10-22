# Installer regression workspace

Status: initial guided workflow implemented in the regular interface under
Developer diagnostics > Installer regression workspace (local evidence).
Automated fixture/control checks pass. Real vendor lifecycle, live UI,
screen-reader/DPI, clean Windows and ARM64 acceptance remain unverified.

## Use locally

Configure narrow snapshot roots and exclusions in Settings for the fixture.
Enter a name, Prepare scope from Settings, review roots/exclusions and isolation,
then Create local workspace. Explicitly Capture and save checkpoint for Before.
Perform the reviewed install externally, record its observed outcome and optional
exit code/note, review again, then capture After install. Optionally capture After
upgrade; capture After uninstall when finished. SetupScope runs none of these
vendor operations. Choose saved evidence pairs and Compare saved evidence.

Refresh saved workspaces and Open or reload workspace to resume after restart,
another instance's work or an uncertain save. Earlier evidence and failed/pending
attempts remain. A failed finish publication can leave a pending receipt and an
uncommitted snapshot; retrying reserves new evidence and preserves both.

Comparison pages show up to 50 differences and summary counts; select One
difference per page for long paths. Copying a page explicitly includes local paths
and registry names, not raw registry contents. Pages cap at 128 KiB and mark
truncation. Full raw registry values remain in sensitive saved snapshots, so
protect the data directory. No delete/apply action or automatic export/upload exists.

Settings changes do not alter a saved run's scope. To narrow a plan that reaches
a limit, change Settings and create a new workspace; repeating the old plan
cannot narrow it. Missing roots, access failures and uncertain reported outcomes
remain explicit. Same-size/same-time file-content changes can be missed because
files are inspected by metadata rather than hashes.

Current limits: 1 MiB manifest, 256 KiB scope, 64 file roots, 256 entries per
registry/exclusion list, 32 attempts, 256 IDs per listing, 50,000 cooperative work
checkpoints including registry values/visits, 64 MiB payload accounting, 8 MiB per
native registry value buffer, 64 MiB compressed and 128 MiB expanded per snapshot.
Conservative accounting may stop earlier. These are not total process-memory or
time caps; OS calls may block until they return.

Windows MachineGuid and current token SID are hashed into a local identity
marker; their raw values are not saved. Names are also checked. Unavailable or
different identity blocks capture/compare. This guards accidental reuse; cloned
systems, edited receipts or concurrent malicious path changes are not prevented.
Receipts are not signed or authenticated. No new dependency/runtime was added.

## Intended workflow

1. Review a named fixture plan and exact capture scope in an appropriate isolated
   Windows test environment. Freeze expanded roots, exclusions and machine/user
   identity when creating the workspace. Exclude SetupScope's evidence directory
   from captures. Choosing or opening a plan never executes an installer.
2. Explicitly capture and save Before evidence. Perform a reviewed vendor install
   externally, then record its observed outcome and capture Installed evidence.
3. Optionally repeat for an upgrade, then uninstall and capture Uninstalled
   evidence. Success, failure, cancellation, unknown and reboot-required outcomes
   must be distinguishable. User-reported outcomes are not verified exit codes.
4. Compare adjacent checkpoints and Before against Uninstalled. Review remaining
   deltas without an automatic delete/apply action. Background changes cannot be
   attributed to the vendor merely because they occurred between captures.
5. Reopen versioned local receipts after restart. Show incomplete evidence,
   warnings and interrupted attempts. Never infer a successful uninstall or a
   clean machine from absent, unreadable, cancelled or incomplete evidence.

No automatic testing against the owner's installed software, vendor command
execution, upload, new background service or added packaging runtime is required.

## Data and recovery requirements

- Bounded, versioned manifests with validated stage ordering, opaque local IDs,
  regular non-reparse evidence paths and explicit unsupported-version failures.
- Per-workspace operation lock plus revision checks; unique reserved capture
  directories; write evidence before publishing the receipt. Preserve earlier
  evidence and interrupted/orphan captures instead of deleting them automatically.
- Persist the reviewed scope, date, identity, outcome provenance and warnings.
  Refuse cross-machine comparisons or changed scope presented as the same run.
- Bound input, checkpoint count, capture work and report output. Item cancellation
  is cooperative and cannot establish a hard allocation/time bound. Registry
  values and parser allocations require separate consideration.
- Reports may contain local paths and registry data. Saving evidence is explicit;
  copying/exporting raw evidence must explain that it is not a redacted bundle.

## Implemented snapshot prerequisites

- `load_snapshot` rejects expanded input above 512 MiB; callers can use
  `load_snapshot_bounded` for a smaller quota. Exact-boundary valid snapshots still
  undergo gzip CRC/trailer validation. Limits cover expanded input, not all parser
  allocations. Legacy JSON and binary snapshot tests still pass.
- `Progress` supports an optional item limit, configured before starting work.
  Reaching it cancels cooperatively and capture returns an explicit limit error.
  Existing captures remain unlimited unless a caller selects a limit.
- A failed file worker now fails the capture rather than silently dropping its
  records. All worker handles are joined before the error returns.
- Imported file-size totals saturate at `u64::MAX` instead of wrapping/panicking.
  A saturated number is a display ceiling, not an exact byte-count guarantee.

## Verification and remaining gates

Thirteen Windows core tests cover synthetic file/registry lifecycle, upgrade and
leftovers, own-data exclusion with normal/forward-slash/verbatim path spellings,
reopening, missing roots, outcomes/exit codes, cancellation/retry, interrupted
receipts, locks/stale writers, forced finish-publication failure, invalid IDs,
schema/identity/scope changes, corrupt/missing/oversized evidence and budgets.
Four regular-interface control tests cover review gates, invalid exit codes,
keyboard scope preparation, accessible explanations and paged bounded Unicode
reports. These are automated controls, not real screen-reader acceptance.

The development-only `regression_probe` executable was run against newly created
scratch files on this Windows PC. It reopened four checkpoints, detected the
upgrade metadata change and one synthetic leftover, and excluded its own data.
No vendor operation or owner application/registry mutation ran in this probe.
Its retained evidence and log are development artifacts, not a shipped CLI.

Disk-full/permission fault injection, actual process-kill/power-loss recovery,
real vendor lifecycle and live UI/platform acceptance are still open. A locked
receipt publication and manually constructed interrupted receipt were tested;
they do not prove every storage interruption scenario.

Use temporary file trees and an in-memory registry for reproducible
before/install/upgrade/uninstall fixtures. Cover unchanged and leftover cases,
failed and cancelled vendor outcomes, reboot requirements, capture warnings,
stale writers, lock contention, interrupted publication, corrupt/oversized input,
unsafe paths, scope changes and unknown schema versions. Fixture operations must
not touch the user's actual installed applications or registry.

Exercise the live UI's review gate, keyboard order, accessible explanations,
busy/cancel/retry states and reopening flow. Real vendor lifecycle, clean Windows,
DPI/screen-reader and ARM64 checks remain separate release gates; synthetic tests
cannot prove them.
