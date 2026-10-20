<p align="center">
  <img src="assets/logo-wide.png" alt="InstallScope" width="600">
</p>

<p align="center">
  <b>An all-Rust Windows 11 toolkit for install monitoring, safe removal, portable packaging, and system recovery</b><br>
  Record what an installation changes. Review it, restore it, or repackage it.
</p>

<p align="center">
  <a href="https://github.com/SeanShaffer88/InstallScope/releases/latest"><img alt="Download" src="https://img.shields.io/github/v/release/SeanShaffer88/InstallScope?label=download&color=e53935"></a>
  <a href="https://github.com/SeanShaffer88/InstallScope/actions/workflows/ci.yml"><img alt="CI" src="https://github.com/SeanShaffer88/InstallScope/actions/workflows/ci.yml/badge.svg"></a>
  <a href="LICENSE"><img alt="License: GPL v3" src="https://img.shields.io/badge/license-GPLv3-blue"></a>
  <img alt="Platform: Windows 11" src="https://img.shields.io/badge/platform-Windows%2011-0078d4">
</p>

<p align="center">
  <a href="#overview">Overview</a> •
  <a href="#everything-total-uninstall-does-and-more">Feature comparison</a> •
  <a href="#the-tools">The tools</a> •
  <a href="#download">Download</a> •
  <a href="#getting-started">Getting started</a> •
  <a href="#command-line">Command line</a>
</p>

---

## Overview

Most uninstallers run the vendor's uninstaller and then *guess* at what was left behind. Snapshot tools show a before-and-after comparison, but stop there.

**InstallScope connects install monitoring to reviewable cleanup.** It records installation changes, builds an itemized removal plan, and backs up supported changes before applying them so they can be restored. Its Windows toolkit also includes classic uninstall with a leftover scan, startup and cleanup utilities, snapshots, a System Restore manager, PortableApps Forge, and silent setup-package generation from a recorded install.

InstallScope is free software under the GPLv3. It is built for everyday Windows users, power users, and developers who want clear reviews, recoverable changes, local-first operation, and practical setup tools. See the feature guide below for what is available today and the roadmap for work that is still planned.

## Core uninstaller and install-monitoring workflows

InstallScope covers many workflows found in established uninstallers, with an emphasis on inspectable changes and recovery:

| Total Uninstall feature | In InstallScope |
|---|---|
| Analyze installed programs | Programs page: details, signature, size, and a deep leftover scan |
| Add a program to the list for analysis | Programs > **Add program…** (any folder, such as a portable program) |
| Complete uninstall | Programs page: precise removal for monitored programs, deep leftover scan for the rest, batch and silent uninstalls |
| Monitor installations | Monitor page, **Install with InstallScope** in the right-click menu, and optional background monitoring |
| Agent that notices running installers | Background monitoring, which records new installations automatically (Settings, Advanced mode) |
| Compare system snapshots | Snapshots page |
| Search in detected changes | Install logs: search across every log |
| Group programs in folders | Install logs: groups and tags |
| Merge installation logs | Install logs > **Merge with** |
| Custom profiles | Monitoring profiles (Settings, Monitor and Snapshots pages) |
| Export detected changes, XML export, registry export, print | Install logs > Export (HTML, XML, `.reg`, PowerShell, JSON) and **Print** |
| Export program lists | File > Export program list (CSV) |
| Backup and restore of uninstalled programs | Backups page: restore supported changes InstallScope applies after review; a vendor uninstaller's own deletions are not guaranteed reversible |
| Transfer programs to another PC | Programs > **Transfer to another PC** |
| Cleaner | Cleaner page, with browser privacy cleaning, scheduled cleaning and deep cleanup |
| Autorun manager | Startup page, with startup impact, delayed start, services, tasks and other autostarts |
| Command line | Extensive CLI for monitoring, snapshots, install logs and rollback, cleanup, startup management, backup/restore, and package operations (`installscope --help`) |
| Portable version | Portable download, for x64 and Arm64 |

Additional InstallScope tools and differentiators:

| Only in InstallScope | Where |
|---|---|
| Visual map of what an installation changed, grouped by where it went (startup, services, Explorer, Windows folders, AppData, registry…) | Install logs, Monitor and Snapshots: **Map** |
| Confidence colours on leftovers: green (safe), yellow (review), red (careful), each with its reasons | Every removal review: **Select only green** |
| Pre-install check for bundled adware, offer screens, coin miners, web installers and unsigned or tampered files | Monitor page, before an installer runs |
| Test an installer in Windows Sandbox first and see everything it would change | Monitor page: **Test in Windows Sandbox first** |
| Silent setup package (`Setup.exe`) made from a recorded installation, for redeploying without the setup wizard | Install logs > **Create Setup Package…** |
| Questions in plain language ("top 5 space-wasters I haven't opened in 6 months") | **Ask InstallScope** (Ctrl+K) |
| When each program was last used, how often, its CPU use and startup cost | Program details, **Not used for 6 months** view, Dashboard |
| Lightweight and open-source alternatives, installed with one click through winget or Scoop | Program details and after an uninstall |
| Right-click menu manager: turn entries off, remove broken ones, deep clean of COM classes and environment entries | **Right-click menu** page |
| One-click removal of OEM bloatware, trials and pre-installed Store apps, also for new user accounts | **Bloatware Shield** page |
| System Restore manager | System Restore page |
| PortableApps Forge with a portability check and the PortableApps.com Launcher built in | PortableApps Forge |
| Free and open source (GPLv3) | |

