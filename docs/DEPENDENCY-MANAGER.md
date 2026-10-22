# Developer Dependencies

Open **Project Readiness → Dependencies**. The manager shares that tool's window; it does not add another Workflow Board tool. Pinned downloads require an explicit action; separately reviewed vendor setup can perform its own networking. Other diagnostics remain read-only.

## Native Windows Toolchain

Expand **MSVC / Windows SDK / CMake / Ninja — Vendor Setup**. Use the official Build Tools 2022 release-history link to download `vs_buildtools.exe` for the intended supported channel, then select that local file. SetupScope does not silently download/refresh it. Its observed SHA-256 and cached Microsoft Authenticode organization are reviewed; this hash is **not pinned upstream**, unlike the full-installer catalog. A signature does not establish product/channel/license eligibility: inspect file properties and Microsoft's visible setup UI.

Choose Install, Create/Update Offline Layout, Verify Offline Layout, or Install From Offline Layout. Offline installation requires the bootstrapper in the layout root and requests `--noWeb`. MSVC x64/x86 is selected; Windows SDK 26100, CMake/Ninja, ARM64 target tools and recommended components are configurable. Required dependencies remain vendor-controlled. Component IDs target Build Tools 2022; other versions require vendor confirmation. Layout languages: English, German, Spanish, French, Japanese and Simplified Chinese.

Prepare a publisher-verified review, inspect command/components/privileges/recovery, acknowledge it, then launch. Hash/signature are rechecked and a durable receipt is mandatory. Only one vendor operation can run at a time in SetupScope. Finish/cancel its UI yourself. No silent switches, automatic license acceptance, certificate import or rollback. Installation can change shared components, PATH, registry and Program Files. Recovery uses Visual Studio Installer Modify/Repair/Uninstall. Detached work may outlive SetupScope.

Layouts can consume several GiB, update the shared installer and overwrite existing layout content. Back up before updating. SetupScope does not estimate total size, freeze channel versions or certify complete air-gapped setup. Verification uses vendor `--verify`; offline installation is a request, not a network sandbox. Prerequisite certificates/installer updates may still be needed. Transfer complete vendor-managed layout folders separately: they are **not included in pinned bundle import/export**.

