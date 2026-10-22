# Project environment profiles

Initial workflow implemented in the regular interface under Developer diagnostics
> Project environment profiles (reviewed local workflow). This is not a sandbox,
tool installer, activation-script runner or compatibility guarantee.

## Default: child process only

Enter a name and absolute Windows project directory. Add absolute PATH directories
(one per line) and optional developer path variables. Preview process-local
environment shows changes to selected inherited values. Unrelated environment
values are neither displayed nor exported. Choose an absolute `.exe`, enter its
exact Windows command-line arguments, review the directory/environment/privileges
and Launch with process-local profile. Profile edits invalidate both previews;
executable, argument or console-mode edits require renewed launch review.

The child inherits remaining environment values and SetupScope's permissions.
SetupScope never mutates its global process environment. No PATH executable search,
shell substitution or elevation fallback is used. Explicitly selecting an
interpreter can execute commands/project code. Opening a console is optional;
without one, interactive console input is unavailable. Only the direct child's
exit is tracked; output is not captured and children/descendants are not killed
when SetupScope closes. Interactive console acceptance remains a live UI gate.

Supported explicit path variables: CARGO_HOME, RUSTUP_HOME, JAVA_HOME, DOTNET_ROOT,
DOTNET_ROOT_X64, DOTNET_ROOT_ARM64, GOPATH, GOROOT, GRADLE_USER_HOME, MAVEN_HOME,
VCPKG_ROOT, CMAKE_PREFIX_PATH, PYTHONPATH and VIRTUAL_ENV. The latter alone does not
activate a virtual environment. GOPATH/CMAKE_PREFIX_PATH/PYTHONPATH accept lists.
Removal is explicit. Arbitrary variables, `.env` files, credentials, URLs, percent
expansion and arbitrary exported inherited environments are unsupported. This path
schema reduces accidental disclosure; it is not a secret detector. Review values
before export. Paths can direct programs to load code; UNC paths may access shares.

PATH prepends preserve a nonempty existing suffix, including internal/trailing
empty entries and percent references. An absent or wholly empty PATH becomes only
the specified prefix; no extra current-directory entry is added. A matching
literal prefix is not added twice; this does not
resolve short-name/expanded-variable aliases or establish shell resolution.
Drive-relative `C:` and absolute `C:\` remain distinct. Existing paths are not
silently cleaned, reordered or removed.

## Import/export

Import profile for review loads bounded version-1 JSON and replaces only editor
contents after validation. Missing/unknown fields and unknown versions fail; null
variable values explicitly mean removal. Nothing applies or launches on import.
Export new profile saves only explicit overrides/name/project to a new file; no
overwrite is permitted. Exported paths are not redacted and may be machine-specific.
There is no automatic project discovery, profile activation, download or upload.

## Optional current-user persistence

Expand Advanced, Preview saved user-level changes, review exact before/after
values/types and acknowledge the local prior-state backup. Apply reviewed
current-user changes writes only selected values under **HKCU\Environment**.
Machine values are never edited. User PATH is not the combined process PATH and
its prepended entries may still follow machine PATH in future processes.

The existing Environment key must be present; the adapter never silently creates
it. Standard strings and expandable strings are supported. Malformed/unsupported
old data blocks editing. Exact original bytes/type/name casing, present-empty
values and absence are retained in a private local transaction receipt under
`SetupScopeData/environment-profiles/transactions/<ID>/receipt.json`.
Protect this data directory: selected old values may be sensitive. They are not
included in exported profiles or uploaded.

Receipts progress Prepared → Applying → Applied or Undoing → Undone. Journal
publication precedes mutation; write/read-back verification and one registry flush
precede completion. A named mutex coordinates same-user/session SetupScope copies;
a data-folder file lock protects receipt updates. Windows identity markers guard
accidental cross-machine/user recovery. These are not signed/authenticated receipts,
protection against edited data, or an atomic multi-value registry transaction.
Other sessions and non-cooperating applications can race between reads and writes.

Refresh receipts and Open environment receipt after an uncertain save or restart.
Review the loaded original/intended states, then explicitly Restore reviewed prior
user values. All current values must match a saved before/after state before undo
starts. Conflicting third-party values stop it; changes to reviewed receipt data
also stop it. Interrupted apply/undo retains a resumable receipt. No halfway cancel
is offered for persistent operations. Do not delete pending receipts. A changed or
missing Environment key, permissions or corrupt evidence can require manual recovery.

Notification is best effort after successful writes. Restart terminals/IDEs that
retain old values. Existing apps are not guaranteed to refresh; no broadcast or
owner-key mutation was used by the native fixture probe.

## Limits and evidence

Limits: 64 KiB profile input/serialization, 64 PATH entries, 16 variable overrides,
4,096 bytes per explicit path, 32,766 UTF-16 units per resulting value, 64 KiB per
old raw value, 4 MiB receipt with reserved completion space, 4,096 native user
environment names inspected and 256 receipt IDs per list. Inputs/receipts must be
regular non-reparse files. Native calls may block; these are not process memory or
time caps. Old receipts are never automatically deleted.

Fourteen Windows core tests cover import/export preservation, strict schemas,
unsupported secret names, malformed values, prefix behavior, exact undo, stale
previews, third-party conflicts, interrupted apply/undo, flush failure, locked
publication, native mutex coordination and identity/path rejection. Four UI control
tests cover separate review gates, keyboard preview, no automatic operations and
editor invalidation. They do not establish real screen-reader acceptance.

The development-only environment_probe ran a child with synthetic CARGO_HOME/PATH,
verified the child overlay and unchanged parent/saved user values, and round-tripped
the profile. user_environment_probe ran native read/set/flush/delete/undo with only
its own process's HKCU redirected to a newly created fixture key. It verified exact
restoration, unchanged real user values, removed the fixture and suppressed broadcast.
Synthetic files/receipts remain under work/; no owner Environment values were edited.

Live GUI/console, DPI/screen-reader, shell notification propagation, cross-session
races, full disk/permission/power-loss fault injection, clean Windows and ARM64
acceptance remain open. No 1.0 readiness is inferred from these component checks.

## API references

- [Windows environment variables](https://learn.microsoft.com/en-us/windows/win32/procthread/environment-variables)
- [CreateProcessW and explicit child environments](https://learn.microsoft.com/en-us/windows/win32/api/processthreadsapi/nf-processthreadsapi-createprocessw)
- [RegFlushKey access and durability contract](https://learn.microsoft.com/en-us/windows/win32/api/winreg/nf-winreg-regflushkey)
- [WM_SETTINGCHANGE notification](https://learn.microsoft.com/en-us/windows/win32/winmsg/wm-settingchange)
- [RegOverridePredefKey process-local fixture mapping](https://learn.microsoft.com/en-us/windows/win32/api/winreg/nf-winreg-regoverridepredefkey)
