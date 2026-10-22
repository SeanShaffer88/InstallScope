# Redacted diagnostic bundles

Developer diagnostics > Redacted diagnostic bundle (local export), regular interface.

1. Refresh the build readiness scan below if needed.
2. Select application runtime, readiness summary and/or known tool candidate counts.
3. Generate a preview and inspect its complete JSON.
4. Acknowledge review and save to a new filename. Nothing is uploaded.

Schema 1 JSON is bounded to 32 KiB. The exact preview bytes are saved using staged,
no-replace durable writes; an existing destination is retained. Changing sections
or generating a new preview clears review. A preview is an immutable snapshot;
subsequent scans do not silently change its bytes. No automatic collection or save.

Redaction uses an allowlist, not a heuristic secret detector. Only fixed tool names,
counts, fixed profile/target/status summaries, app version, process architecture,
OS family and assembly date are included. Raw paths, usernames, machine IDs,
addresses, PIDs, custom command names, free-form diagnostic messages, embedded
versions, environment values, manifest contents, profile receipts and regression
snapshots are excluded. Even permitted counts can describe your setup: review
before sharing. There is no option to include raw source evidence in this workflow.

The assembly timestamp is not the original scan date; that date is unavailable
for existing readiness reports. The bundle says this explicitly. Cancelled/partial
flags remain visible. Presence/counts do not prove a build works. Other diagnostic
workspaces are not yet bundle sources. Files are local JSON, not ZIP archives.
No third-party dependency or service was added; existing JSON and durable-write
infrastructure were reused after consulting the saved resource catalog.

Verification: all seven section combinations exclude injected private Unicode/path
sentinels from source fields; missing evidence/empty selection and size/relative
path/overwrite guards are tested. Live Windows scan/export probe verified exact
preview bytes and no replacement. Full suite: 435 core, 54 regular UI, 58 native
preview checks pass (one core and two doc tests ignored). Clippy passes with
existing warnings. Live GUI, accessibility, ARM64 and concurrent parent-path-swap
resistance remain unverified; staged writes are not a hostile-filesystem boundary.

Subsequent live local x64 GUI smoke: runtime preview rendered, save was disabled
before review and enabled after acknowledgment; changing a section invalidated
the preview/review. Genuine screenshot captured. Full accessibility remains open.
