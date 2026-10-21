# Guided PortableApps.com package projects

Status: **project planning, icon conversion and reviewed source staging**.
The regular PortableApps Forge page offers **Guided PAF project** alongside
**Installation capture**. The wizard prepares files for the official tools. It is
not a complete installer builder and introduces no competing package format.

## Workflow available now

1. Start a project, choose separate source/workspace folders, and save or reopen
   incomplete answers. Ten screens cover identity, portability observations,
   launcher settings, icons, official tools, rights/components, review and testing.
2. Enter explicit metadata and license declarations. Unknown permissions block
   preparation; no publisher, version, language or redistribution rights are
   invented. Component notices and permission evidence remain human-reviewed.
3. Select a square static PNG, 256..4096 pixels and at most 16 MiB. Convert it in
   memory and inspect the 128-pixel preview. The original image is unchanged.
4. On Review, choose **Review source mapping and icon**. The worker hashes source
   files, checks names/layout, converts the selected icon, and proposes an exact
   new output path. Inspect the generated configurations, icon and file mappings.
5. Choose **Prepare reviewed files in the new folder**. Only this action creates
   output. Originals are never moved, deleted or overwritten. A changed source
   fails the copy/hash check; create another review plan before retrying.
6. Inspect the prepared folder, project snapshot and preparation report. Build
   using the official tools and complete portability tests before distribution.

### Source mapping

The source folder contains application files, not an already assembled PAF tree.
For `ProgramExecutable=Example\example.exe`, the selected source folder must
contain `example.exe`; its contents are copied beneath `App\Example`. For
`Example\bin\example.exe`, it must contain `bin\example.exe`. Nested directories
are retained. This mapping is a **wizard design**, not a universal PAF rule.
The wizard requires an application subfolder and reserves AppInfo/DefaultData for
its generated layout. Review the mapping rather than renaming/moving originals.

The output wrapper contains the new AppID package folder, `project.json`,
`preparation-report.json`, and a completion marker written only after success.
The package includes source files, UTF-16LE/BOM metadata and launcher configuration,
converted icons, help.html, and empty Data/DefaultData/Other/Source directories.
No unnecessary installer.ini, official Installer receipt or approval is generated.
The preparation report records reviewed source-file hashes; it is not a compiler
log, complete package validation report, legal opinion or portability test.

Review and preparation are cancellable. Partial new output is retained with an
error naming its location, never reported as complete. Existing output directories
are not reused. Disk exhaustion, denied access and source changes fail explicitly.
Cancellation is checked between files/buffer operations; blocking OS/filesystem
calls can delay it. These operations require trusted, privately controlled source
and workspace directories: path checks are not hostile-filesystem race containment.
ACLs, timestamps and alternate data streams are not copied.

### App icons

The independent Rust converter generates `appicon_16.png`, `_32.png`, `_75.png`,
`_128.png`, `_256.png` and `appicon.ico`. ICO includes 16/32/48-pixel 32-bit alpha
frames, corresponding 8-bit compatibility frames, and a 256-pixel PNG frame.
Downsampling uses alpha-weighted area filtering. Only legacy 8-bit frames use an
RGB332 palette and binary transparency; gradients can look less smooth there.

The importer rejects invalid/truncated chunks, duplicate headers, oversized or
animated input and unsupported formats; the shared decoder checks chunk CRCs and
the zlib Adler-32 checksum. Non-interlaced PNG is supported; ICO/JPEG/SVG import,
cropping and arbitrary installer artwork are not yet supported. Older saved ICO
references remain readable as project data but need a PNG selection for conversion.

Arbitrary artwork in the official Installer's left panel still has no verified
supported interface in the reviewed sources. General NSIS customization does not
establish an official PortableApps.com Installer setting. No key is invented and
no official installer source is modified to simulate support.

## Persistence and responsiveness

Project snapshots are InstallScope records outside the package. Saving uses a new
filename, never replaces an existing file, and flushes the file on success. Failure
can leave an incomplete new snapshot with a reported path; earlier files remain
intact. No atomic-publication or power-loss-durability claim is made.
Unsupported versions, unknown project/component fields, malformed JSON and records
over 1 MiB are rejected. Opening another project or closing the regular application
requires confirmation for unsaved answers; active project writes/preparation must
finish first. New-project reset is protected too. Answers and the current step can
be resumed. Opening the original companion `project.json` reconnects its prepared
folder only when its completion marker, bounded report, generated configurations
and recorded application-file sizes/SHA-256 values pass a fresh worker check.
Changed or incomplete output leaves the answers available with an explanatory
warning; no tools execute automatically. Standalone snapshots restore answers
only. These checks do not cover unrecorded extra files, icons, host writes or
portability, and the companion report is not a signed authenticity record.
Official build execution/log capture in this wizard remains pending.

Filesystem checks, hashing, conversion, copying and snapshot I/O use workers.
Mapping rows are rendered on demand instead of allocating/rendering the entire
file list each frame. Plans become stale when answers change. Preparation uses
exactly the reviewed icon bytes, even if the original image later changes.

Wizard resource limits, not PAF requirements: 100,000 source entries, 64 GiB of
source contents, 32 MiB of aggregate source/destination path bytes, 64 KiB of draft
configuration, 1 MiB project snapshots and the image limits above. Symlinks,
Windows reparse points, special files, unsafe Windows names and case-folded name
collisions are rejected. Case folding is conservative and can reject some names
that a particular filesystem treats differently.

