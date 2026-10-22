# Installer and archive support

Local verification: Windows 11 x64, 2026-10-21. This matrix describes specific
workflows, not a promise to handle every file bearing an extension. No vendor
installers were executed on the owner's PC for this milestone.

## Verification tiers

- **Verified component:** local automated fixtures passed for the named behavior.
- **Partial:** code paths exist, but a real installation/removal on an isolated
  Windows client has not been verified in this milestone.
- **Unsupported:** no workflow is provided, or the reader deliberately refuses it.

## Installers

| Input / engine | Available workflow | Evidence and limits |
|---|---|---|
| MSI | Monitor; PortableApps Forge capture; MSI uninstall commands | Verified component: quoted install commands, optional quiet/no-restart switches, uninstall command tests. MSI recognition uses the extension, not database validation. Real client install: partial. |
| NSIS EXE | Monitor; Forge capture; recognized silent switches | Verified component: bounded signature hints and command construction. A matching string is heuristic, not authenticity or validity evidence. Real client install: partial. |
| Inno Setup EXE | Monitor; Forge capture; recognized silent switches | Verified component: string hints and command construction. Real client install: partial. |
| Other EXE, including wrappers and web installers | Interactive monitored execution; vendor uninstall information when present | Partial. Unknown engines receive no guessed quiet-install switches. Downloaded payloads and child installers may differ from the screened file. |
| WiX Burn / Squirrel uninstall records | Existing engine-specific uninstall command construction | Verified component: command tests. This does not provide a general installer parser or reconstruct the installer. Real client uninstall: partial. |
| MSIX / APPX | Monitor's Windows package installation path; Store-app removal | Partial. Requires Windows package policy, signing and dependency requirements. Not a Forge capture-format guarantee. |
| BAT / CMD | Explicitly selected monitored scripts | Partial. Scripts execute their contents with the user's rights; screening cannot establish safety. These are not archives or an isolated runner. |

The integration workflow contains additional installer scenarios. Its presence
does not prove those scenarios ran successfully on this PC or on a clean Windows
client. ARM64, elevation-dependent behavior, repair/reboot behavior and all vendor
versions require separate testing. Snapshot monitoring can miss activity outside
its scope; an install log does not prove arbitrary software can be repackaged.

## Archives and packages

| Format / feature | Tier | Scope |
|---|---|---|
| ZIP stored (method 0), Deflate (8) | Verified component | Internal reader: independent Python-generated fixtures, CRC/size checks, UTF-8 and CP437 names, directory entries. Not a general archive-extraction UI. |
| Single-disk ZIP64 | Verified component | Small fixtures exercise 64-bit metadata and round trips. Multi-gigabyte payloads and million-entry archives remain unverified. |
| SetupScope `.setupscope` | Verified component | ZIP container plus SetupScope manifest and existing transfer tests. Arbitrary ZIP files do not become valid transfer packages by renaming them. Restore can change files/registry and follows the existing review/backup workflow. |
| PortableApps Forge source ZIP | Verified component | Internal toolchain download extraction, not arbitrary archive imports. Unsafe Windows names fail before output is created. Only approved, pinned toolchain sources should be used. |
| Encrypted ZIP / split or multi-disk ZIP | Unsupported | Rejected. Unsupported compression methods are rejected when the entry is opened; directories may still be listed. |
| RAR, 7z, TAR, gzip, CAB, ISO and other archive families | Unsupported as generic archive imports | No general extractor is provided. An EXE wrapper may run as an installer, which does not imply its archive payload is understood. |

The ZIP central directory is limited to 64 MiB. The reader refuses out-of-bounds
directories/data, malformed ZIP64 fields, conflicting local/central names or
compression methods, exact duplicate entry names and invalid indexes. CRC and
expanded-size checks require reading an entry through EOF. Names intended for
extraction reject traversal, absolute/device paths, Windows reserved devices,
alternate streams, control characters and trailing dots/spaces.

These checks are not a security boundary or a full decompression-bomb policy.
No aggregate expanded-byte, disk-space or CPU budget is promised. A corrupt later
entry can fail after earlier output was written; do not assume transactional
extraction. Preexisting filesystem links and concurrent path changes require
separate protection. Case aliases and Unicode normalization are not globally
deduplicated. Keep backups and use the intended managed toolchain/transfer flows.

## Reproduce

From the project directory, run `scripts\local.ps1 test` (full suite), or
`cargo test --lib zip::tests --locked` for archive fixtures. Relevant tests also
live in `patools`, `pafbuild`, `silent` and `transfer`. Tests create disposable
fixtures; they do not run the fixture installers or install vendor software.

Format references: [PKWARE ZIP specification](https://pkware.cachefly.net/webdocs/casestudies/APPNOTE.TXT)
and [Microsoft Windows filename rules](https://learn.microsoft.com/en-us/windows/win32/fileio/naming-a-file).
