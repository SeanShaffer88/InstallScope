# Guided PortableApps.com package projects

Status: **planning and configuration-preview foundation, not a complete package builder**.
The regular PortableApps Forge page now offers **Guided PAF project** and the
existing **Installation capture** workflow separately. This new workflow prepares
projects for the official PortableApps.com tools; it introduces no package format.

## Available in this revision

- Ten planning screens: welcome/workspace, source files, identity, portability,
  launcher, icons, installer, licenses/components, review, and manual test records.
- Save incomplete answers and the current step to a new JSON snapshot; open it to
  resume. Snapshot files are InstallScope records and do not belong inside PAF.
- Existing snapshot files are never overwritten. Use a new filename per revision.
  A failed write can leave an incomplete new snapshot; its error reports that
  location. Earlier snapshots and source files remain intact. Successful saves
  flush the snapshot file; no claim of atomic publication or power-loss durability
  is made. Opening a corrupt snapshot preserves the in-memory project.
- Unsaved answers require confirmation before opening another project or closing
  the regular application. Closing waits for an in-progress project save/read.
- Explicit unanswered/yes/no license declarations. No permission, publisher,
  homepage, language, category or version is invented. Component records track
  names, licenses, notice/evidence locations and redistribution review.
- Local metadata/launcher draft previews after relevant findings are resolved.
  No installer.ini is generated for defaults. Drafts are not written into source
  folders, and no official Installer log/receipt is fabricated.
- Findings distinguish official format rules, official tool behavior and wizard
  safeguards; relevant findings link to their reference. Navigation can continue
  with unresolved answers without marking them passed.
- Background path checks identify missing/inaccessible files and source/workspace
  overlap. They do not execute tools or establish authenticity. Results are marked
  stale when answers change. Opening and saving also run outside the UI thread.

The JSON reader accepts at most 1 MiB, rejects malformed JSON, unsupported schema
versions, invalid steps and unknown project/component fields rather than silently
losing them. Draft previews are limited to 64 KiB. These are wizard limits, not
PAF format rules. Projects contain local paths and possibly private notes; review
snapshots before sharing them. No telemetry or automatic downloads are added.

## Official references and rule categories

| Source | Classification and use |
|---|---|
| [Current Format specification](https://portableapps.com/development/portableapps.com_format) | Official format: sections Directory and File Layout, appinfo.ini, Icons, Installer, Portability and Permissions. Current indexed heading is 3.9 (2026-03-02). |
| [Official Launcher configuration](https://portableapps.com/manuals/PortableApps.comLauncher/ref/launcher.ini/index.html) | Official tool behavior: configuration filename matches the launcher basename within App/AppInfo/Launcher. |
| [Launch section](https://portableapps.com/manuals/PortableApps.comLauncher/ref/launcher.ini/launch.html) | Official tool behavior: ProgramExecutable relative to App and optional CommandLineArguments. |
| [Official Launcher quick start](https://github.com/PortableApps/Launcher/blob/master/Other/Source/Manual/intro/quickstart.rst) | Official tool behavior: Launcher Generator and its development tools. Verify applicability to the installed release. |
| [Official development index](https://portableapps.com/development) | Tool/template release discovery. Indexed versions observed on 2026-10-21: Installer 3.9.18, Launcher 2.2.9, Application Template 3.9.2. Direct page retrieval was restricted; these observations do not attest installed binaries. |
| [Official AppConfigs](https://github.com/PortableApps/AppConfigs) | Example-specific conventions, not universal requirements. Individual configurations and licenses need review before reuse. |
| This wizard's snapshot schema and validation policy | Proposed wizard features/safeguards. Not official format additions or certification. |

The old Launcher-bundled format reference describes older format versions. Do not
copy its full example installer.ini or obsolete options. The current specification
also contains stale prose: its Version explanation refers to 3.5 despite a 3.9
example; RemoveDataDirectory remains mentioned although its history says it was
removed. Disputed options are not exposed. A versioned rule catalog needs to be
completed against the current official template and tool source before release.

## Deliberate boundaries and remaining work

The screens explicitly state that this revision does **not** assemble packages,
extract archives, convert/preview artwork, verify tool provenance/version, run the
Launcher Generator or Installer, capture compiler output, or export an installable
artifact/report. File selection records a reference only. Language values are not
yet checked against the complete locale list. Source executable existence/layout,
ICO contents and PNG dimensions require later asset/package validation. Component
permission records are user declarations, not legal conclusions or proof of rights.

Arbitrary images in the official Installer's left panel have no verified supported
configuration interface in the reviewed sources. Do not invent a key, modify the
official installer to simulate support, or confuse general NSIS artwork support
with a supported PortableApps.com option.

No GPL implementation has been copied into this module. Official tools are planned
as separately invoked executables, with a license/provenance review before bundling.
The new workflow will require the official Installer; the existing capture workflow
and its legacy compiler API are separate and retain their documented limits.

## Next implementation batches

1. Complete the source-versioned rule catalog and review current official template,
   Installer source/distribution, invocation interface and applicable licenses.
2. Add reviewed source mapping and staging into a new package folder, with archive
   boundaries, cancellation, no source overwrites, and file/configuration previews.
3. Add app-icon import/preview, variant generation and content validation. Resolve
   official artwork customization separately before offering it.
4. Connect official tools with exact command review, explicit custom-code consent,
   retained logs, cancellation, fresh output checks and actionable recovery.
5. Export validation evidence and run clean install/launch/relocation/upgrade tests;
   keep automated checks distinct from manual results and official approval.

New screen text is currently English; localization is pending.

## Verification

Core tests cover Unicode snapshot round trips, existing-file refusal, corrupt and
oversized input, unsupported versions/fields, configuration injection, explicit
license declarations, generated metadata interoperability, version overflow,
unresolved components, draft size limits and path overlap. Host all-target checking
and Clippy run separately. Windows GUI, keyboard/screen-reader interaction, scaling,
file-dialog and close-confirmation behavior require execution on Windows; source
inspection and compilation do not establish those checks passed.

The earlier official-source Launcher plus InstallScope-installer lifecycle fixture
still timed out at its first Launcher dialog. It is not evidence that the official
Installer frontend or this new wizard has passed a package lifecycle test.
