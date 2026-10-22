//! Small read-only workstation summary; no identifiers, tool execution or saves.
use std::time::{Duration, Instant};
pub struct SystemInfo {
    next: Instant,
    text: String,
}
impl Default for SystemInfo {
    fn default() -> Self {
        Self {
            next: Instant::now(),
            text: String::new(),
        }
    }
}
impl SystemInfo {
    pub fn localized_parts(&mut self) -> Vec<String> {
        self.summary().split("  ·  ").map(setupscope_core::i18n::t).collect()
    }
    pub fn localized_summary(&mut self) -> String {
        self.localized_parts().join("  ·  ")
    }

    pub fn summary(&mut self) -> &str {
        if self.text.is_empty() || Instant::now() >= self.next {
            self.text = summary();
            self.next = Instant::now() + Duration::from_secs(5);
        }
        &self.text
    }
}
pub fn summary() -> String {
    let cpu = std::thread::available_parallelism()
        .map(|n| n.get().to_string())
        .unwrap_or_else(|_| "?".into());
    let app = match std::env::consts::ARCH {
        "x86_64" => "x64",
        "aarch64" => "ARM64",
        "x86" => "x86",
        other => other,
    };
    #[cfg(windows)]
    {
        use windows_sys::Win32::System::{Registry::*, SystemInformation::*};
        let mut info: SYSTEM_INFO = unsafe { std::mem::zeroed() };
        unsafe { GetNativeSystemInfo(&mut info) };
        let host = match unsafe { info.Anonymous.Anonymous.wProcessorArchitecture } {
            9 => "x64",
            12 => "ARM64",
            0 => "x86",
            _ => "Unknown",
        };
        let mut memory: MEMORYSTATUSEX = unsafe { std::mem::zeroed() };
        memory.dwLength = std::mem::size_of_val(&memory) as u32;
        let ram = if unsafe { GlobalMemoryStatusEx(&mut memory) } != 0 {
            format!(
                "RAM {:.1} / {:.1} GiB Available",
                memory.ullAvailPhys as f64 / 1073741824.0,
                memory.ullTotalPhys as f64 / 1073741824.0
            )
        } else {
            "RAM Unknown".into()
        };
        static BUILD: std::sync::OnceLock<String> = std::sync::OnceLock::new();
        let build = BUILD.get_or_init(|| {
            let wide = |s: &str| s.encode_utf16().chain(Some(0)).collect::<Vec<_>>();
            let mut buffer = [0u16; 32];
            let mut size = std::mem::size_of_val(&buffer) as u32;
            let rc = unsafe {
                RegGetValueW(
                    HKEY_LOCAL_MACHINE,
                    wide("SOFTWARE\\Microsoft\\Windows NT\\CurrentVersion").as_ptr(),
                    wide("CurrentBuildNumber").as_ptr(),
                    RRF_RT_REG_SZ | RRF_SUBKEY_WOW6464KEY,
                    std::ptr::null_mut(),
                    buffer.as_mut_ptr().cast(),
                    &mut size,
                )
            };
            if rc == 0 && (2..=64).contains(&size) && size.is_multiple_of(2) {
                if let Ok(text) = String::from_utf16(&buffer[..size as usize / 2 - 1]) {
                    if !text.is_empty() && text.bytes().all(|b| b.is_ascii_digit()) {
                        return format!("Windows Build {text}");
                    }
                }
            }
            "Windows Build Unknown".into()
        });
        format!("{build}  ·  Host {host} / App {app}  ·  {cpu} CPU Threads  ·  {ram}")
    }
    #[cfg(not(windows))]
    {
        format!(
            "{}  ·  App {app}  ·  {cpu} CPU Threads",
            std::env::consts::OS
        )
    }
}