Sources checked 2026-10-22: [component IDs](https://learn.microsoft.com/en-us/visualstudio/install/workload-component-id-vs-build-tools?view=vs-2022), [channels/downloads](https://learn.microsoft.com/en-us/visualstudio/releases/2022/release-history), [offline layouts](https://learn.microsoft.com/en-us/visualstudio/install/create-an-offline-installation-of-visual-studio), [parameters](https://learn.microsoft.com/en-us/visualstudio/install/use-command-line-parameters-to-install-visual-studio?view=vs-2022). Real installation/UAC, multi-GiB layout creation and offline installation have not been executed on this PC.

## Pinned catalog

| Package | Version | Architectures | Included scope |
| --- | --- | --- | --- |
| .NET SDK | 10.0.402 | x64, ARM64 | Full vendor offline installer; includes Desktop Runtime. |
| .NET Desktop Runtime | 10.0.13 | x64, ARM64 | Full vendor offline runtime installer. |
| CMake | 4.4.4 | x64, ARM64 | Full vendor MSI; build configuration only. |

The catalog snapshot is dated 2026-10-21. All six hashes were rechecked against official metadata on 2026-10-22. This is a fixed catalog, not an automatic update service. New versions require a reviewed application/catalog update. Project packages, NuGet restores, workloads, MSVC, Windows SDK, Ninja and other transitive prerequisites are excluded. An offline bundle is not a complete air-gapped development environment. SDK presets avoid a redundant runtime installer; Custom permits explicit combinations.

Official sources: [.NET release metadata](https://builds.dotnet.microsoft.com/dotnet/release-metadata/10.0/releases.json), [.NET installation options](https://learn.microsoft.com/en-us/dotnet/core/install/windows), [CMake downloads](https://cmake.org/download/), [CMake hashes](https://github.com/Kitware/CMake/releases/download/v4.4.4/cmake-4.4.4-SHA-256.txt).

## Download and verify

1. Choose architecture and a preset, or select packages individually. Details expose the exact version, vendor URL, digest, publisher, license link and exclusions.
2. Optionally choose a local Library Root. Downloads use `dependencies/downloads`; receipts use `dependencies/receipts`. Select a short, absolute local folder. Network, device, alternate-stream and reparse paths are rejected.
3. **Download Selected** uses Windows WinHTTP, normal OS TLS/proxy configuration, fixed official hosts and at most five approved HTTPS redirects. Cookies and automatic authentication are disabled. No login or credential collection is implemented.
4. Exact size and SHA-256/SHA-512 must match the compiled catalog before an installer is atomically published. Failed/cancelled staging files are discarded. Existing cache files are reverified, never silently overwritten. Invalid cached files need manual inspection or a clean library folder.
5. Authenticode must validate against cached Windows trust, and the embedded signing certificate organization must match the expected publisher. Certificate display names can differ from organizations. No online revocation lookup is performed; an unavailable trust chain blocks use. This is not proof of vendor safety or future revocation status.

Transfers have bounded calls (up to 30-second connect/read waits) and a 30-minute per-file limit. Cancellation is checked between operations; it cannot interrupt an OS call instantly. Hashing and copying stream in 64 KiB chunks, with a 512 MiB per-installer cap. Progress measures bytes processed across download/hash/copy stages, not a percentage or network-only total. Sufficient disk space is required for staging and bundle copies; disk-full errors preserve existing files.

## Offline bundles

Use a new destination folder for export. Review vendor licensing/redistribution links and acknowledge terms for the exact selection and destination. The source project's license alone does not establish every installer's redistribution rights; vendor installation terms still apply.

The bundle contains only the selected fixed filenames and a bounded `bundle.json` with schema, catalog revision and package IDs. The manifest is written last. An interrupted export can leave an incomplete folder; it is not an importable bundle without its manifest. Existing destinations are never replaced. Choose a new folder after investigating an incomplete export.

Import reads at most 32 KiB of manifest, rejects duplicate/unknown fields, unknown packages, mismatched catalog revisions and traversal IDs, and verifies all source entries before cache copying. External URLs, digests, commands and arbitrary archive extraction are not supported. Each copied installer is verified again; complete cache entries can remain after later cancellation/failure. Source bundles and existing cached files are preserved. Files omitted from the manifest are not used.

## Reviewed installation and recovery

Select exactly one package, choose whether to request Administrator/UAC, then **Prepare Exact Install Review**. This holds the installer against writes/deletion and ancestor paths against rename during review and launch. Discard Review releases the held file. These reduce races; they are not a security boundary against other processes running as your user, administrators or kernel attackers.

The review shows the exact path, version, architecture, pinned digest, publisher, fixed command, privilege request, vendor terms and system/recovery effects. Changing selection, architecture, library root or privilege request clears review. A new review must be acknowledged before launch. Hash and signature are rechecked immediately before installation; a durable prepared receipt is mandatory before the launch call.

- .NET: the selected EXE with `/install /norestart`.
- CMake: the absolute Windows system `msiexec.exe` with `/i "selected.msi" /norestart`.

No arbitrary arguments or quiet/silent switches are accepted. Use the vendor UI to configure its supported options and licensing. Vendors may request elevation themselves even when SetupScope did not request it. ARM64 launch is blocked on x64 hosts; x64 on ARM64 can require emulation. Architecture selection is not proof of runtime/build compatibility.

Installation can change Program Files, shared runtimes, registry and PATH, remove older vendor versions and require a reboot. `/norestart` requests no automatic reboot; vendor/OS behavior remains outside SetupScope's control. SetupScope tracks only the launched process; detached children may continue. Closing SetupScope does not cancel, roll back or undo the installer. Finish/cancel the vendor UI yourself. If tracking is interrupted, a prepared/pending receipt remains; inspect vendor state before retrying.

Receipts record exact package, path, hash, command, elevation request, time, launched PID and available exit status. Success, reboot-required (3010), reboot-initiated (1641), user cancellation (1602), other errors and unknown completion remain distinct. Exit success does not prove installed state or build readiness; reinspect the existing toolchain diagnostics separately. Receipt refresh is bounded to 256 directory entries and the 20 newest named records; malformed/oversized records report an error rather than disappearing silently. Receipts contain local paths and are not diagnostic exports.

Recovery uses the vendor uninstaller or Windows Installed Apps. Automatic rollback is unavailable; uninstalling a shared runtime can affect other projects and may not restore the earlier state. SetupScope does not run an automatic uninstall, repair or restore-point operation.

## Validation scope

Tests cover catalog identity/URL limits, digest vectors/chunk boundaries, changed/truncated bytes, manifest rejection, missing reviews/signatures/receipts, cancellation, vendor decline, reboot receipts and Windows file/parent locking. Installer execution uses a mock process in tests. Real x64 CMake and .NET SDK downloads passed pinned verification, offline export/import and review preparation without executing installers. ARM64 and real vendor installation/UAC/recovery remain unverified on this PC.
