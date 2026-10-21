# User-provided debloat and privacy references

Reviewed 2026-10-21. These links are research leads, not policy sources. They
do not establish that a setting works on every Windows edition or build. Use
current Microsoft documentation for Windows behavior, and verify any new
setting against supported Windows builds before adding it. No source code from
these repositories was copied into InstallScope.

## Resources

### List Decluttering Tool

- [Repository](https://github.com/MeiFagundes/ListDeclutteringTool)
- [WPF implementation at the latest reviewed commit](https://github.com/MeiFagundes/ListDeclutteringTool/blob/75fe57d3fbec8d47b6cf62e871fea20e93a31d21/ListDeclutteringTool/ListDeclutteringTool.xaml.cs)
- [MIT license at its initial commit](https://github.com/MeiFagundes/ListDeclutteringTool/blob/64d2ec42eb2a91f6bfb21fc45ad45525bf0206d3/LICENSE)

The small WPF tool transforms pasted or opened text into a cleaned list. Its
options include spaces, tabs, dots, or a custom separator; removing numeric
tokens or a chosen substring; and copying or saving the output. The repository's
latest commit found in this review is from 2018. The current default-branch
contents endpoint did not expose a `LICENSE` file, so the historical MIT file
is not enough to infer the current branch's licensing state.

This is not currently a fit for InstallScope's core workflows: pasted strings
are not trustworthy installed-app identities. A future custom-list import
would need a preview, exact identifier matching, explicit selection, and the
existing reviewed removal/backup flow; it must never execute arbitrary pasted
commands or remove items directly from text.

### Reading Resources

- [Repository](https://github.com/mycaule/reading-resources)
- [README](https://github.com/mycaule/reading-resources/blob/master/README.md)
- [MIT license](https://github.com/mycaule/reading-resources/blob/master/LICENSE)

This is a broad, manually curated reading log with a table of contents and
topic-grouped links, rather than an implementation reference for Windows
maintenance. Its useful contribution here is the idea of keeping research
links in one navigable project register. The register should record what was
checked, what is applicable, and what needs independent verification.

### UnbloatWindows

- [Repository and README](https://github.com/jkerai1/UnbloatWindows)
- [PowerShell script](https://github.com/jkerai1/UnbloatWindows/blob/main/DeCrapify%20Windows%20%2B%20DarkMode.ps1)
- [Initial script commit](https://github.com/jkerai1/UnbloatWindows/commit/a885fa2a005a94eb957115b94da165986fe94a32)
- [Latest repository commit found in this review](https://github.com/jkerai1/UnbloatWindows/commit/07d378e9d3d7a495fb9be2ca8021f012c757dabd)

The README identifies the project as an elevated PowerShell debloat script and
asks users to inspect it before running. The reviewed script performs broad
registry changes, removes a fixed set of Appx packages, disables multiple
services, changes an AutoLogger directory ACL, changes Windows optional
features, and includes restart behavior. It was not a per-setting review and
restore workflow. The script dates back to 2021; the latest commit found was a
README-only change in 2024. No license file was exposed on the current default
branch, so its implementation must not be copied or redistributed.

## InstallScope decisions

- Keep Windows privacy changes narrow and user-selected. The Windows Privacy
  page records observed state and scope, explains tradeoffs, confirms each
  change, and saves exact previous registry values through Backups. That
  workflow already adopts the useful *review each setting* lesson without
  adopting this script's system-wide batch changes.
- Keep app removal on the Bloatware Shield's discovery, classification,
  selection, review, and backup path. Do not import an old static Appx list as
  a current compatibility or safety verdict.
- Do not disable diagnostic services, edit ACLs to suppress Windows logging,
  force restarts, or turn off security, updates, accessibility, or other broad
  system services based on these references. Any proposed control needs a
  current authoritative source, supported-build scope, state inspection,
  confirmation, exact backup/restore behavior, and regression coverage.
- Do not add a standalone list-cleaning surface without a concrete InstallScope
  workflow that benefits from it. If custom package-list import becomes a
  requirement, treat the text as untrusted input and resolve only exact known
  identities through the existing review flow.

## Microsoft Recall policy review

Reviewed 2026-10-21 against Microsoft's current [WindowsAI Policy CSP](https://learn.microsoft.com/en-us/windows/client-management/mdm/policy-csp-windowsai)
and [Manage Recall for Windows clients](https://learn.microsoft.com/en-us/windows/client-management/manage-recall).
The `DisableAIDataAnalysis` policy is documented for supported Pro,
Enterprise, Education, and IoT Enterprise editions on Windows 11 24H2 with
KB5055627 (build 26100.3915) and later. Enabling it prevents Recall snapshots
from being saved, and Microsoft states that existing snapshots are deleted.

InstallScope exposes this as a current-account policy only after checking the
installed build, update revision, and edition. It displays the deletion effect
before confirmation and saves the exact prior registry value. That backup does
not contain Recall snapshots and cannot restore them. Unknown or unsupported
system profiles fail closed for applying the policy; an existing policy value
can still be cleared after review.
