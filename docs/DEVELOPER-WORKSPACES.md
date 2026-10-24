# Developer command center and workspaces

> **Archival record (updated 2026-10-24).** This page describes the earlier
> single-window command center, its desktop scratch workspaces and the App
> Migration entry point. Those surfaces are retired: the compiled interface is
> the six-card Workflow Board with five tool windows, and the searchable
> dashboard, scratch and migration code is not linked into it. Kept as a dated
> design record, not as a description of current behavior. Current navigation:
> the README and [Project Readiness](PROJECT-READINESS.md).

The main window is a searchable tool dashboard. Each tool opens in its own native desktop window using the existing desktop renderer. Several tools can be open at once. Selecting an already open tool focuses its window instead of starting another instance. Window geometry can be adjusted independently.

Closing a tool hides its window and preserves its in-memory inputs, reports and jobs for the current SetupScope session. It does not cancel work. Reopening returns to that state. These in-memory sessions are not persisted across application exits. Closing the main SetupScope window closes all tool windows. Finish active operations before exiting.

Consequential shared workflows (program removal, transfer review, backup recovery and confirmation dialogs) stay in the main window. New reviews bring the dashboard forward. Each tool has a Show Dashboard button. A single shared application state avoids competing inventory, settings and recovery stores. This is concurrent access to distinct tools, not parallel independent copies of the same destructive workflow.

Developer diagnostics are individual tools: Build Readiness (including PATH and version evidence), Project Doctor, Build-Log Triage, WSL And Containers, Ports And File Owners, Diagnostic Bundle, Environment Profiles, Installer Regression and Repeatable Setup Export.

## One developer interface

There is no beginner/advanced selector. Settings and technical details are directly available. An old serialized mode flag remains for compatibility with existing settings. Loading a legacy basic-mode settings file preserves its effective backup/confirmation protection and leaves formerly disabled risky options disabled. It then migrates to the unified interface. High-risk removal items remain visible and start unchecked; explicit recovery and review gates remain.

## Disk-backed scratch workspace

Choose an existing absolute parent directory, including a folder on a separate local scratch drive, then Create New Scratch Folder. A unique folder and plain-text notice are created without replacing existing content. No files are deleted automatically. Creation runs on a background worker and failure reports identify a partially created directory where possible.

Copy Folder Path or copy literal PowerShell TEMP/TMP assignments. SetupScope does not execute the assignments. Pasting them changes only that shell and subsequently launched child processes, not persistent user/machine environment values or existing processes. The generated single-quoted assignments escape apostrophes. Restarting the shell resets its inherited environment.

This is an ordinary folder on existing storage: **not a RAM disk, virtual disk, quota, capacity reservation or sandbox**. Builds can fill the underlying drive. The system TEMP location can be cleaned by Windows or other software; do not store source, secrets or durable outputs there. Review and remove unneeded workspaces manually. SetupScope does not keep a permanent scratch catalog or clean it on exit.

A bounded dynamically sized VHDX scratch volume remains a separate roadmap item. It needs reviewed creation/formatting, explicit capacity, elevation handling, detach/recovery receipts and failure tests before it can be offered safely.

## App migration

App Migration makes the existing transfer workflow discoverable. Open Programs to select applications and choose its transfer action; review collected files/settings and warnings before export. Inspect Transfer Package checks an existing package against the destination before the separate restore review. Opening the tool performs no restore or migration.

Transfer is not universally supported for installed applications. Services, drivers, Store packages, licensing, activation, credentials, machine-specific registration and native dependencies can require vendor reinstall or separate configuration. Preserve originals and backups until the destination is verified. No automatic relocation of installed folders or junction-based app moves is provided.

## Keyboard and limits

Use Tab/Shift+Tab and Enter/Space on launcher cards; Ctrl+F searches the main launcher. Existing Ctrl+1 through Ctrl+9 shortcuts open/focus their corresponding tools. Ctrl+Tab cycles launch targets. Tool-specific refresh remains contextual. F1 toggles help inside the active tool window; shared operation reviews use the main window. Some workflows still open their existing review dialogs there.

The regular Windows desktop backend supports native tool viewports. The optional native Programs preview remains an incomplete migration prototype. Automated tests cover scratch uniqueness, non-overwrite behavior, literal escaping and settings migration; live multiwindow, keyboard, DPI and screen-reader checks must be reported separately. Broad Windows/architecture certification is not implied.
