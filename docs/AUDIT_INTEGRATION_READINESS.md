# Source, downloads and PR integration — October 21, 2026

This records the portfolio audit's release and integration disposition. It is not a product acceptance certificate. Reviewed default branch: `d48d07de5497a0484786709c2a2f39ee52fdbc2e`.

## Downloads

The existing `v0.1.0` download is **InstallScope 0.1.0 — legacy development build**, explicitly marked as a prerelease. Its existing installer, x64/ARM64 ZIP names and checksums are preserved. Those binaries predate the SetupScope rename and the newer developer-toolkit work shown in the README.

The README's newer local screenshots are not evidence that equivalent code or downloads have been published. The next developer-toolkit release must first commit the intended source, run its gates, and package binaries from that exact source revision. Do not attach a locally built binary to the old tag or describe the newer tools as available in the legacy package.

## Open PR disposition

Prerequisites and conflicts were checked with `git merge-tree --write-tree` against the reviewed default branch; that check does not modify a checkout. Original PR branches and attribution are retained.

| PR | Reviewed scope | Disposition and gate |
| --- | --- | --- |
| [#45](https://github.com/SeanShaffer88/SetupScope/pull/45) | Portable Builder Kit, process supervision, project/staging model, GUI wizard and official-tool lifecycle workflow; 34 changed files | Keep separate. README conflicts with the newer developer-toolkit positioning. Resolve that scope, preserve current CI pins, and rerun the SDK's contracts/process tests plus the official frontend lifecycle on the resolved commit. Historical branch checks do not certify a merged app. |
| [#46](https://github.com/SeanShaffer88/SetupScope/pull/46) | Supplemental live-monitor coverage, notification/distinct-path counts, overflow diagnostics and backward-compatible saved logs | Keep open for integration. README conflict must preserve current scope. Adapt old crate names in added code where necessary, run old-log round trips and real temporary-folder watcher tests, and check watcher failure/stop/restart reporting. Requested/started roots are not proof of continuous capture; final snapshot comparison remains authoritative. |
| [#47](https://github.com/SeanShaffer88/SetupScope/pull/47) | Contextual undo for a failed health check only when this run has a finalized backup | Keep open for integration. ROADMAP conflict and old InstallScope wording/imports require reconciliation. Verify the confirmation routes to the exact completed backup, offers no undo for failed/off backups, and explains that restoring may reinstall the whole removed app. Do not execute removals on a personal PC as an acceptance shortcut. |
| [#48](https://github.com/SeanShaffer88/SetupScope/pull/48) | Before/after cleanup health deltas, with causation caveat | Keep open pending scope reconciliation. README, ROADMAP and `src/gui/cleaner.rs` conflict. The newer local product direction retires general cleaning, but current hosted source still contains it; this PR cannot yet be called obsolete. Decide during source consolidation whether to port its delta logic to a retained workflow. |

These four branches are not demonstrably superseded by main, so none was closed as redundant. Review of the large builder branch was scoped to architecture, process and lifecycle boundaries; it was not an exhaustive security review of every added line. No new combined source/binary release is claimed here.

## Required next-release evidence

1. Select and commit the intended developer-toolkit scope without overwriting unrelated local development.
2. Resolve PR overlap and rerun the repository's format, tests, lint and release-build gates on the final combined commit.
3. Record standard-user Windows runtime checks, restart/persistence behavior and safe failure/cancellation cases. Keep destructive uninstall/rollback tests on explicitly disposable targets.
4. Create a fresh version/tag and package from that exact commit. Publish architecture-specific checksums, source revision, actual checks and known gaps together.
5. Verify downloaded assets against the published checksums before changing the README's download claim. Native ARM64, installer and accessibility acceptance must retain their own evidence labels.

No unrelated local development checkout was modified and no live cleanup, install removal or machine-policy operation was run for this audit.
