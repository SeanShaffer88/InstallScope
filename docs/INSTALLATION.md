# Install or use portably

Both downloads contain the same Windows 11 x64 application and console executable.
Both are unsigned. Native ARM64, clean-VM and broad Windows-build acceptance remain
unverified. No reboot, account, service, startup entry or PATH modification is required.

## Portable ZIP

Extract the ZIP into a writable folder and launch SetupScope.exe. The included
`portable` marker keeps settings, profiles and recovery evidence in the adjacent
`SetupScopeData` folder. Keep that folder when updating; close the app first and
replace application files. Never overwrite saved data with another person's copy.

## Installer

Run SetupScope-Setup-1.0.0.0-win-x64.exe. Installation is for the current user,
without administrator elevation, by default under `%LOCALAPPDATA%\Programs\SetupScope`.
Choose an empty folder or an existing folder owned by this installer. Existing
portable folders are rejected. Start Menu shortcuts are selectable; a desktop
shortcut is optional and off by default. Setup does not launch the app automatically.

Installed-mode settings and evidence live separately in `%LOCALAPPDATA%\SetupScope`.
Installing does not migrate portable data. Close all app/tool windows before
updating or uninstalling. A running-app guard blocks changes while the GUI is open;
files used by another process can still require closing that process and retrying.
An interrupted install may leave an installer-owned partial folder: rerun setup in
that folder. Setup is not a transactional Windows Installer package.

Remove SetupScope through Windows Installed Apps or its Start Menu uninstaller.
Uninstall removes the explicitly packaged files, shortcuts and current-user Apps
registration. It never recursively deletes the destination or removes saved
settings, receipts, profiles, recovery evidence or unknown files. A folder left
behind can contain preserved files; inspect it manually. Uninstalling SetupScope
does not undo environment edits or uninstall separately selected developer tools.

`/S` is supported for unattended setup/uninstall. NSIS `/D=absolute-path` must be
the final installer argument. Existing portable folders and unrelated nonempty
folders are refused even in silent mode. Exit 0 means completion; exit 2 means a
platform, ownership or running-app guard refused the operation; other nonzero
codes indicate failure. Installer standard pages use NSIS language resources;
additional safety messages are currently English.

## Build and verification

Compile the application and run `scripts/package-local.ps1`, then ZIP the portable
payload without any SetupScopeData folder. Build with:

```powershell
python scripts/package-installer.py portable.zip path/to/makensis.exe output.exe
```

The builder verifies each portable payload checksum, stages only manifest-listed
files, omits the portable marker and generates explicit installation/removal lists.
The installed payload receives a matching checksum manifest. The compiler is a
build dependency, not a dependency installed on users' PCs.

The local installer compiler is NSIS 3.13, from the [official download](https://sourceforge.net/projects/nsis/files/NSIS%203/3.13/nsis-3.13.zip/download),
SHA-256 `ba63dffc4410ee89193e1cb5a41989991bd77c61068da17e3156d136b7b0b3d8`.
The [official release notes](https://nsis.sourceforge.io/Docs/AppendixF.html) describe
its maintenance and security history. Preserve NSIS's bundled licensing notices.
Current-host install, same-version upgrade, loader, running-app guards and uninstall
checks are separate from clean-VM, interactive-page and signed-package acceptance.
