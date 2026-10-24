# Source, downloads and PR integration

Original audit: 2026-10-21. Updated: 2026-10-24 for the focused five-tool tree.
Reviewed default branch for this update: `9db51aba9076d66bf3608067a60ee4fe5f708444`.
This records release and integration disposition. It is not a product acceptance
certificate.

## Downloads

The existing `v0.1.0` download is a legacy development prerelease. Its existing
assets and checksums are preserved. Those binaries predate the focused
developer-toolkit work shown in the README.

The October 21 audit described that earlier baseline. The focused 1.0.0.0
publication is now released from revision `5d6268b` and is described in
`RELEASE-VALIDATION.md` and the current README. Do not attach newer binaries to
the old tag or describe the newer tools as available in the legacy package.

## Pull-request disposition (2026-10-24)

PRs #45–#48 were reviewed against the pre-restructure checkout with
`git merge-tree --write-tree`; that check never modifies a checkout. All four are
now **closed without merging** (2026-10-22 20:52–20:53 UTC), because each one
extends a product surface the confirmed five-tool scope does not compile.
Original branches and attribution are retained.

| PR | Reviewed scope | Outcome |
| --- | --- | --- |
| [#45](https://github.com/SeanShaffer88/SetupScope/pull/45) | Portable Builder Kit, process supervision, project/staging model, GUI wizard and official-tool lifecycle workflow | Closed as out of scope. It would add a sixth workflow and re-create packaging/installation surfaces the current tree does not compile. Its README rewrite conflicts with the focused developer-toolkit positioning. |
| [#46](https://github.com/SeanShaffer88/SetupScope/pull/46) | Supplemental live-monitor coverage, notification/distinct-path counts, overflow diagnostics and backward-compatible saved logs | Closed as out of scope. Install monitoring is a retired surface; `src/gui/monitor.rs` is uncompiled. Retained saved-log compatibility is unchanged. |
| [#47](https://github.com/SeanShaffer88/SetupScope/pull/47) | Contextual undo for a failed health check only when this run has a finalized backup | Closed as out of scope. Canary-style health checks and the Programs removal page are not part of the compiled interface. |
| [#48](https://github.com/SeanShaffer88/SetupScope/pull/48) | Before/after cleanup health deltas, with causation caveat | Closed as out of scope. General cleaning and its preview panel are retired; there is no current route for the delta panel. |

These closures are scope decisions, not acceptance of the branches' code. None of
them was merged, and historical branch checks do not certify the merged app.

## Pre-restructure audit branches

The audit fix series was written against the earlier single-window product. The
restructure published with 1.0.0.0 replaced that interface with five focused tool
windows, so most of those branches have no compiled surface left. No branch below
is a pending integration gate.

| Branch family | What it changed | Current disposition |
| --- | --- | --- |
| `fix/p0-tooltips-1` … `-11` | Explanations reachable without a mouse (tooltips, focus text, screen-reader descriptions) across the earlier pages | Landed before 1.0.0.0. The affected pages were retired by the restructure; every retained tool now uses `gui::a11y::explain`, and the focused-shell tests cover control names and widths. |
| `fix/p0-wizard-disclosure`, `fix/p0-bloat-shield`, `fix/p0-cleaner-preview`, `fix/p0-confirm-system`, `fix/p0-dashboard-keyboard` | Wizard disclosure, bloat shielding, cleaner preview, confirmation routing and dashboard keyboard work | No compiled surface remains (installer wizard, debloat, cleaner and the sidebar dashboard are retired). Retained as history, not integration candidates. |
| `fix/p1-cli-safety` | CLI confirmation discipline, per-command help data and classification tests | Already present in the supported console entry point: `src/toolkit_cli.rs` requires typed `INSTALL`, `APPLY`, `UNDO` or `RUN` on an interactive terminal and refuses non-interactive confirmation, and `docs/CLI.md` documents it. The legacy `src/cli.rs` copy is uncompiled archival source, so its extra per-command help was not ported. |
| `fix/p1-copy-accuracy`, `fix/p1-monitor-honesty`, `fix/p1-progress-honesty`, `fix/p1-space-wasters`, `fix/p1-sidebar-placeholders` | Copy and behavior corrections on retired pages (backups, monitor, cleaner progress, space wasters, sidebar) | Superseded by the restructure. Those pages, the sidebar and the space-wasters panel are not compiled; the current five-tool copy is written from scratch and reviewed in `TOOL-WINDOW-QUALITY.md`. |
| `fix/p1-1-errors` | Failures get a distinct error presentation instead of ordinary status styling | Ported to the restructured tree and merged as PR #60. `App` and the compiled pages track severity and render errors with ✖ in the error colour; recorded in `CHANGELOG.md`. |

Retained core/library modules and uncompiled legacy source (for example
`src/gui/cleaner.rs`, `src/gui/monitor.rs`, `src/cli.rs` and the storage, startup
and privacy modules) are compatibility/archival code. They are not supported
current application features and are not evidence that a surface exists.

## Current release evidence

1.0.0.0 is published from revision `5d6268b`. Two changes landed afterwards:
PR #59 rewrote the README download and workflow guide, and PR #60 changed failure
presentation and added regression coverage. Neither re-ran the packaging,
installer or clean-host checks, so those recorded results stay attached to the
1.0.0.0 revision rather than to the current tree.

Remaining release gates are unchanged and cannot be satisfied in this repository
alone:

1. A disposable clean Windows 11 VM run (currently documented as skipped at the
   owner's request).
2. Actual screen-reader coverage and a broad DPI/GPU/Windows-build matrix.
3. Native ARM64 runtime acceptance (CI proves compilation only).
4. Real vendor installation, UAC, reboot and uninstall/recovery behavior on a
   disposable target.
5. Human native-speaker review of the ten translated catalogs.

No signing certificate, clean-machine certification or universal compatibility
claim is made. No unrelated local development checkout was modified and no live
cleanup, install removal or machine-policy operation was run for this record.
