use super::*;
use anyhow::Context;
use std::{
    ffi::c_void,
    mem::{offset_of, size_of},
    net::{Ipv4Addr, Ipv6Addr},
};
use windows_sys::Win32::{
    Foundation::{CloseHandle, FILETIME, HANDLE},
    NetworkManagement::IpHelper::*,
    Networking::WinSock::{AF_INET, AF_INET6},
    Storage::FileSystem::GetDriveTypeW,
    System::{
        RestartManager::{
            RmEndSession, RmGetList, RmRegisterResources, RmStartSession, RM_PROCESS_INFO,
        },
        Threading::{
            GetProcessTimes, OpenProcess, QueryFullProcessImageNameW,
            PROCESS_QUERY_LIMITED_INFORMATION,
        },
    },
};
const TABLE_LIMIT: usize = 16 * 1024 * 1024;
const ROW_LIMIT: usize = 4096;
const PROCESS_LIMIT: usize = 512;
const OWNER_LIMIT: usize = 1024;
fn stamp(value: FILETIME) -> u64 {
    (u64::from(value.dwHighDateTime) << 32) | u64::from(value.dwLowDateTime)
}
fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(Some(0)).collect()
}
struct Handle(HANDLE);
impl Drop for Handle {
    fn drop(&mut self) {
        unsafe { CloseHandle(self.0) };
    }
}
fn process(pid: u32, expected: Option<u64>) -> ProcessEvidence {
    let unavailable = |detail: String| ProcessEvidence {
        pid,
        creation: None,
        image: None,
        identity: Identity::Unavailable,
        detail,
    };
    let handle = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
    if handle.is_null() {
        return unavailable(format!(
            "OpenProcess failed: {}. Access may be denied or the process may have exited.",
            std::io::Error::last_os_error()
        ));
    }
    let handle = Handle(handle);
    let mut created = FILETIME {
        dwLowDateTime: 0,
        dwHighDateTime: 0,
    };
    let mut exited = created;
    let mut kernel = created;
    let mut user = created;
    if unsafe { GetProcessTimes(handle.0, &mut created, &mut exited, &mut kernel, &mut user) } == 0
    {
        return unavailable(format!(
            "Process creation time unavailable: {}. Image attribution withheld.",
            std::io::Error::last_os_error()
        ));
    }
    let creation = stamp(created);
    if expected.is_some_and(|start| start != creation) {
        return ProcessEvidence{pid,creation:Some(creation),image:None,identity:Identity::StartMismatch,detail:"Current PID creation time differs from Restart Manager's reported process; do not attribute its image to the reported owner.".into()};
    }
    let mut path = vec![0u16; 32768];
    let mut length = path.len() as u32;
    let (image, detail) = if unsafe {
        QueryFullProcessImageNameW(handle.0, 0, path.as_mut_ptr(), &mut length)
    } == 0
    {
        (
            None,
            format!(
                "Image path unavailable: {}",
                std::io::Error::last_os_error()
            ),
        )
    } else if length as usize > path.len() {
        (None, "Image path length exceeded its buffer".into())
    } else {
        match String::from_utf16(&path[..length as usize]){Ok(path)=>(Some(path),"Metadata belongs to the opened process handle at query time. It may have exited or changed since observation.".into()),Err(_)=>(None,"Image path contains unsupported Unicode".into())}
    };
    ProcessEvidence {
        pid,
        creation: Some(creation),
        image,
        identity: if expected.is_some() {
            Identity::StartMatched
        } else {
            Identity::PidOnly
        },
        detail,
    }
}
// Private whitelist: these C rows contain only integers/byte arrays. Reading
// their copied bytes cannot create an invalid Rust enum, reference or bool.
trait Row: Copy {
    const OFFSET: usize;
}
impl Row for MIB_TCPROW_OWNER_PID {
    const OFFSET: usize = offset_of!(MIB_TCPTABLE_OWNER_PID, table);
}
impl Row for MIB_TCP6ROW_OWNER_PID {
    const OFFSET: usize = offset_of!(MIB_TCP6TABLE_OWNER_PID, table);
}
impl Row for MIB_UDPROW_OWNER_PID {
    const OFFSET: usize = offset_of!(MIB_UDPTABLE_OWNER_PID, table);
}
impl Row for MIB_UDP6ROW_OWNER_PID {
    const OFFSET: usize = offset_of!(MIB_UDP6TABLE_OWNER_PID, table);
}
fn decode<T: Row>(bytes: &[u8]) -> Result<Vec<T>> {
    ensure!(
        bytes.len() >= 4,
        "Endpoint table is missing its count header"
    );
    let count = u32::from_ne_bytes(bytes[..4].try_into().unwrap()) as usize;
    ensure!(
        count <= ROW_LIMIT,
        "Endpoint table exceeds 4096 rows; coverage unknown"
    );
    let end = T::OFFSET
        .checked_add(
            count
                .checked_mul(size_of::<T>())
                .context("Endpoint row size overflow")?,
        )
        .context("Endpoint table size overflow")?;
    ensure!(
        end <= bytes.len(),
        "Endpoint table is truncated or changed during query"
    );
    Ok((0..count)
        .map(|index| unsafe {
            std::ptr::read_unaligned(
                bytes
                    .as_ptr()
                    .add(T::OFFSET + index * size_of::<T>())
                    .cast::<T>(),
            )
        })
        .collect())
}
fn query<T: Row>(
    mut api: impl FnMut(*mut c_void, *mut u32) -> u32,
    progress: &Progress,
) -> Result<Vec<T>> {
    ensure!(!progress.cancelled(), "Endpoint query cancelled");
    let mut needed = 0u32;
    let rc = api(std::ptr::null_mut(), &mut needed);
    ensure!(
        rc == 0 || rc == 122,
        "Endpoint size query failed (Windows error {rc}); coverage unknown"
    );
    for _ in 0..4 {
        ensure!(!progress.cancelled(), "Endpoint query cancelled");
        ensure!(
            (4..=TABLE_LIMIT).contains(&(needed as usize)),
            "Endpoint table size exceeds bounds or has no header"
        );
        let mut aligned = vec![0u64; (needed as usize).div_ceil(8)];
        let capacity = needed;
        let rc = api(aligned.as_mut_ptr().cast(), &mut needed);
        if rc == 122 {
            continue;
        }
        ensure!(
            rc == 0,
            "Endpoint query failed (Windows error {rc}); coverage unknown"
        );
        ensure!(
            needed <= capacity,
            "Endpoint table grew beyond the allocated buffer"
        );
        let bytes =
            unsafe { std::slice::from_raw_parts(aligned.as_ptr().cast::<u8>(), needed as usize) };
        return decode(bytes);
    }
    anyhow::bail!("Endpoint table kept changing across four attempts; coverage unknown")
}
fn port(value: u32) -> u16 {
    u16::from_be(value as u16)
}
fn tcp4(row: MIB_TCPROW_OWNER_PID) -> Endpoint {
    Endpoint {
        transport: Transport::Tcp,
        address: Ipv4Addr::from(row.dwLocalAddr.to_ne_bytes()).into(),
        scope_id: 0,
        port: port(row.dwLocalPort),
        pid: row.dwOwningPid,
    }
}
fn tcp6(row: MIB_TCP6ROW_OWNER_PID) -> Endpoint {
    Endpoint {
        transport: Transport::Tcp,
        address: Ipv6Addr::from(row.ucLocalAddr).into(),
        scope_id: row.dwLocalScopeId,
        port: port(row.dwLocalPort),
        pid: row.dwOwningPid,
    }
}
fn table(kind: usize, progress: &Progress) -> Result<Vec<Endpoint>> {
    match kind {
        0 => Ok(query::<MIB_TCPROW_OWNER_PID>(
            |p, n| unsafe {
                GetExtendedTcpTable(p, n, 0, AF_INET as u32, TCP_TABLE_OWNER_PID_LISTENER, 0)
            },
            progress,
        )?
        .into_iter()
        .filter(|r| r.dwState == 2)
        .map(tcp4)
        .collect()),
        1 => Ok(query::<MIB_TCP6ROW_OWNER_PID>(
            |p, n| unsafe {
                GetExtendedTcpTable(p, n, 0, AF_INET6 as u32, TCP_TABLE_OWNER_PID_LISTENER, 0)
            },
            progress,
        )?
        .into_iter()
        .filter(|r| r.dwState == 2)
        .map(tcp6)
        .collect()),
        2 => Ok(query::<MIB_UDPROW_OWNER_PID>(
            |p, n| unsafe { GetExtendedUdpTable(p, n, 0, AF_INET as u32, UDP_TABLE_OWNER_PID, 0) },
            progress,
        )?
        .into_iter()
        .map(|r| Endpoint {
            transport: Transport::Udp,
            address: Ipv4Addr::from(r.dwLocalAddr.to_ne_bytes()).into(),
            scope_id: 0,
            port: port(r.dwLocalPort),
            pid: r.dwOwningPid,
        })
        .collect()),
        _ => Ok(query::<MIB_UDP6ROW_OWNER_PID>(
            |p, n| unsafe { GetExtendedUdpTable(p, n, 0, AF_INET6 as u32, UDP_TABLE_OWNER_PID, 0) },
            progress,
        )?
        .into_iter()
        .map(|r| Endpoint {
            transport: Transport::Udp,
            address: Ipv6Addr::from(r.ucLocalAddr).into(),
            scope_id: r.dwLocalScopeId,
            port: port(r.dwLocalPort),
            pid: r.dwOwningPid,
        })
        .collect()),
    }
}
pub(super) fn ports(include_udp: bool, progress: &Progress) -> Result<PortReport> {
    let mut report = PortReport {
        started: crate::util::now_string(),
        finished: String::new(),
        endpoints: vec![],
        processes: BTreeMap::new(),
        sources: vec![],
        partial: false,
        cancelled: false,
    };
    for (kind, name) in ["TCP IPv4", "TCP IPv6", "UDP IPv4", "UDP IPv6"]
        .into_iter()
        .enumerate()
        .take(if include_udp { 4 } else { 2 })
    {
        progress.set_phase(name);
        match table(kind, progress) {
            Ok(rows) => {
                report.sources.push(Source {
                    name,
                    rows: rows.len(),
                    error: None,
                });
                report.endpoints.extend(rows);
            }
            Err(error) => {
                report.partial = true;
                report.sources.push(Source {
                    name,
                    rows: 0,
                    error: Some(format!("{error:#}")),
                });
            }
        }
        progress.tick();
    }
    report.endpoints.sort();
    report.endpoints.dedup();
    let pids: std::collections::BTreeSet<_> = report.endpoints.iter().map(|e| e.pid).collect();
    for (index, pid) in pids.into_iter().enumerate() {
        let evidence = if progress.cancelled() {
            ProcessEvidence::skipped(pid, "Cancelled before metadata query")
        } else if index >= PROCESS_LIMIT {
            ProcessEvidence::skipped(pid, "512 distinct PID metadata limit reached")
        } else {
            progress.set_phase(format!("Current PID metadata: {pid}"));
            process(pid, None)
        };
        if evidence.image.is_none() {
            report.partial = true;
        }
        report.processes.insert(pid, evidence);
        progress.tick();
    }
    report.cancelled = progress.cancelled();
    report.partial |= report.cancelled;
    report.finished = crate::util::now_string();
    Ok(report)
}
struct Session(u32);
impl Drop for Session {
    fn drop(&mut self) {
        unsafe { RmEndSession(self.0) };
    }
}
fn owners(session: u32, progress: &Progress) -> Result<(Vec<RM_PROCESS_INFO>, u32)> {
    owner_rows(
        |needed, count, infos, reasons| unsafe {
            RmGetList(session, needed, count, infos, reasons)
        },
        progress,
    )
}
fn owner_rows(
    mut api: impl FnMut(&mut u32, &mut u32, *mut RM_PROCESS_INFO, &mut u32) -> u32,
    progress: &Progress,
) -> Result<(Vec<RM_PROCESS_INFO>, u32)> {
    let mut infos: Vec<RM_PROCESS_INFO> = vec![];
    for _ in 0..4 {
        ensure!(!progress.cancelled(), "File owner query cancelled");
        let mut needed = 0u32;
        let mut count = infos.len() as u32;
        let mut reasons = 0u32;
        let rc = api(
            &mut needed,
            &mut count,
            if infos.is_empty() {
                std::ptr::null_mut()
            } else {
                infos.as_mut_ptr()
            },
            &mut reasons,
        );
        if rc == 0 {
            ensure!(
                count as usize <= infos.len(),
                "Restart Manager returned an invalid owner count"
            );
            infos.truncate(count as usize);
            return Ok((infos, reasons));
        }
        ensure!(
            rc == 234,
            "Restart Manager owner query failed (Windows error {rc}); ownership unknown"
        );
        ensure!(
            needed > 0 && needed as usize <= OWNER_LIMIT,
            "Restart Manager exceeds the 1024-owner limit; ownership unknown"
        );
        infos = vec![unsafe { std::mem::zeroed() }; needed as usize];
    }
    anyhow::bail!(
        "Restart Manager owner list kept changing across four attempts; ownership unknown"
    )
}
fn validate_file(path: &Path) -> Result<std::fs::Metadata> {
    use std::path::{Component, Prefix};
    ensure!(
        path.is_absolute()
            && !path
                .components()
                .any(|c| matches!(c, Component::CurDir | Component::ParentDir)),
        "Choose an absolute local file path without dot components"
    );
    let drive = match path.components().next() {
        Some(Component::Prefix(prefix)) => match prefix.kind() {
            Prefix::Disk(letter) | Prefix::VerbatimDisk(letter) => letter,
            _ => anyhow::bail!(
                "UNC and device paths are unsupported; remote services are not queried"
            ),
        },
        _ => anyhow::bail!("Choose a local drive file"),
    };
    let value = path
        .to_str()
        .context("Selected file path must be valid Unicode")?;
    ensure!(
        !value.chars().any(|c| c.is_control()) && value.encode_utf16().count() < 32767,
        "Selected path is too long or contains control characters"
    );
    let drive_type = unsafe { GetDriveTypeW(wide(&format!("{}:\\", drive as char)).as_ptr()) };
    ensure!(
        matches!(drive_type, 2 | 3 | 5 | 6),
        "Remote, unavailable or unsupported drive; ownership unknown"
    );
    crate::store::durable::check_path(path)?;
    let metadata = std::fs::symlink_metadata(path).context("Selected file is unavailable")?;
    ensure!(metadata.is_file(), "Select a regular file, not a directory");
    Ok(metadata)
}
pub(super) fn locks(path: &Path, progress: &Progress) -> Result<LockReport> {
    ensure!(!progress.cancelled(), "File inspection cancelled");
    let started = crate::util::now_string();
    let before = validate_file(path)?;
    progress.set_phase("Registering selected file (read-only)");
    let mut session = 0u32;
    let mut key = [0u16; 33];
    let rc = unsafe { RmStartSession(&mut session, 0, key.as_mut_ptr()) };
    ensure!(
        rc == 0,
        "Restart Manager session unavailable (Windows error {rc}); ownership unknown"
    );
    let session = Session(session);
    let file = wide(path.to_str().unwrap());
    let pointer = file.as_ptr();
    let rc = unsafe {
        RmRegisterResources(
            session.0,
            1,
            &pointer,
            0,
            std::ptr::null(),
            0,
            std::ptr::null(),
        )
    };
    ensure!(
        rc == 0,
        "Selected file could not be registered (Windows error {rc}); ownership unknown"
    );
    progress.set_phase("Reading Restart Manager owner list");
    let (infos, reboot_reasons) = owners(session.0, progress)?;
    let mut report=LockReport{path:path.into(),started,finished:String::new(),owners:vec![],reboot_reasons,notes:vec!["Only one explicitly selected local file was registered. No content was read. Restart Manager may omit some handle, driver, service or alias cases; this is not a complete lock inventory.".into()],partial:reboot_reasons!=0,cancelled:false};
    if reboot_reasons != 0 {
        report.notes.push("Restart Manager reported permission/session/critical-resource or other reboot reasons. Treat coverage as partial; no reboot or process action is requested.".into());
    }
    for (index, info) in infos.into_iter().enumerate() {
        let pid = info.Process.dwProcessId;
        let expected = stamp(info.Process.ProcessStartTime);
        let evidence = if progress.cancelled() {
            ProcessEvidence::skipped(pid, "Cancelled before process verification")
        } else if index >= PROCESS_LIMIT {
            ProcessEvidence::skipped(pid, "512 process metadata limit reached")
        } else {
            process(pid, (expected != 0).then_some(expected))
        };
        if evidence.identity != Identity::StartMatched || evidence.image.is_none() {
            report.partial = true;
        }
        let text = |buffer: &[u16]| {
            String::from_utf16_lossy(
                &buffer[..buffer.iter().position(|c| *c == 0).unwrap_or(buffer.len())],
            )
        };
        report.owners.push(LockOwner {
            pid,
            reported_start: expected,
            name: text(&info.strAppName),
            service: text(&info.strServiceShortName),
            process: evidence,
        });
        progress.tick();
    }
    match validate_file(path) {
        Ok(after)
            if before.len() == after.len() && before.modified().ok() == after.modified().ok() => {}
        Ok(_) => {
            report.partial = true;
            report.notes.push("Selected-file metadata changed during query; observed owners may no longer describe this path.".into());
        }
        Err(_) => {
            report.partial = true;
            report.notes.push("Selected file became unavailable or unsupported during query; current ownership is unknown.".into());
        }
    }
    report.notes.push("No persistent target handle or file identity is held. A same-metadata replacement can go undetected; refresh before drawing conclusions.".into());
    report.cancelled = progress.cancelled();
    report.partial |= report.cancelled;
    report.finished = crate::util::now_string();
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn encoded<T: Row>(row: T) -> Vec<u8> {
        let mut bytes = vec![0; T::OFFSET + size_of::<T>()];
        bytes[..4].copy_from_slice(&1u32.to_ne_bytes());
        unsafe {
            std::ptr::copy_nonoverlapping(
                (&row as *const T).cast::<u8>(),
                bytes.as_mut_ptr().add(T::OFFSET),
                size_of::<T>(),
            );
        }
        bytes
    }
    #[test]
    fn native_table_layout_and_network_order_decode_correctly() {
        let mut row: MIB_TCPROW_OWNER_PID = unsafe { std::mem::zeroed() };
        row.dwState = 2;
        row.dwLocalAddr = u32::from_ne_bytes([127, 0, 0, 1]);
        row.dwLocalPort = u32::from(8080u16.to_be());
        row.dwOwningPid = 42;
        let bytes = encoded(row);
        let parsed = query::<MIB_TCPROW_OWNER_PID>(
            |pointer, size| {
                unsafe {
                    let capacity = *size;
                    *size = bytes.len() as u32;
                    if pointer.is_null() || capacity < bytes.len() as u32 {
                        return 122;
                    }
                    std::ptr::copy_nonoverlapping(bytes.as_ptr(), pointer.cast(), bytes.len());
                }
                0
            },
            &Progress::default(),
        )
        .unwrap();
        assert_eq!(tcp4(parsed[0]).local_address(), "127.0.0.1:8080");
        assert_eq!(tcp4(parsed[0]).pid, 42);
        for p in [0u16, 80, 65535] {
            assert_eq!(port(u32::from(p.to_be())), p);
        }
        let mut ipv6: MIB_TCP6ROW_OWNER_PID = unsafe { std::mem::zeroed() };
        ipv6.ucLocalAddr = "fe80::1".parse::<Ipv6Addr>().unwrap().octets();
        ipv6.dwLocalScopeId = 9;
        ipv6.dwLocalPort = u32::from(443u16.to_be());
        assert_eq!(tcp6(ipv6).local_address(), "[fe80::1%9]:443");
    }
    #[test]
    fn malformed_oversized_and_changing_tables_fail_as_unknown_not_empty() {
        assert!(decode::<MIB_TCPROW_OWNER_PID>(&[]).is_err());
        assert!(decode::<MIB_TCPROW_OWNER_PID>(&1u32.to_ne_bytes()).is_err());
        assert!(decode::<MIB_TCPROW_OWNER_PID>(&((ROW_LIMIT + 1) as u32).to_ne_bytes()).is_err());
        assert!(decode::<MIB_TCPROW_OWNER_PID>(&0u32.to_ne_bytes())
            .unwrap()
            .is_empty());
        let error = query::<MIB_TCPROW_OWNER_PID>(|_, _| 5, &Progress::default())
            .err()
            .unwrap();
        assert!(error.to_string().contains("5"));
        assert!(query::<MIB_TCPROW_OWNER_PID>(
            |_, size| {
                unsafe { *size = (TABLE_LIMIT + 1) as u32 };
                122
            },
            &Progress::default()
        )
        .is_err());
        let mut calls = 0;
        let result = query::<MIB_TCPROW_OWNER_PID>(
            |_, size| {
                calls += 1;
                unsafe { *size = 4 * calls };
                122
            },
            &Progress::default(),
        );
        assert!(result.is_err());
        assert_eq!(calls, 5);
    }
    #[test]
    fn cancellation_prevents_table_api_calls() {
        let progress = Progress::default();
        progress
            .cancel
            .store(true, std::sync::atomic::Ordering::Relaxed);
        let mut calls = 0;
        assert!(query::<MIB_TCPROW_OWNER_PID>(
            |_, _| {
                calls += 1;
                5
            },
            &progress
        )
        .is_err());
        assert_eq!(calls, 0);
        let report = ports(true, &progress).unwrap();
        assert!(
            report.partial && report.cancelled && report.sources.iter().all(|s| s.error.is_some())
        );
    }
    #[test]
    fn restart_manager_errors_limits_and_invalid_counts_are_not_empty_success() {
        assert!(owner_rows(|_, _, _, _| 5, &Progress::default()).is_err());
        assert!(owner_rows(
            |needed, _, _, _| {
                *needed = (OWNER_LIMIT + 1) as u32;
                234
            },
            &Progress::default()
        )
        .is_err());
        assert!(owner_rows(
            |_, count, _, _| {
                *count = 1;
                0
            },
            &Progress::default()
        )
        .is_err());
        let (empty, _) = owner_rows(
            |_, count, _, _| {
                *count = 0;
                0
            },
            &Progress::default(),
        )
        .unwrap();
        assert!(empty.is_empty());
        let mut calls = 0;
        let (rows, reasons) = owner_rows(
            |needed, count, infos, reasons| {
                calls += 1;
                if infos.is_null() {
                    *needed = 1;
                    return 234;
                }
                unsafe {
                    (*infos).Process.dwProcessId = 42;
                }
                *count = 1;
                *reasons = 1;
                0
            },
            &Progress::default(),
        )
        .unwrap();
        assert_eq!(calls, 2);
        assert_eq!(rows[0].Process.dwProcessId, 42);
        assert_eq!(reasons, 1);
    }
    #[test]
    fn current_process_start_time_guards_reused_pid_attribution() {
        let pid = std::process::id();
        let initial = process(pid, None);
        let creation = initial.creation.expect("current process creation time");
        assert_eq!(initial.identity, Identity::PidOnly);
        assert_eq!(
            process(pid, Some(creation)).identity,
            Identity::StartMatched
        );
        let wrong = process(pid, Some(creation + 1));
        assert_eq!(wrong.identity, Identity::StartMismatch);
        assert!(wrong.image.is_none());
        assert_eq!(process(0, None).identity, Identity::Unavailable);
    }
    #[test]
    fn unsupported_remote_device_missing_and_directory_targets_are_errors() {
        for value in ["relative.txt", r"\\server\share\file", r"\\.\pipe\fixture"] {
            assert!(validate_file(Path::new(value)).is_err());
        }
        let temp = crate::util::tempdir().unwrap();
        assert!(validate_file(temp.path()).is_err());
        assert!(validate_file(&temp.path().join("missing")).is_err());
    }
}
