# WSL and container host readiness

**WSL And Containers** tool window (read-only).
Click Inspect to collect a dated, bounded host report. Cancel at a checkpoint;
operating-system calls must finish first. Previous reports remain visible while
refreshing or if a scan fails, and are labeled as potentially stale.

Passive evidence:
- Fixed wsl.exe, docker.exe, podman.exe and nerdctl.exe candidates in up to 256
  inherited PATH entries, plus Windows' own system directory. No PATH fallback to
  the current directory, shell scripts, PATHEXT execution or recursive discovery.
- Up to 32 regular-file candidates per client. Unsafe/network/reparse paths,
  inaccessible evidence and limits mark the report partial. Files are never run.
- Query-only states for conventional WslService, LxssManager, vmcompute, hns,
  com.docker.service and docker service names. No service start, stop or change.
- OS firmware-virtualization and second-level-address-translation hints. A false
  report can occur with a hypervisor and does not diagnose BIOS settings.
- Presence of seven fixed container routing/config override names. Values are
  neither displayed nor saved; configuration and credential files are not read.

This does not enumerate distributions, determine optional-feature enablement,
query engines or contexts, open pipes, contact services, inspect guest files,
install components, enable virtualization, alter security or launch commands.
Running services and candidate files do not prove readiness. User-mode/custom
engines and non-PATH installs can be missed. Host/guest/container architectures,
WSL versions, modes, permissions, nested virtualization and builds remain unknown.
Do not infer that a missing conventional service means a product is absent.

Report copying is explicit and includes local paths; review it before sharing.
This report is not a diagnostic-bundle source. No automatic save or upload.
No dependency was added: existing windows-sys bindings and guarded file metadata
inspection were reused after consulting the saved reference catalogs.

Verification: fixed-file fixtures (including invalid executable bytes), duplicate
PATH, missing files, directory impostors, malformed/oversized inputs, cancellation,
and native system-directory/absent-service queries pass. Full suite: 438 core,
54 regular UI, 58 native-preview checks; one core and two doc tests ignored.
Clippy passes with existing warnings. Broader live app/accessibility, ARM64, service-state
transitions and broad WSL/container installations remain acceptance work.

Primary references:
- https://learn.microsoft.com/en-us/windows/win32/api/processthreadsapi/nf-processthreadsapi-isprocessorfeaturepresent
- https://devblogs.microsoft.com/oldnewthing/20201216-00/?p=104550
- https://docs.docker.com/reference/cli/docker/
- https://learn.microsoft.com/en-us/windows/wsl/reference

Live local x64 GUI smoke also passed: expand the section, run a scan without
elevation and view dated partial results. A fresh genuine screenshot was captured.
This does not close screen-reader, keyboard, high-DPI or ARM64 acceptance gates.

Live native probe also confirmed four fixed-client entries, processor/service
evidence, unchanged process environment and omission of an injected private
DOCKER_HOST value. The report was partial, explicitly displayed as such.
