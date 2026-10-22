# Clean Windows release check

VM checks were explicitly skipped by the user for the local 1.0.0.0 package on
2026-10-22. No clean Windows installation or guest test is claimed. VirtualBox
was installed during the earlier attempt; the attempted guest was subsequently
removed. This optional procedure remains available for a future clean PC/VM.

## Run on a clean Windows 11 x64 PC or VM

Copy the distribution folder to a writable local folder on the test machine.
Do not copy an existing `SetupScopeData` folder. No developer SDK or network
connection is required for this baseline test.

From Windows PowerShell 5.1 or later, in that distribution folder:

```powershell
powershell.exe -NoProfile -ExecutionPolicy Bypass -File .\Test-CleanMachine.ps1 -Package . -KeepSandbox
```

This invocation uses a process-only execution-policy option for the reviewed
unsigned test script. It does not change saved Windows execution policy. Inspect
the script first; use your organization's approved script policy if applicable.

The harness checks both executable hashes against the supplied manifest before
copying/starting anything. Hashes establish file consistency, not publisher trust.
It creates a uniquely named temporary portable folder, checks copied hashes,
then tests loader/version/help, eleven embedded languages, offline catalog,
structured errors, passive project inspection and bounded log triage. It uses
System32-only PATH and fresh profile/AppData/temp paths for its own children.
No project scripts, vendor installers, downloads or persistent edits run.

`RESULT.json` and test files remain in the printed temporary folder with
`-KeepSandbox`; remove that specific folder manually after collecting results.
Without that switch, successful CLI-only fixtures are removed after checking their
ownership marker and exact temporary parent. Failed fixtures remain for diagnosis.
The result records the tested executable hashes and OS build. Do not reuse a
passed result after changing binaries.

For manual GUI checks, add `-LaunchGui`. This keeps the temporary folder and opens
its fresh portable copy with the same child-only environment. Close any other
SetupScope instance first, since the singleton applies across these copies. The
script reports only that it requested startup; a person must verify the window.
Close the app before collecting/removing test files.

Check the board, all five tools, language selection/persistence, CJK text,
keyboard focus, high contrast, interface scale and an ordinary smaller display.
Return to the board while tools remain open; reopen a tool and launch another
copy to check instance reuse. Inspect a synthetic project/log and generate a
redacted preview. Prepare/discard an execution review; do not run unknown project
code. Check error paths and readable guidance when developer tools are absent.
Record actual observations, OS/build, language/font availability and limitations.

Vendor installation/UAC/reboot/uninstall and persistent environment recovery
require separate reviewed tests on a disposable VM with a checkpoint. They are
not authorized or executed by this baseline harness. Never perform those tests
on a production machine merely to complete a checklist.

## Local evidence and limits

The harness passed locally with per-process environment/profile isolation. This
still shares the host OS, installed files, registry, fonts, drivers and Windows
account. It is not a new user, clean VM, security sandbox, network restriction or
proof that no optional runtime exists on the machine. A successful GUI startup
also does not establish screen-reader support or all Windows/DPI combinations.

A real clean Windows run was skipped at the user’s request. Windows Sandbox, where supported and
already enabled, can provide an additional disposable environment; do not enable
it or reboot a host automatically. Its mapped-folder and startup behavior is
documented by [Microsoft](https://learn.microsoft.com/windows/security/application-security/application-isolation/windows-sandbox/windows-sandbox-configure-using-wsb-file).
This project has not tested a Sandbox configuration on the current Home host.