## The tools

InstallScope is organized into pages, each with its own job. Every page has built-in help (the **Help** button or **F1**), and anything that could harm Windows or slow the PC appears only in **Advanced mode**.

### Dashboard

<img src="docs/screenshots/dashboard.png" alt="Dashboard" width="720">

Your system at a glance.

- **Summary tiles:** installed programs, monitored programs, broken entries, and backups (with the space they use). Click a tile to jump straight to it.
- **Quick actions:** install a program with monitoring, uninstall programs, speed up startup, clean junk files, System Restore, make a portable app, remove bloatware, and find leftovers of removed programs (Advanced mode).
- **Space-wasters you don't use:** the five largest programs that haven't been opened in six months, ready to review and uninstall.
- **Recently monitored:** the latest install logs, one click away.

### Programs (uninstaller)

<img src="docs/screenshots/programs.png" alt="Programs" width="720">

Every installed program, and the safest way to remove it.

- **Complete program list:** 64-bit, 32-bit, and per-user desktop programs with their icons, plus Microsoft Store apps. Search, sort by name, publisher, version, date, or size, and filter by *Monitored*, *Broken*, *Unsigned*, or *Store apps*.
- **Add a program by hand:** list a program Windows doesn't know about, such as a portable program, by choosing its folder. In the native preview, manual entries are analysis-only; removing one from the list never deletes its files.
- **Program details:** publisher, version, install date, size, location, digital signature, uninstaller, and registry key, with buttons to open its folder, open its registry key, search online, or visit its website.
- **Precise uninstallation of monitored programs:** added files, folders, registry keys, and values are deleted; values the installer changed or removed are **restored**; services are stopped and deleted; scheduled tasks are removed; and only the `PATH` entries the installer added are taken out.
- **Classic uninstallation with a deep leftover scan:** after the vendor's uninstaller finishes, InstallScope looks for remaining folders, shortcuts, registry keys, autostart entries, App Paths, services, COM registrations, file types and associations, context-menu entries, shell extensions, environment variables, `PATH` entries, firewall rules, scheduled tasks, and folders left empty. The system-wide scan also finds clearly stale uninstall registry entries, but only when both the declared install folder and uninstaller are missing; it excludes MSI, hidden, protected, and Windows-owned entries. Every item is reviewed before removal and registry data is backed up.
- **Unattended batch uninstallation:** tick any number of programs and walk away. Silent switches are detected for each installer technology (Windows Installer, Inno Setup, NSIS, WiX, and Squirrel); uninstallers run one at a time; restarts are held until the end; stalled uninstallers are skipped; and one combined leftover scan follows.
- **Silent uninstallation** of a single program, and **forced removal** (Advanced mode) when a program's uninstaller is broken or missing.
- **Trust information:** who digitally signed each program, with unsigned programs and broken signatures (possible tampering) flagged.
- **Usage:** when each program was last opened and how often (from Windows' own records, kept on this PC), how much processor time it uses while running, and how long it adds to each Windows startup. The **Not used for 6 months** view lists what you could do without.
- **Lightweight alternatives:** for heavy or ad-supported programs, free alternatives that do the same job (for example 7-Zip for WinRAR, SumatraPDF for Adobe Reader, VLC for RealPlayer or PowerDVD), installed with one click through winget or Scoop when you choose. They are suggested again right after an uninstall.
- **Confidence colours** in every removal review: green items are safe to remove, yellow ones deserve a look, red ones touch shared parts of Windows. Each colour comes with its reasons, and **Select only green** ticks just the certain ones.
- **Running programs and locked files:** if the program is running, it is asked to close (so it can save its work) before its files go; only the program being removed is ever closed, never another application that merely loaded one of its files. Services are stopped and InstallScope waits until their process has exited. If Windows Explorer has the program's shell extensions loaded, Explorer restarts during the removal so they are deleted at once. Anything still in use is scheduled for deletion at the next restart, and InstallScope says which program, service or Windows component is holding it.
- **User data is respected:** files a program created later are kept unless you choose otherwise.
- **Transfer to another PC:** pack programs with their files, settings, and registry entries into one file and restore them on a new PC, with a compatibility check, path rewriting for different user names or drives, and one-click undo.
- **Export the program list** as a CSV file.

### Monitor (install monitoring)

Record an installation so it can be removed precisely later.

- **Install and monitor:** choose an installer, or right-click it in File Explorer and select **Install with InstallScope**. InstallScope takes a snapshot, runs the installer, and records every change.
- **Pre-install check:** before anything runs, the setup file is checked for bundled adware (OpenCandy, InstallCore and others), offer screens, coin miners, web installers that download more while they run, missing or broken digital signatures, and download-site wrappers. Each finding is explained in plain words.
- **Test in Windows Sandbox first** (Windows 11 Pro, Enterprise and Education): the installer runs in a throwaway copy of Windows and InstallScope shows everything it would change, on a map, without touching your PC.
- **Try it, then undo** (Windows 11 Home, or wherever Windows Sandbox is off): the program is installed on your PC while every change is recorded, after a restore point is made; you review what it did, then InstallScope removes all of it again, or you keep it with a complete install log.
- **Manual monitoring** for anything else, including Microsoft Store apps, winget packages, scripts, and portable applications.
- **Live activity:** files being added, changed, and removed stream in while the installer runs, and registry areas report when something under them changes.
- **Restart-safe:** monitoring survives restarts, so installers that need a reboot are captured completely.
- **Review before saving:** inspect every captured change and discard background noise with one click.
- **Installer Decrapifier:** the review lists every service, scheduled task, autorun and Startup shortcut the installer added, judged *Required*, *Optional* or *Bloatware / telemetry / silent updater* with the reasons, for example "Discord added 6 things that run in the background; 2 look like telemetry or silent updaters". Ticked items are removed (backed up) when you save. Built-in rules judge most of them; for the rest you can ask a language model running on your own PC (Ollama, llama.cpp, LM Studio). Nothing leaves your computer.
- **Automatic detection** of the program's name, publisher, and version.
- **Automatic exclusions:** temporary files, logs, and browser and application caches are ignored, so each log contains only the installer's real changes. **Learn background noise** (Settings) finds what your PC changes on its own and excludes that too.
- **Monitoring profiles:** save scopes under a name, for example a quick one for Program Files only and a deep one for the whole system, and switch between them before monitoring.
- **Background auto-monitoring** (Advanced mode, optional): a low-priority process started at sign-in records installations automatically, even the ones you forgot to monitor.

### Install logs

Everything each monitored installation changed.

- **Complete records** of added, changed, and removed files, folders, registry keys and values, services, and scheduled tasks.
- **A map of where it put things:** changes grouped by what they mean (starts with Windows, services and drivers, scheduled tasks, Explorer and file types, Windows folders, program folders, AppData, shortcuts, registry) with a proportional overview bar. Click an area to focus on it, expand folders and keys like a tree, or switch to the classic list.

<img src="docs/screenshots/change-map.png" alt="Map of an installation's changes" width="720">

- **Groups and tags:** organize logs into groups and label them with tags; search and filter by either.
- **"Which program created this?":** type part of any file or registry path to find the monitored program it belongs to.
- **Merge logs:** combine a program and its update or plugins into one log. Later changes win, and the originals are kept.
- **Export** a log as an HTML report, XML, a `.reg` undo script, a PowerShell removal script, JSON for import on another PC, or **PortableApps.com Launcher settings**. **Print** opens a print-ready report.
- **Setup Package Creator:** select **Create Setup Package…** on an install log to build a silent `Setup.exe` and companion deploy folder (`install.reg`, `install.ps1`, `uninstall.ps1`, and captured files) for redeploying that recorded installation on another PC. InstallScope refuses to create `Setup.exe` when captured files are missing or copying fails, rather than producing a misleading partial installer. This creates a silent deployment package, not a customizable interactive setup wizard.
- **Uninstall from the log** with the same precise removal as the Programs page.

### Backups

Restore supported changes InstallScope makes during reviewed cleanup and startup workflows.

- **Automatic backups** of supported file and registry changes before InstallScope applies them. Files go to a quarantine folder, and registry entries are also exported to a timestamped `.reg` file that can be restored with a double-click, even without InstallScope.
- **One-click restore** of backed-up changes from an operation. Vendor uninstallers, permanent junk deletion, restore-point deletion, and Windows deep-cleanup actions are not fully reversible.
- **Optional extras:** a System Restore point before uninstalling, and copies of removed files in the Recycle Bin.

### Startup (autorun manager)

<img src="docs/screenshots/startup.png" alt="Startup manager" width="720">

Choose what starts with Windows.

- **Startup programs** from Run keys and Startup folders, turned on and off exactly like Task Manager does, so the change is reversible.
- **Startup impact:** how long the last Windows startup took and how much each program slowed it down (High, Medium, or Low), measured by Windows itself.
- **Delayed start:** start a program two minutes after you sign in instead of immediately, so Windows is ready sooner. *Start normally* undoes it exactly.
- **Services and scheduled tasks** (Advanced mode), with Windows' own entries hidden unless you ask for them.
- **Other autostarts** (Advanced mode): Winlogon, Active Setup, program replacements (IFEO), AppInit DLLs, Explorer extensions, icon overlays, browser helper objects, boot-time programs, and WMI subscriptions. Entries whose file is gone are flagged.
- **Protection:** entries that Windows, security software, or hardware need can't be switched off in Simple mode, and deleted entries are backed up.

### Cleaner (junk cleaner)

<img src="docs/screenshots/cleaner.png" alt="Junk cleaner" width="720">

Free the space taken by temporary files and caches.

- **Junk categories:** your and Windows' temporary files, crash dumps and error reports, system memory dumps, Windows setup and update logs, Delivery Optimization files, Windows Update downloads, shader and thumbnail caches, browser caches, Prefetch data, the font cache, and the Recycle Bin. Each one is explained, and *Scan* shows how much space it uses before anything is deleted.
- **Browser privacy cleaning** (each item off until you tick it): cookies, browsing history, website data, and saved sessions for Chrome, Edge, Brave, Vivaldi, Opera, Firefox, LibreWolf, Waterfox, and Floorp, in every profile. Bookmarks and passwords are never touched.
- **Clean automatically** every day, week, or month through Windows Task Scheduler, even when InstallScope is closed.
- **Deep cleanup** (Advanced mode): the previous Windows installation (Windows.old), superseded Windows components, files of turned-off features, and old restore points, using Windows' own tools. Each item is analyzed first and confirmed separately.

### System Restore

Manage Windows' restore points without digging through Control Panel.

- **Every restore point** with its date, description, and type (program installed or removed, Windows update, created manually).
- **Create a restore point** with your own description at any time. Windows normally allows only one a day; InstallScope lifts that limit for its own request and puts it back afterwards.
- **Roll Windows back** to any restore point, with a clear explanation of what changes (system files, settings, and programs) and what doesn't (your documents and photos), or open Windows' own System Restore wizard.
- **Advanced mode:** delete individual restore points, turn System Protection on or off, and set how much disk space restore points may use.

### Snapshots

<img src="docs/screenshots/snapshots.png" alt="Snapshots" width="720">

Capture the system at any moment and compare any two captures.

- **Quick before and after:** click once before you run, use, or install something, and once afterwards, to see exactly what it changed or left behind.
- **Named snapshots** saved to disk, so you can compare any two, even days apart.
- **Turn the differences into an install log**, so they can be undone like a monitored installation, or export them as **PortableApps.com Launcher settings** (`[RegistryKeys]`, `[DirectoriesMove]`, `[FilesMove]`, and the matching clean-up sections, with paths written as Launcher variables such as `%APPDATA%`).

### PortableApps Forge

<img src="docs/screenshots/portable-builder.png" alt="PortableApps Forge" width="720">

Turn a program's installer into a PortableApps.com Format package.

- **Choose the installer** (`.exe` or `.msi`) and describe the program: name, version, publisher, category, description, homepage, and license.
- **Isolated capture in Windows Sandbox** (Windows 11 Pro, Enterprise, and Education): the installation happens in a throwaway copy of Windows, so your PC is never touched. On other editions it runs on your PC and is recorded, so the program can be removed precisely afterwards.
- **Clean results:** InstallScope first watches Windows for 30 seconds to learn what changes on its own, then runs the installer (silently for MSI, NSIS, and Inno Setup installers if you like) and lets you start the program once so its first-run settings are captured.
- **A complete package:** the program's files in `App\AppName`, `appinfo.ini`, icons taken from the program, `help.html`, and Launcher settings made from the comparison, with shortcuts, installer bookkeeping, and the program's own folder left out. The settings created on first run can be included as the package's defaults.
- **Captured associations:** candidate file extensions and URL protocols registered during capture are included in `appinfo.ini` so the PortableApps.com Platform can offer them for the portable version. Command-line arguments are not inferred; review apps that need custom handling before publishing.
- **Optional size optimization:** when the official [PortableApps.com AppCompactor](https://portableapps.com/apps/utilities/portableapps.com_appcompactor) is installed in the configured PortableApps folder, open it from the completed-package view and select the package to compress. Compression is manual; test the resulting package because not every program can be safely compressed.
- **Portability check:** before you share the package, InstallScope tells you what could stop the program from working portably, and adjusts the package where it can:
  - **Java:** programs written in Java that don't bring their own Java runtime are marked as needing Java, so the PortableApps.com Launcher uses jPortable or an installed Java.
  - **.NET:** the .NET Framework is part of Windows 11, but programs built for newer .NET versions need the matching runtime on every PC unless they include it; InstallScope names the version.
  - **Visual C++ runtimes** the installer put into Windows are copied next to the program, so the portable version doesn't need the Redistributable.
  - **Things that can't travel:** services, drivers, Explorer extensions and COM components, files written into the Windows folder, and programs that always ask for administrator rights.
  - **64-bit-only programs** are marked as such in the package.
- **Nothing else to install:** InstallScope compiles the launcher with the official PortableApps.com Launcher source and the NSIS compiler that comes with it, exactly as the PortableApps.com Launcher Generator does. Release downloads include it; otherwise it is downloaded once from GitHub (about 6 MB) and checked against a pinned SHA-256 fingerprint. The `.paf.exe` installer is built with InstallScope's own installer script, which installs packages like PortableApps.com installers do (into the PortableApps folder, keeping your settings on upgrades, with the Platform's `/DESTINATION=` option and silent installs). If the official PortableApps.com Installer is installed, InstallScope uses it instead, which is what portableapps.com expects for published apps.
- **Compile again with one click** after editing the Launcher settings.

### Bloatware Shield

<img src="docs/screenshots/bloatware-shield.png" alt="Bloatware Shield" width="720">

Remove what came with the PC but you never asked for.

- **One scan, one click:** antivirus trials, sponsored games, VPN and storage offers, manufacturer welcome and promotion apps from Dell, HP, Lenovo, Acer, ASUS, MSI, Samsung and Dynabook, and the Store apps Windows adds for consumers, including the ones ordinary uninstallers skip.
- **Recommended and optional:** trials, ads and promotions are ticked; maker tools some people use, such as support assistants, are listed but left unticked. Drivers, audio and hotkey tools and Windows' own parts are never listed.
- **Also for new accounts:** stop Windows from installing the removed Store apps for user accounts created later.
- **Reviewed and reversible:** programs are uninstalled one after another and their leftovers go through the usual review, with everything backed up.

### Right-click menu

<img src="docs/screenshots/right-click-menu.png" alt="Right-click menu manager" width="720">

Tidy the menu that appears when you right-click files, folders, drives and the desktop.

- **Every entry programs added:** shell extensions and menu commands, where each appears, and which file provides it. Windows' own entries are hidden unless you ask for them.
- **Turn entries off** with a checkbox, the way Windows itself does, so nothing is deleted and each one can be turned back on.
- **Broken entries** left by removed programs are flagged and can be removed in one go; they slow File Explorer down and can make it show errors.
- **Deep clean:** also finds COM classes, "Open with" entries, and `PATH` and environment entries that removed programs left anywhere in Windows.

### Ask InstallScope

Press **Ctrl+K** (or the **Ask InstallScope** button) and ask in your own words: *"find and wipe everything related to that video converter I downloaded last week"*, *"top 5 space-wasters I haven't opened in 6 months"*, *"games installed this year bigger than 10 GB"*. InstallScope shows how it understood the question, lists matching programs and install logs as you type, and sends removals through the usual review. It all runs on your PC; nothing is sent anywhere. Questions are understood in English.

### Settings and accessibility

- **Simple and Advanced modes**, chosen on first launch. Simple mode uses plain language and offers only options that cannot harm Windows or slow the PC; Advanced mode shows every tool and technical detail.
- **Safety options:** backups before removal, confirmations, restore points, and Recycle Bin copies (always on in Simple mode).
- **Monitoring scope and profiles:** which registry keys and folders snapshots cover, with exclusions and background-noise learning, saved as named profiles.
- **Explorer integration:** the **Install with InstallScope** entry in the right-click menu of installers.
- **Full keyboard access:** a visible focus ring, arrow-key navigation in lists, Esc to close dialogs, and shortcuts for every page (Ctrl+1 to Ctrl+9, Ctrl+Tab, Ctrl+F, Ctrl+K, F5, Delete, and F1).
- **Screen-reader support** for Narrator and NVDA, adjustable text size, and a high-contrast option.
- **Eleven languages:** English, Español, Deutsch, Français, Português (Brasil), Italiano, Polski, Русский, 简体中文, 日本語, and 한국어. Additional languages can be added without recompiling (see [Translating](docs/TRANSLATING.md)).

### Design and distribution

- **Built for Windows 11:** a dark interface with a native red Windows 11 window border, and an administrator check at every launch with a one-click relaunch as administrator.
- **Fast and fluid:** a native GPU-rendered interface; slow work (scans, sizes, signatures, usage) runs in the background so the window never freezes. Pages fade in, and hover and selection animate softly; animations follow Windows' own *Animation effects* setting.
- **No dependencies:** a single native executable, with no .NET, no Visual C++ runtime, and nothing else to download.
- **Installer or portable**, for x64 and Arm64 PCs.
- **Full command line** for scripting and automation.

## Download

Download the latest version from the [Releases page](https://github.com/SeanShaffer88/InstallScope/releases/latest).

| File | Description |
|---|---|
| `InstallScope-Setup-x.y.z.exe` | Installer: installs to Program Files and adds a Start menu shortcut and an *Installed apps* entry. |
| `InstallScope-x.y.z-portable.zip` | Portable: unzip anywhere; all data is stored next to the executable. |
| `InstallScope-x.y.z-arm64-portable.zip` | Portable build for Windows 11 on Arm PCs. |

**System requirements:** 64-bit Windows 11 (build 22000 or later), on x64 or Arm64. InstallScope is designed exclusively for Windows 11 and will not start on earlier versions of Windows. Administrator rights are required for full functionality; InstallScope offers to relaunch itself with elevated rights. PortableApps Forge's sandbox needs the *Windows Sandbox* feature, which is part of Windows 11 Pro, Enterprise, and Education.

To preserve data created by earlier Expunge builds, installed logs, backups, settings, and snapshots remain in `%LOCALAPPDATA%\Expunge`; portable builds continue to use `ExpungeData` beside the executable. Existing Task Scheduler identifiers and the uninstall registry key also retain their `Expunge` names for compatibility. Program-transfer packages keep the `.expunge` extension and remain supported.

**"Windows protected your PC":** InstallScope's downloads are not yet code-signed, and new programs have no download history with Microsoft, so Windows SmartScreen may show this warning the first time you run it. Click **More info**, check that the file name is right, then **Run anyway**. To be sure the file is the one GitHub Actions built from the public source, compare its SHA-256 checksum with the one published next to the download (in PowerShell: `Get-FileHash .\InstallScope-Setup-x.y.z.exe`). Some antivirus programs are wary of any tool that watches system files and stops programs; if yours flags InstallScope, please [open an issue](https://github.com/SeanShaffer88/InstallScope/issues) so it can be reported as a false positive.

## Getting started

1. Select **Monitor → Choose installer…** and pick a setup file. Wait for the "before" snapshot to finish.
2. Install the program as usual. Optionally, run it once so that its first-run settings are captured as well.
3. Click **Finish monitoring**, review the changes, and click **Save install log**.
4. When you want to remove the program, go to **Programs** (or **Install logs**) and click **Uninstall**. Review the plan, then confirm. If you change your mind, use **Backups → Restore**.

To make a portable app instead, open **PortableApps Forge**, choose the installer, fill in the program's name, and click **Start**.

## Command line

```
installscope monitor --name "Photo Editor" -- C:\Downloads\setup.exe
installscope logs [--group G] [--tag T] [--json]
installscope revert <log-id> --dry-run
installscope revert <log-id> --close-programs --restart-explorer
installscope components <log-id> [--json] [--ask-model] [--strike]
installscope export <log-id> xml changes.xml
installscope export <log-id> portable PhotoEditorPortable.ini
installscope repackage <log-id> D:\Deploy\PhotoEditor
installscope merge <log-id> <log-id> --name "Photo Editor"
installscope snapshot before.snap
installscope diff before.snap after.snap --name "Photo Editor"
installscope backups
installscope restore <backup-id>
installscope restore-point create "Before installing drivers"
installscope restore-point list
installscope learn-noise [--seconds N] [--apply]
installscope add-program D:\Tools\PortableEditor
installscope uninstall "Photo Editor" "Old Toolbar" --silent
installscope orphans
installscope clean --dry-run
installscope schedule-clean weekly
installscope startup disable Spotify
installscope transfer export moved.expunge "Photo Editor"
installscope transfer import moved.expunge
installscope portable-build C:\Downloads\setup.exe --name "Photo Editor" --silent --out D:\PortableApps
installscope --help
```

Any command that takes snapshots accepts `--profile NAME` to use a saved monitoring profile.

Installer quality checks in a build pipeline:

```
installscope monitor --name MyApp --no-prompt --json -- dist\MyApp-Setup.exe /S
installscope revert <log-id> --keep-log
installscope check-clean <log-id>            # exits with code 2 if anything was left behind
```

## Building from source

```
git clone https://github.com/SeanShaffer88/InstallScope
cd InstallScope
cargo build --release
```

InstallScope builds with stable Rust on Windows. If Visual Studio is not installed, push to GitHub and let the included Actions workflow build it, or cross-compile from Linux (see `scripts/cross-env.sh`). Releases are built automatically when a `v*` tag is pushed.

The first native Win32 Programs view is available as an experimental preview.
Its dark/red app shell and sidebar follow the product screenshots; Programs is
the only migrated page, and the other sidebar destinations are visual
placeholders while their workflows remain in the regular interface:

```
cargo run --features native-ui-preview
```

It supports browsing, search, program details, **Add program…** for analysis-only
entries, multi-select batch uninstall, and **Find by window**. A manual entry
can be removed from InstallScope's list without touching its files. The Utilities
menu also runs the same reviewed, backup-aware system-wide leftovers scan as
the regular interface, including the conservative stale uninstall registration
checks.
**Uninstall** starts the vendor uninstaller for registered desktop programs or
asks Windows to remove a Store app for the current user. After the operation
succeeds, InstallScope presents leftovers in a risk-scored checklist before removal;
cleanup uses InstallScope's backup and protected-target checks. Batch uninstall
runs one program at a time and pauses for a separate reviewed leftover
checklist after each. The window picker requires a unique executable
path/install-location match and never starts an uninstall. The regular build
continues to use the complete interface.

The core engine (`src/`, excluding `gui/`) is fully unit-tested against an in-memory registry: `cargo test --lib`. Real installers are also installed, removed, checked, and turned into portable packages on clean Windows runners by the Integration workflow.

## How it works

| Stage | What happens |
|---|---|
| Snapshot | Registry keys and values (raw data of every type) and file metadata are recorded under configurable locations, skipping known Windows noise. |
| Comparison | Added, removed, and changed keys, values, and files are identified, compared case-insensitively as Windows does. |
| Plan | Services and tasks are handled first, then files, then folders (deepest first), then registry deletions, and finally restorations. |
| Execution | Every step is backed up first; protected locations are refused; locked files are scheduled for deletion at restart. |
| Undo | Backups are replayed in reverse order to restore everything. |

## Why you can feel safe using InstallScope

A lot of work has gone into making InstallScope careful, thorough, and fast. Here is what has been done.

**Every change can be checked and undone**
- Supported file and registry changes InstallScope applies after review are backed up first and can be restored from the Backups page.
- Registry backups are also saved as standard `.reg` files, so they can be restored by double-clicking, even without InstallScope.
- A System Restore point can be created before uninstalling, and the System Restore page lets you create one, or roll back to one, at any time.
- Startup changes work the way Task Manager's do (the entry stays and can be switched back on), and delayed starts can be undone with one click.
- Removed files can also go to the Recycle Bin, and Windows asks before permanently deleting anything too large for it.
- You see a full plan before anything is removed, with every item labeled *Safe*, *Review*, or *Careful*, and you confirm it first. The command line offers `--dry-run` to preview changes.
- PortableApps Forge can install programs inside Windows Sandbox, so nothing on your PC changes at all.

**Built-in protection**
- Critical Windows folders and registry keys are hard-coded as protected and can never be deleted, whatever an install log or scan says. Paths are compared the way Windows resolves them, so tricks such as `..`, trailing dots, `\\?\` prefixes, short names and junctions can't get around the protection, and the registry layer itself refuses to delete protected keys whoever asks. Tests try every critical location in many spellings.
- A link (junction or symbolic link) inside a folder being removed is removed on its own; what it points to is never touched.
- Removals are crash-safe: each change is recorded in its backup, flushed to disk, before it is made. If a removal is cut off by a crash or power cut, InstallScope says so the next time it starts and offers to undo what was done.
- Registry entries that belong to Windows itself (its security components, app packages, and Task Scheduler bookkeeping) are recognized and left alone, even if Windows changed them while an installation was being monitored.
- Simple mode hides every option that could harm Windows or slow the PC, and keeps backups and confirmations switched on.
- Startup entries that Windows, security software, or hardware need can't be switched off in Simple mode.
- The junk cleaner only touches well-known temporary and cache locations, never their parent folders, and skips files that are in use.
- InstallScope checks at every launch that it is running on Windows 11, and tells you when it needs administrator rights instead of failing silently.

**Private by design**
- InstallScope has no telemetry, no accounts, and no ads, and it never sends information about you or your PC anywhere.
- It is open source under the GPLv3, so anyone can read exactly what it does.

**Tested at every step**
- Over 130 automated tests run on every change, including stress tests with very deep registry trees, thousands of files, paths over 260 characters and files held open by other programs, on both Windows and Linux, together with static code analysis (Clippy). On Windows they also use real locks: a file open in another program, a DLL loaded by another process, a running program inside the folder being removed, and a real Windows service that keeps its files open until its process has exited.
- Integration tests install real installers (Windows Installer, NSIS, Inno Setup, Squirrel and a WiX Burn bundle, plus InstallShield and Wise when a test installer is configured) on clean Windows machines, remove them with InstallScope, and verify that nothing is left behind; the same installers are also turned into PortableApps.com packages and checked. These tests have already found and fixed real problems before release.
- A weekly soak test installs and removes programs over and over for hours with background monitoring running, and fails if anything is left behind, if background monitoring misfires, or if its memory or handle use keeps growing. `scripts/soak.ps1` runs the same test for days on a test machine.
- Performance is measured automatically: snapshot speed and memory, time to open the window, and CPU and memory use while idle, each against a fixed budget; a check fails if any budget is exceeded.
- The Arm64 version is built and checked on every change, and translations are checked automatically for missing placeholders.
- Releases are built by GitHub Actions directly from the public source code, and every download is published with a SHA-256 checksum so you can verify it.

**Fast and light**
- InstallScope is a single native program written in Rust, with no runtime to install and no just-in-time compilation, built with full link-time optimization.
- Heavy work (snapshots, scans, signature checks, measurements) runs in the background at low priority, so the window never freezes, and folders are scanned in parallel.
- Results that are expensive to compute are cached, and the window is only redrawn when something changes, so InstallScope uses no CPU while it sits idle.
- Background monitoring, when you turn it on, runs at low priority so it never competes with your work.

**Made for everyone**
- Plain-language help on every page, full keyboard access, screen-reader support for Narrator and NVDA, adjustable text size, high contrast, and eleven languages.

## Use with care

InstallScope has been extensively tested, both with automated tests and with real installers, but like any software that changes Windows it may still contain bugs. Many safety measures are built in: Simple mode hides anything that could harm Windows or slow the PC, critical folders and registry keys are protected and can never be deleted, every removal item is labeled by risk, and you review and confirm changes before they happen. Even so, please use InstallScope with caution, and stay in Simple mode unless you know what you are doing.

By default, supported file and registry changes InstallScope makes during reviewed removal, cleanup, startup, and shell-menu workflows are backed up and can be restored from the **Backups** page (registry entries are also exported as `.reg` files). Simple mode always keeps backups on; Advanced mode lets you disable them, in which case those changes are not recoverable from InstallScope. If backup creation fails while backups are enabled, InstallScope stops before applying the change; backup-finalization errors are reported.

An uninstaller's own removal is performed by that vendor's program (or Windows for Store apps), so InstallScope cannot promise that it can restore the application itself; its backups cover the subsequent changes InstallScope applies after your review. A System Restore point is an optional additional recovery measure, not a guaranteed undo. When a requested restore point cannot be created, InstallScope cancels the uninstall or deep cleanup instead of proceeding as though it succeeded. Permanently deleting temporary files, deleting a restore point, and Windows deep-cleanup actions remain explicitly identified as irreversible and separately confirmed.

## Roadmap

InstallScope already includes [PortableApps Forge](#portableapps-forge),
[System Restore manager](#system-restore), [Snapshots](#snapshots), installer
preflight checks, and Windows Sandbox-assisted installer preview/package
capture on supported editions. The visible **Create Setup Package…** action
builds a silent redeployment package from a captured install log; it is not a
general-purpose interactive installer authoring wizard. The planned wizard is
intended to inventory and bundle all application-owned resources, runtimes,
libraries, and dependencies available in its selected inputs, and to stop on
unresolved required dependencies instead of silently creating an incomplete
package. Windows-owned binaries and dependencies that cannot be redistributed
must be identified as explicit system or prerequisite requirements, not copied
into the package.

The separate in-app Disposable Runner, read-only VSS content browser,
temporary scratch volume, reviewed WinGet project-setup export, reversible
per-project environment profiles, Dev Drive guidance, and optional handoff to
installed Sysinternals tools remain planned—not available features. The
Utilities sidebar shows the Developer Sandbox, Temporary Scratch Disk, VSS
Viewer, and general-purpose Setup Package Creator wizard as disabled **Planned**
entries. The existing log-specific silent setup-package action remains
available. The Disposable Runner is intended to use restricted-token and Job
Object containment; it must not be presented as a virtual machine or a stronger
security boundary than its implementation. See [ROADMAP.md](ROADMAP.md) for
scope and safety boundaries. Ideas and pull requests are welcome; see
[CONTRIBUTING.md](CONTRIBUTING.md) for guidelines.

## License

Copyright © 2026 Sean Shaffer.

InstallScope is free software: you can redistribute it and/or modify it under the terms of the **GNU General Public License, version 3** (or, at your option, any later version). See [LICENSE](LICENSE) for details.

The release downloads include the [PortableApps.com Launcher](https://github.com/PortableApps/Launcher) source and the NSIS compiler distributed with it, under their own licenses (GPLv2 and the zlib/libpng license); they are separate programs that InstallScope runs, unchanged.

Total Uninstall is a trademark of its owner. InstallScope is an independent project and is not affiliated with Total Uninstall or its developer.