Snapshots/reports can contain local paths, notes and permission declarations;
review before sharing. No telemetry, automatic software downloads, source program
execution, custom script execution or elevation is added by staging.
New wizard screen text is currently English; localization is pending.

## References and classifications

| Reference | Classification/use |
|---|---|
| [Current Format specification](https://portableapps.com/development/portableapps.com_format), Directory Layout, appinfo.ini and Icons sections | Official format: package layout, metadata and icon filenames/sizes/depths. Current indexed heading is 3.9 (2026-03-02). |
| [Launcher configuration](https://portableapps.com/manuals/PortableApps.comLauncher/ref/launcher.ini/index.html) | Official tool behavior: App/AppInfo/Launcher configuration basename matches the launcher. |
| [Launch section](https://portableapps.com/manuals/PortableApps.comLauncher/ref/launcher.ini/launch.html) | Official tool behavior: executable relative to App and optional arguments. |
| [Launcher quick start](https://github.com/PortableApps/Launcher/blob/master/Other/Source/Manual/intro/quickstart.rst) | Official tool workflow; check applicability to the installed release. |
| [Official development index](https://portableapps.com/development) and [PortableApps.com SourceForge distributions](https://sourceforge.net/projects/portableapps/files/PortableApps.com%20Installer/) | Release discovery. The SourceForge listing shows Installer 3.9.22 (2026-10-12), newer than the cached website index's 3.9.18. Launcher 2.2.9 and Template 3.9.2 are also listed in their distribution folders. Direct website retrieval was restricted; listings do not attest installed binaries. |
| [Official AppConfigs](https://github.com/PortableApps/AppConfigs) | Example-specific conventions, not universal requirements; individual code/licenses need review before reuse. |
| [Microsoft BITMAPINFOHEADER](https://learn.microsoft.com/windows/win32/api/wingdi/ns-wingdi-bitmapinfoheader) and [RFC 1950](https://www.rfc-editor.org/rfc/rfc1950) | Underlying image/container behavior, not additional PAF requirements. |
| Snapshot schema, source mapping, resource limits, hash review, reserved staging folders | Proposed wizard features/safeguards. No certification or format extension. |

Older Launcher-bundled format references and some current specification prose are
stale: Version prose refers to 3.5 despite a 3.9 example; RemoveDataDirectory remains
mentioned despite its recorded removal. Disputed options are not exposed. The
versioned rule catalog still needs completion against the current official template
and distribution/source before release. No GPL implementation was copied into these
modules. Bundling official tools still requires license/provenance review.

## Official builds and remaining gates

The separate capture screen stops after capture for command review and requires
both official frontends. Missing tools stop before compilation/download. Recognized
custom NSIS entry points need explicit per-attempt consent. The legacy source-build
API remains for development compatibility, not a desktop fallback.

This wizard still does not extract archives, run official frontends, capture their
compiler logs, or export an
installable artifact/final validation report. Complete language-value checking,
notice-file verification, file/registry redirection assistance and current-tool
lifecycle verification also remain. Static PE checks establish header signatures,
not authenticity, a complete loader validation or runtime portability.

Core tests cover persistence, injection, declarations, generated metadata, layout,
hash-bound copies, unchanged originals, cancellation before/during copying, changed
source size/content, existing-output refusal, Unix symlinks/case collisions, PNG
bounds/checksums, icon sizes/depths and alpha resampling. Independent Pillow decoding
checks exported ICO/PNG data; Windows CI also attempts icon loading through Win32.
Distinguish passing results from configured tests. Windows GUI, screen-reader,
scaling, file-dialog and close-confirmation interaction remain unverified.

The legacy official-source Launcher plus InstallScope-installer fixture still
stopped at its first Launcher dialog. It is not evidence that the official
Installer frontend or this new wizard passed a complete package lifecycle test.

### Verified historical distributions and test baseline

For development research, Installer 3.9.18 and Launcher 2.2.9 were downloaded
from the official PortableApps SourceForge project, SHA-256 checked against the
[Installer](https://portableapps.com/apps/development/portableapps.com_installer)
and [Launcher](https://portableapps.com/apps/development/portableapps.com_launcher)
page values, then extracted without running their self-extractors. The CI fixture
pins these same hashes and calls the extracted official frontends separately.
This is a historical regression baseline, not a latest-release or certification
claim. A configured test is not a passing result.

Inspected Installer 3.9.18 distribution files (tool behavior, not format rules):
`Other/Source/InstallerWizard.nsi`, `.onInit` accepts the package directory as
its command line; `Section Main` writes `Data/PortableApps.comInstallerLog.txt`,
replaces generated installer source/artwork, and invokes its bundled NSIS.
`App/installer/PortableApps.comInstaller.nsi` selects the generated welcome bitmap.
Placing a custom bitmap at that generated path is therefore not a supported
customization mechanism in this inspected version. Launcher 2.2.9's
`Other/Source/GeneratorWizard.nsi` similarly accepts a package directory and writes
`Data/PortableApps.comLauncherGeneratorLog.txt`.

The bundled `Other/Source/Readme.txt` and `License.txt` identify GPL-2.0-or-later
Installer code. Its welcome text also describes unmodified use for open-source
and freeware packages and contacting PortableApps.com for commercial software.
Treat this as a licensing/trademark question for human/upstream review; this
project makes no legal conclusion and copies no GPL implementation into MIT code.
