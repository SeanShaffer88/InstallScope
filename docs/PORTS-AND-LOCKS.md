# Ports and file owners

Initial read-only workflow implemented in the regular interface under Developer
diagnostics > Ports and file owners (read-only). Nothing scans automatically.

## Local endpoints

Refresh local ports queries TCP IPv4/IPv6 listener tables. Optionally include UDP
bound endpoints; these can be client sockets and are not TCP-style listeners.
Literal numeric local addresses, IPv6 scope IDs, ports and reported owner PIDs are
shown. There are no DNS lookups, connections to reported services, packet sends,
firewall changes or process actions. Wildcard/specific/loopback bindings are
described without claiming remote reachability or firewall exposure.

Tables are sequential snapshots. Process images/creation times are read afterwards
with query-only handles. Endpoint tables do not supply process start identities,
so current PID metadata is explicitly unverified as endpoint-time ownership.
Access failures, exited processes, missing metadata, cancellation and limits remain
visible. Changing a filter does not refresh the evidence; changing Include UDP
affects the next scan only. Equivalent protocol/address/scope/port/PID rows are
merged, so displayed counts are not a count of all sockets or handles.

This covers the Windows host's reported tables, not guest/container/remote network
namespaces. Zero matching rows does not prove a port is free or reachable. Scanning
does not reserve a port; another process can bind or exit immediately afterwards.

## One explicitly selected file

Choose a file, then Inspect selected file owners. Choosing alone does not query.
The target must be an absolute local-drive regular file with supported Unicode;
directories, UNC/device paths, network drives and observed reparse paths are
rejected. Missing/inaccessible paths and API failures are errors, never silently
converted into a successful empty owner list. Previous reports retain their old
target/time if a new query fails or another file is chosen.

A temporary owned Restart Manager session registers only this file and is ended
on return. SetupScope requests no shutdown, restart, unlock, deletion or process
termination. It reads metadata/owner records, not target-file bytes. Windows can
maintain transient session bookkeeping. This uses a separate checked API path;
the older uninstall-oriented helper's silent-empty behavior is not reused.

Reported PID/start-time tuples are compared with current process creation times.
A mismatch withholds the current image to prevent attributing a reused PID's
executable to the old owner. Matching means a dated process-identity observation,
not a guarantee the process is still alive or still holding the file. Permission
failures and unavailable timestamps remain unknown.

Restart Manager is not a complete handle/driver/service/alias lock inventory.
Zero reported owners does not prove a file is unlocked. Size/mtime changes or an
unavailable target after query mark partial results. No persistent target handle
or file-identity lock is held; same-metadata replacement may go undetected.
Reported reboot-reason flags are retained without requesting or recommending a
reboot. No escalation prompt or control of an observed process is provided.

## Review and privacy

Use exact decimal port/PID filters and pages of up to 50 rows (or one row for long
paths). Display/clipboard pages cap at 128 KiB and explicitly mark truncation.
Only Copy this page puts evidence on the clipboard. Local addresses, PIDs, selected
file and process paths are unredacted. No report is automatically saved/exported
or uploaded; this is not a redacted diagnostic bundle.

Limits: four table buffer-fetch attempts after sizing, 16 MiB per table buffer,
4,096 rows per table, 512 distinct port PID metadata queries, four Restart Manager
list calls, 1,024 reported owners and 512 file-owner metadata queries. Process image
buffers hold 32,768 UTF-16 units; selected file paths must be shorter than 32,767
units. Limits and unsupported sources are unknown coverage, not absence. Native
calls may block; cancellation is cooperative between calls/metadata checkpoints.
These are not total process-memory or wall-clock guarantees.

## Verification

Nine core tests cover numeric filters, scoped addresses, bounded Unicode reports,
pagination, native row layout/network order, corrupt/oversized/changing tables,
access-error propagation, cancellation, Restart Manager limits, process creation
identity mismatches and invalid/remote/missing/directory targets. Three GUI control
tests cover explicit keyboard actions, accessible explanations and missing/busy
query gates without launching a scan from the controls.

The development-only ports_locks_probe executable passed on this Windows PC:
cold IP Helper query; owned loopback TCP and UDP on IPv4 and IPv6; exclusive
Unicode-file holder PID/start matching; holder stayed alive/locked during inspection;
unchanged target bytes; reported holder absent after normal fixture release/exit.
It bound only owned loopback fixtures and made no connections, accepts or datagram
sends. Only scratch fixtures were created, and evidence is retained under work/.
No real user process was stopped or unlocked and no firewall rule was changed.

Live GUI/file-picker/DPI/screen-reader, protected/enterprise process coverage,
broader driver/service/alias cases, clean Windows and ARM64 checks remain open.
The regular interface implements this workflow; native-preview build tests do not
mean it has been migrated to the native Win32 UI. This is not 1.0 certification.

## API references

- [GetExtendedTcpTable](https://learn.microsoft.com/en-us/windows/win32/api/iphlpapi/nf-iphlpapi-getextendedtcptable)
- [GetExtendedUdpTable](https://learn.microsoft.com/en-us/windows/win32/api/iphlpapi/nf-iphlpapi-getextendedudptable)
- [TCP row semantics](https://learn.microsoft.com/en-us/windows/win32/api/tcpmib/ns-tcpmib-mib_tcprow_owner_pid)
- [RmGetList](https://learn.microsoft.com/en-us/windows/win32/api/restartmanager/nf-restartmanager-rmgetlist)
- [Restart Manager process identity](https://learn.microsoft.com/en-us/windows/win32/api/restartmanager/ns-restartmanager-rm_unique_process)
