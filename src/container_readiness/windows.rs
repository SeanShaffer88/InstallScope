use super::{Observation, Report, SERVICES};
use crate::snapshot::Progress;
use std::path::PathBuf;
use windows_sys::Win32::{
    Foundation::GetLastError,
    System::{Services::*, SystemInformation::GetSystemDirectoryW, Threading::*},
};
fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(Some(0)).collect()
}
pub(super) fn system_directory() -> Result<PathBuf, String> {
    use std::os::windows::ffi::OsStringExt;
    let mut buffer = vec![0u16; 32768];
    let len = unsafe { GetSystemDirectoryW(buffer.as_mut_ptr(), buffer.len() as u32) } as usize;
    if len == 0 || len >= buffer.len() {
        return Err(
            "Windows system directory unavailable; conventional WSL client lookup skipped.".into(),
        );
    }
    Ok(std::ffi::OsString::from_wide(&buffer[..len]).into())
}
struct Handle(SC_HANDLE);
impl Drop for Handle {
    fn drop(&mut self) {
        unsafe {
            CloseServiceHandle(self.0);
        }
    }
}
fn service(manager: &Handle, name: &str) -> (String, bool) {
    let raw = unsafe { OpenServiceW(manager.0, wide(name).as_ptr(), SERVICE_QUERY_STATUS) };
    if raw.is_null() {
        let error = unsafe { GetLastError() };
        return if error == 1060 {
            (
                "Not registered under this conventional name; alternative installations possible."
                    .into(),
                false,
            )
        } else {
            (
                format!("Unknown: query access failed (Windows error {error})."),
                true,
            )
        };
    }
    let handle = Handle(raw);
    let mut status: SERVICE_STATUS_PROCESS = unsafe { std::mem::zeroed() };
    let mut needed = 0;
    if unsafe {
        QueryServiceStatusEx(
            handle.0,
            SC_STATUS_PROCESS_INFO,
            (&mut status as *mut SERVICE_STATUS_PROCESS).cast(),
            std::mem::size_of_val(&status) as u32,
            &mut needed,
        )
    } == 0
    {
        return (
            format!("Unknown: status query failed (Windows error {}).", unsafe {
                GetLastError()
            }),
            true,
        );
    }
    let state = match status.dwCurrentState {
        SERVICE_STOPPED => "Stopped",
        SERVICE_START_PENDING => "Start pending",
        SERVICE_STOP_PENDING => "Stop pending",
        SERVICE_RUNNING => "Running",
        SERVICE_CONTINUE_PENDING => "Continue pending",
        SERVICE_PAUSE_PENDING => "Pause pending",
        SERVICE_PAUSED => "Paused",
        _ => "Unknown service state",
    };
    (format!("{state} at observation time. No service start/stop requested; this does not verify engine health."), !matches!(status.dwCurrentState, 1..=7))
}
pub(super) fn host(report: &mut Report, progress: &Progress) {
    for (name, feature) in [
        ("Firmware virtualization OS hint", PF_VIRT_FIRMWARE_ENABLED),
        (
            "Second-level address translation OS hint",
            PF_SECOND_LEVEL_ADDRESS_TRANSLATION,
        ),
    ] {
        if progress.cancelled() {
            return;
        }
        let present = unsafe { IsProcessorFeaturePresent(feature) } != 0;
        report.host.push(Observation { name, evidence: format!("OS reports {present}. A false value is inconclusive under hypervisors/architecture differences; no firmware or security changes requested.") });
    }
    if progress.cancelled() {
        return;
    }
    let raw = unsafe { OpenSCManagerW(std::ptr::null(), std::ptr::null(), SC_MANAGER_CONNECT) };
    if raw.is_null() {
        report.partial = true;
        report.notes.push(format!(
            "Service manager query unavailable (Windows error {}); no elevation requested.",
            unsafe { GetLastError() }
        ));
        return;
    }
    let manager = Handle(raw);
    for &name in SERVICES {
        if progress.cancelled() {
            return;
        }
        progress.tick();
        let (evidence, partial) = service(&manager, name);
        report.partial |= partial;
        report.host.push(Observation { name, evidence });
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_queries_handle_absent_service_and_system_directory() {
        assert!(system_directory().unwrap().is_absolute());
        let raw = unsafe { OpenSCManagerW(std::ptr::null(), std::ptr::null(), SC_MANAGER_CONNECT) };
        assert!(!raw.is_null());
        let (text, partial) = service(
            &Handle(raw),
            "SetupScope_ReadOnly_Nonexistent_Service_719B1",
        );
        assert!(!partial);
        assert!(text.starts_with("Not registered"));
    }
}
