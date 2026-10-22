//! Windows implementations: registry access, process launching, elevation.

use super::Registry;
use crate::model::{fold, KeyData, RegValue};
use anyhow::{anyhow, Context, Result};
use std::ffi::OsStr;
use std::os::windows::ffi::OsStrExt;
use std::path::Path;
use windows_sys::Win32::System::Registry::{
    RegCloseKey, RegCreateKeyExW, RegDeleteTreeW, RegDeleteValueW, RegEnumKeyExW, RegEnumValueW,
    RegOpenKeyExW, RegQueryInfoKeyW, HKEY, HKEY_CLASSES_ROOT, HKEY_CURRENT_CONFIG,
    HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, HKEY_USERS, KEY_ALL_ACCESS, KEY_ENUMERATE_SUB_KEYS,
    KEY_NOTIFY, KEY_QUERY_VALUE, KEY_READ, KEY_SET_VALUE, KEY_WOW64_64KEY, REG_OPTION_NON_VOLATILE,
};

use windows_sys::Win32::Foundation::{
    CloseHandle, GetLastError, HANDLE, WAIT_FAILED, WAIT_OBJECT_0,
};

/// A kernel handle that is closed when it goes out of scope, on every path
/// (early returns, errors, panics).
struct OwnedHandle(HANDLE);

impl OwnedHandle {
    /// Wrap a handle returned by Windows; `None` if the call failed.
    fn new(h: HANDLE) -> Option<Self> {
        (!h.is_null() && h != INVALID_HANDLE).then_some(OwnedHandle(h))
    }
    fn get(&self) -> HANDLE {
        self.0
    }
}

impl Drop for OwnedHandle {
    fn drop(&mut self) {
        unsafe { CloseHandle(self.0) };
    }
}

const INVALID_HANDLE: HANDLE = -1isize as HANDLE;
use windows_sys::Win32::Security::{
    GetTokenInformation, TokenElevation, TOKEN_ELEVATION, TOKEN_QUERY,
};
use windows_sys::Win32::Storage::FileSystem::{MoveFileExW, MOVEFILE_DELAY_UNTIL_REBOOT};
use windows_sys::Win32::System::Registry::RegSetValueExW;
use windows_sys::Win32::System::Threading::{
    CreateProcessW, GetCurrentProcess, GetExitCodeProcess, OpenProcessToken, WaitForSingleObject,
    PROCESS_INFORMATION, STARTUPINFOW,
};
use windows_sys::Win32::UI::Shell::{
    ShellExecuteExW, ShellExecuteW, SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW,
};
use windows_sys::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

const ERROR_ELEVATION_REQUIRED: u32 = 740;

pub fn wide(s: &str) -> Vec<u16> {
    OsStr::new(s)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect()
}

fn hive(short: &str) -> HKEY {
    match short {
        "HKLM" => HKEY_LOCAL_MACHINE,
        "HKCU" => HKEY_CURRENT_USER,
        "HKU" => HKEY_USERS,
        "HKCR" => HKEY_CLASSES_ROOT,
        _ => HKEY_CURRENT_CONFIG,
    }
}

/// An open registry key, closed when dropped (hive roots are never closed).
struct Key {
    h: HKEY,
    owned: bool,
}

impl Drop for Key {
    fn drop(&mut self) {
        if self.owned {
            unsafe { RegCloseKey(self.h) };
        }
    }
}

fn os_err(rc: u32) -> std::io::Error {
    std::io::Error::from_raw_os_error(rc as i32)
}

impl Key {
    fn raw_handle(&self) -> HKEY {
        self.h
    }

    /// Size the buffers for enumerating this key: (longest subkey name,
    /// longest value name, largest value data), in characters / bytes.
    fn sizes(&self) -> (usize, usize, usize) {
        let (mut sub, mut name, mut data) = (0u32, 0u32, 0u32);
        unsafe {
            RegQueryInfoKeyW(
                self.h,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null(),
                std::ptr::null_mut(),
                &mut sub,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                &mut name,
                &mut data,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            )
        };
        (sub as usize + 1, name as usize + 1, data as usize)
    }

    fn values(&self) -> Vec<RegValue> {
        const ERROR_MORE_DATA: u32 = 234;
        let (_, n, d) = self.sizes();
        let mut name = vec![0u16; n.max(256)];
        let mut data = vec![0u8; d.max(256)];
        let mut out = Vec::new();
        let mut i = 0u32;
        loop {
            let mut nlen = name.len() as u32;
            let mut dlen = data.len() as u32;
            let mut kind = 0u32;
            let rc = unsafe {
                RegEnumValueW(
                    self.h,
                    i,
                    name.as_mut_ptr(),
                    &mut nlen,
                    std::ptr::null(),
                    &mut kind,
                    data.as_mut_ptr(),
                    &mut dlen,
                )
            };
            if rc == ERROR_MORE_DATA {
                let grow = (name.len() * 2).max(nlen as usize + 1);
                name.resize(grow, 0);
                let grow = (data.len() * 2).max(dlen as usize);
                data.resize(grow, 0);
                continue;
            }
            if rc != 0 {
                break;
            }
            out.push(RegValue {
                name: String::from_utf16_lossy(&name[..nlen as usize]),
                kind,
                data: data[..dlen as usize].to_vec(),
            });
            i += 1;
        }
        out
    }

    fn subkey_names(&self) -> Vec<String> {
        const ERROR_MORE_DATA: u32 = 234;
        let (s, _, _) = self.sizes();
        let mut name = vec![0u16; s.max(256)];
        let mut out = Vec::new();
        let mut i = 0u32;
        loop {
            let mut len = name.len() as u32;
            let rc = unsafe {
                RegEnumKeyExW(
                    self.h,
                    i,
                    name.as_mut_ptr(),
                    &mut len,
                    std::ptr::null(),
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                )
            };
            if rc == ERROR_MORE_DATA {
                let grow = name.len() * 2;
                name.resize(grow, 0);
                continue;
            }
            if rc != 0 {
                break;
            }
            out.push(String::from_utf16_lossy(&name[..len as usize]));
            i += 1;
        }
        out
    }
}

/// Open `HIVE\sub\key` in the 64-bit view.
fn open(path: &str, access: u32) -> std::io::Result<Key> {
    let (h, rest) = super::split_hive(path)
        .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, "unknown hive"))?;
    let root = hive(h);
    if rest.is_empty() {
        return Ok(Key {
            h: root,
            owned: false,
        });
    }
    let sub = wide(rest);
    let mut k: HKEY = std::ptr::null_mut();
    let rc = unsafe { RegOpenKeyExW(root, sub.as_ptr(), 0, access | KEY_WOW64_64KEY, &mut k) };
    if rc != 0 {
        return Err(os_err(rc));
    }
    Ok(Key { h: k, owned: true })
}

/// Create (or open) `HIVE\sub\key` and any missing parents.
fn create(path: &str, access: u32) -> std::io::Result<Key> {
    let (h, rest) = super::split_hive(path)
        .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, "unknown hive"))?;
    let root = hive(h);
    if rest.is_empty() {
        return Ok(Key {
            h: root,
            owned: false,
        });
    }
    let sub = wide(rest);
    let mut k: HKEY = std::ptr::null_mut();
    let rc = unsafe {
        RegCreateKeyExW(
            root,
            sub.as_ptr(),
            0,
            std::ptr::null(),
            REG_OPTION_NON_VOLATILE,
            access | KEY_WOW64_64KEY,
            std::ptr::null(),
            &mut k,
            std::ptr::null_mut(),
        )
    };
    if rc != 0 {
        return Err(os_err(rc));
    }
    Ok(Key { h: k, owned: true })
}

/// The real Windows registry (64-bit view).
pub struct WinRegistry;

impl Registry for WinRegistry {
    fn capture(
        &self,
        root: &str,
        excl: &crate::util::Exclusions,
        snap: &mut crate::model::Snapshot,
        progress: &crate::snapshot::Progress,
    ) {
        self.capture_native(root, excl, snap, progress);
    }

    fn exists(&self, path: &str) -> bool {
        open(path, KEY_READ).is_ok()
    }

    fn read_key(&self, path: &str) -> Option<KeyData> {
        let key = open(path, KEY_QUERY_VALUE).ok()?;
        let mut kd = KeyData::new(path);
        for rv in key.values() {
            kd.values.insert(fold(&rv.name), rv);
        }
        Some(kd)
    }

    fn subkeys(&self, path: &str) -> Vec<String> {
        match open(path, KEY_ENUMERATE_SUB_KEYS) {
            Ok(k) => k.subkey_names(),
            Err(_) => Vec::new(),
        }
    }

    fn create_key(&self, path: &str) -> Result<()> {
        super::split_hive(path).ok_or_else(|| anyhow!("unknown hive: {path}"))?;
        create(path, KEY_ALL_ACCESS).with_context(|| format!("create {path}"))?;
        Ok(())
    }

    fn set_value(&self, key: &str, value: &RegValue) -> Result<()> {
        self.create_key(key)?;
        let k = open(key, KEY_SET_VALUE).with_context(|| format!("open {key}"))?;
        let name = wide(&value.name);
        // Raw write keeps the exact original type, including unusual ones.
        let rc = unsafe {
            RegSetValueExW(
                k.raw_handle() as _,
                name.as_ptr(),
                0,
                value.kind,
                value.data.as_ptr(),
                value.data.len() as u32,
            )
        };
        if rc != 0 {
            return Err(anyhow!(
                "RegSetValueEx {key}\\{} failed: {}",
                value.name,
                std::io::Error::from_raw_os_error(rc as i32)
            ));
        }
        Ok(())
    }

    fn delete_value(&self, key: &str, name: &str) -> Result<()> {
        let k = open(key, KEY_SET_VALUE).with_context(|| format!("open {key}"))?;
        let w = wide(name);
        let rc = unsafe { RegDeleteValueW(k.raw_handle(), w.as_ptr()) };
        if rc != 0 {
            return Err(os_err(rc)).with_context(|| format!("delete value {key}\\{name}"));
        }
        Ok(())
    }

    fn delete_tree(&self, path: &str) -> Result<()> {
        let (h, rest) = super::split_hive(path).ok_or_else(|| anyhow!("unknown hive: {path}"))?;
        if rest.is_empty() {
            return Err(anyhow!("refusing to delete a hive root"));
        }
        // The last line of defence, whoever asked.
        if crate::util::is_protected_key(path) {
            return Err(anyhow!("refusing to delete protected key {path}"));
        }
        let (parent, leaf) = match rest.rsplit_once('\\') {
            Some((p, l)) => (format!("{h}\\{p}"), l.to_string()),
            None => (h.to_string(), rest.to_string()),
        };
        let pk = open(&parent, KEY_ALL_ACCESS).with_context(|| format!("open {parent}"))?;
        let w = wide(&leaf);
        let rc = unsafe { RegDeleteTreeW(pk.raw_handle(), w.as_ptr()) };
        if rc != 0 {
            return Err(os_err(rc)).with_context(|| format!("delete {path}"));
        }
        Ok(())
    }
}

pub fn delete_on_reboot(path: &Path) -> Result<()> {
    let p = wide(&path.to_string_lossy());
    let ok = unsafe { MoveFileExW(p.as_ptr(), std::ptr::null(), MOVEFILE_DELAY_UNTIL_REBOOT) };
    if ok == 0 {
        return Err(anyhow!(
            "MoveFileEx failed: {}",
            std::io::Error::last_os_error()
        ));
    }
    Ok(())
}

pub fn is_elevated() -> bool {
    unsafe {
        let mut token: HANDLE = std::ptr::null_mut();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) == 0 {
            return false;
        }
        let Some(token) = OwnedHandle::new(token) else {
            return false;
        };
        let mut elev = TOKEN_ELEVATION { TokenIsElevated: 0 };
        let mut len = 0u32;
        let ok = GetTokenInformation(
            token.get(),
            TokenElevation,
            &mut elev as *mut _ as *mut _,
            std::mem::size_of::<TOKEN_ELEVATION>() as u32,
            &mut len,
        );
        ok != 0 && elev.TokenIsElevated != 0
    }
}

/// A process started by SetupScope (installer, uninstaller, tool).
pub struct ChildProcess {
    handle: HANDLE,
}

unsafe impl Send for ChildProcess {}

impl ChildProcess {
    /// `Some(exit_code)` once the process has ended.
    pub fn try_wait(&self) -> Option<u32> {
        if self.handle.is_null() {
            return Some(0);
        }
        unsafe {
            match WaitForSingleObject(self.handle, 0) {
                WAIT_OBJECT_0 => {
                    let mut code = 0u32;
                    if GetExitCodeProcess(self.handle, &mut code) == 0 {
                        code = GetLastError();
                    }
                    Some(code)
                }
                // The handle can't be waited on: report it as ended with the
                // error, rather than waiting forever.
                WAIT_FAILED => Some(GetLastError().max(1)),
                _ => None,
            }
        }
    }

    pub fn wait(&self) -> u32 {
        loop {
            if let Some(c) = self.try_wait() {
                return c;
            }
            std::thread::sleep(std::time::Duration::from_millis(250));
        }
    }
}

impl Drop for ChildProcess {
    fn drop(&mut self) {
        if !self.handle.is_null() {
            unsafe { CloseHandle(self.handle) };
        }
    }
}

/// Run a raw command line exactly as Windows would (like the Run dialog),
/// elevating through UAC if the target requires it.
pub fn spawn_command_line(cmdline: &str) -> Result<ChildProcess> {
    let mut buf = wide(cmdline);
    unsafe {
        let mut si: STARTUPINFOW = std::mem::zeroed();
        si.cb = std::mem::size_of::<STARTUPINFOW>() as u32;
        let mut pi: PROCESS_INFORMATION = std::mem::zeroed();
        let ok = CreateProcessW(
            std::ptr::null(),
            buf.as_mut_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            0,
            0,
            std::ptr::null(),
            std::ptr::null(),
            &si,
            &mut pi,
        );
        if ok != 0 {
            CloseHandle(pi.hThread);
            return Ok(ChildProcess {
                handle: pi.hProcess,
            });
        }
        let err = GetLastError();
        if err == ERROR_ELEVATION_REQUIRED {
            let (prog, args) = crate::util::split_command_line(cmdline);
            return shell_execute_wait(&prog, &args, true);
        }
        Err(anyhow!(
            "could not start `{cmdline}`: {}",
            std::io::Error::from_raw_os_error(err as i32)
        ))
    }
}

/// ShellExecuteEx with an optional "runas" verb, returning a waitable handle.
pub fn shell_execute_wait(file: &str, params: &str, elevate: bool) -> Result<ChildProcess> {
    let verb = wide(if elevate { "runas" } else { "open" });
    let f = wide(file);
    let p = wide(params);
    unsafe {
        let mut info: SHELLEXECUTEINFOW = std::mem::zeroed();
        info.cbSize = std::mem::size_of::<SHELLEXECUTEINFOW>() as u32;
        info.fMask = SEE_MASK_NOCLOSEPROCESS;
        info.lpVerb = verb.as_ptr();
        info.lpFile = f.as_ptr();
        info.lpParameters = if params.is_empty() {
            std::ptr::null()
        } else {
            p.as_ptr()
        };
        info.nShow = SW_SHOWNORMAL;
        if ShellExecuteExW(&mut info) == 0 {
            return Err(anyhow!(
                "could not start {file}: {}",
                std::io::Error::last_os_error()
            ));
        }
        Ok(ChildProcess {
            handle: info.hProcess,
        })
    }
}

/// Open a file, folder or URL with its default handler.
pub fn shell_open(target: &str) {
    let verb = wide("open");
    let t = wide(target);
    unsafe {
        ShellExecuteW(
            std::ptr::null_mut(),
            verb.as_ptr(),
            t.as_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            SW_SHOWNORMAL,
        );
    }
}

/// Restart SetupScope elevated with the given arguments. Returns true if the new
/// process launched.
pub fn relaunch_elevated(args: &[String]) -> bool {
    let exe = std::env::current_exe()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_default();
    let params = args
        .iter()
        .map(|a| {
            if a.contains(' ') || a.is_empty() {
                format!("\"{a}\"")
            } else {
                a.clone()
            }
        })
        .collect::<Vec<_>>()
        .join(" ");
    shell_execute_wait(&exe, &params, true).is_ok()
}

/// Attach to the parent console so CLI output is visible from cmd/PowerShell
/// even though the exe uses the GUI subsystem.
pub fn attach_parent_console() {
    use windows_sys::Win32::System::Console::{AttachConsole, ATTACH_PARENT_PROCESS};
    unsafe {
        AttachConsole(ATTACH_PARENT_PROCESS);
    }
}

/// Open Registry Editor at the given key.
pub fn open_in_regedit(path: &str) -> Result<()> {
    let (h, rest) = super::split_hive(path).ok_or_else(|| anyhow!("unknown hive"))?;
    let long = super::HIVES
        .iter()
        .find(|(s, _)| *s == h)
        .map(|(_, l)| *l)
        .unwrap_or(h);
    let target = if rest.is_empty() {
        format!("Computer\\{long}")
    } else {
        format!("Computer\\{long}\\{rest}")
    };
    WinRegistry.set_value(
        "HKCU\\Software\\Microsoft\\Windows\\CurrentVersion\\Applets\\Regedit",
        &RegValue::string("LastKey", &target),
    )?;
    // Regedit is single-instance; /m opens a new window that honours LastKey.
    shell_execute_wait("regedit.exe", "/m", false)?;
    Ok(())
}

/// (items, bytes) in the Recycle Bin across all drives.
pub fn recycle_bin_info() -> (u64, u64) {
    use windows_sys::Win32::UI::Shell::{SHQueryRecycleBinW, SHQUERYRBINFO};
    unsafe {
        let mut info: SHQUERYRBINFO = std::mem::zeroed();
        info.cbSize = std::mem::size_of::<SHQUERYRBINFO>() as u32;
        if SHQueryRecycleBinW(std::ptr::null(), &mut info) == 0 {
            (info.i64NumItems.max(0) as u64, info.i64Size.max(0) as u64)
        } else {
            (0, 0)
        }
    }
}

pub fn empty_recycle_bin() -> bool {
    use windows_sys::Win32::UI::Shell::{
        SHEmptyRecycleBinW, SHERB_NOCONFIRMATION, SHERB_NOPROGRESSUI, SHERB_NOSOUND,
    };
    unsafe {
        SHEmptyRecycleBinW(
            std::ptr::null_mut(),
            std::ptr::null(),
            SHERB_NOCONFIRMATION | SHERB_NOPROGRESSUI | SHERB_NOSOUND,
        ) == 0
    }
}

/// Windows display locale, e.g. `de-DE`.
pub fn user_locale() -> String {
    use windows_sys::Win32::Globalization::GetUserDefaultLocaleName;
    let mut buf = [0u16; 85];
    let n = unsafe { GetUserDefaultLocaleName(buf.as_mut_ptr(), buf.len() as i32) };
    if n > 1 {
        String::from_utf16_lossy(&buf[..(n as usize - 1)])
    } else {
        "en-US".into()
    }
}

/// Give every visible top-level window of this process a native Windows 11
/// border colour and a title bar that matches the light/dark theme.
pub fn apply_window_style(border_rgb: (u8, u8, u8), dark: bool) {
    use windows_sys::Win32::Foundation::{BOOL, HWND, LPARAM};
    use windows_sys::Win32::Graphics::Dwm::DwmSetWindowAttribute;
    use windows_sys::Win32::System::Threading::GetCurrentProcessId;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        EnumWindows, GetWindowThreadProcessId, IsWindowVisible,
    };

    const DWMWA_USE_IMMERSIVE_DARK_MODE: u32 = 20;
    const DWMWA_BORDER_COLOR: u32 = 34;

    struct Ctx {
        pid: u32,
        color: u32,
        dark: i32,
    }
    unsafe extern "system" fn each(hwnd: HWND, lparam: LPARAM) -> BOOL {
        let ctx = &*(lparam as *const Ctx);
        let mut pid = 0u32;
        GetWindowThreadProcessId(hwnd, &mut pid);
        if pid == ctx.pid && IsWindowVisible(hwnd) != 0 {
            DwmSetWindowAttribute(
                hwnd,
                DWMWA_BORDER_COLOR,
                &ctx.color as *const u32 as *const _,
                4,
            );
            DwmSetWindowAttribute(
                hwnd,
                DWMWA_USE_IMMERSIVE_DARK_MODE,
                &ctx.dark as *const i32 as *const _,
                4,
            );
        }
        1
    }
    let (r, g, b) = border_rgb;
    // COLORREF is 0x00BBGGRR.
    let ctx = Ctx {
        pid: unsafe { GetCurrentProcessId() },
        color: (b as u32) << 16 | (g as u32) << 8 | r as u32,
        dark: dark as i32,
    };
    unsafe {
        EnumWindows(Some(each), &ctx as *const Ctx as LPARAM);
    }
}

/// A plain Windows message box (used only when the window can't be created).
pub fn message_box(title: &str, text: &str) -> i32 {
    use windows_sys::Win32::UI::WindowsAndMessaging::{MessageBoxW, MB_ICONERROR, MB_OK};
    let (t, m) = (wide(title), wide(text));
    unsafe {
        MessageBoxW(
            std::ptr::null_mut(),
            m.as_ptr(),
            t.as_ptr(),
            MB_OK | MB_ICONERROR,
        )
    }
}

/// Windows' "Animation effects" accessibility setting.
pub fn client_area_animation() -> bool {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        SystemParametersInfoW, SPI_GETCLIENTAREAANIMATION,
    };
    let mut on: i32 = 1;
    let ok = unsafe {
        SystemParametersInfoW(
            SPI_GETCLIENTAREAANIMATION,
            0,
            &mut on as *mut i32 as *mut _,
            0,
        )
    };
    ok == 0 || on != 0
}

/// An information message box that stays on top until OK is clicked.
pub fn info_box(title: &str, text: &str) -> i32 {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        MessageBoxW, MB_ICONINFORMATION, MB_OK, MB_SETFOREGROUND, MB_TOPMOST,
    };
    let (t, m) = (wide(title), wide(text));
    unsafe {
        MessageBoxW(
            std::ptr::null_mut(),
            m.as_ptr(),
            t.as_ptr(),
            MB_OK | MB_ICONINFORMATION | MB_SETFOREGROUND | MB_TOPMOST,
        )
    }
}

/// Move a file or folder to the Recycle Bin, silently.
pub fn recycle(path: &Path) -> Result<()> {
    use windows_sys::Win32::UI::Shell::{
        SHFileOperationW, FOF_ALLOWUNDO, FOF_NOCONFIRMATION, FOF_NOERRORUI, FOF_SILENT,
        FOF_WANTNUKEWARNING, FO_DELETE, SHFILEOPSTRUCTW,
    };
    // pFrom must be double-NUL terminated.
    let mut from = wide(&path.to_string_lossy());
    from.push(0);
    unsafe {
        let mut op: SHFILEOPSTRUCTW = std::mem::zeroed();
        op.wFunc = FO_DELETE as _;
        op.pFrom = from.as_ptr();
        op.fFlags =
            (FOF_ALLOWUNDO | FOF_NOCONFIRMATION | FOF_SILENT | FOF_NOERRORUI | FOF_WANTNUKEWARNING)
                as _;
        let rc = SHFileOperationW(&mut op);
        if rc != 0 || op.fAnyOperationsAborted != 0 || path.exists() {
            return Err(anyhow!(
                "could not move {} to the Recycle Bin (code {rc})",
                path.display()
            ));
        }
    }
    Ok(())
}

/// Extract an icon from an .exe, .dll or .ico as RGBA pixels.
pub fn icon_rgba(path: &str, index: i32, size: u32) -> Option<(u32, u32, Vec<u8>)> {
    use windows_sys::Win32::Graphics::Gdi::{
        CreateCompatibleDC, DeleteDC, DeleteObject, GetDIBits, BITMAPINFO, BITMAPINFOHEADER,
        BI_RGB, DIB_RGB_COLORS,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        DestroyIcon, GetIconInfo, PrivateExtractIconsW, HICON, ICONINFO,
    };
    if !Path::new(path).exists() {
        return None;
    }
    let wpath = wide(path);
    let mut hicon: HICON = std::ptr::null_mut();
    let mut id = 0u32;
    unsafe {
        let n = PrivateExtractIconsW(
            wpath.as_ptr(),
            index,
            size as i32,
            size as i32,
            &mut hicon,
            &mut id,
            1,
            0,
        );
        if n == 0 || n == u32::MAX || hicon.is_null() {
            return None;
        }
        let mut info: ICONINFO = std::mem::zeroed();
        if GetIconInfo(hicon, &mut info) == 0 {
            DestroyIcon(hicon);
            return None;
        }
        let dc = CreateCompatibleDC(std::ptr::null_mut());
        let mut bmi: BITMAPINFO = std::mem::zeroed();
        bmi.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
        bmi.bmiHeader.biWidth = size as i32;
        bmi.bmiHeader.biHeight = -(size as i32); // top-down
        bmi.bmiHeader.biPlanes = 1;
        bmi.bmiHeader.biBitCount = 32;
        bmi.bmiHeader.biCompression = BI_RGB;
        let mut px = vec![0u8; (size * size * 4) as usize];
        let ok = !info.hbmColor.is_null()
            && GetDIBits(
                dc,
                info.hbmColor,
                0,
                size,
                px.as_mut_ptr() as *mut _,
                &mut bmi,
                DIB_RGB_COLORS,
            ) != 0;
        let has_alpha = px.chunks_exact(4).any(|p| p[3] != 0);
        if ok && !has_alpha {
            // Old-style icon without alpha: derive transparency from the mask.
            let mut mask = vec![0u8; (size * size * 4) as usize];
            if GetDIBits(
                dc,
                info.hbmMask,
                0,
                size,
                mask.as_mut_ptr() as *mut _,
                &mut bmi,
                DIB_RGB_COLORS,
            ) != 0
            {
                for (p, m) in px.chunks_exact_mut(4).zip(mask.chunks_exact(4)) {
                    p[3] = if m[0] == 0 { 255 } else { 0 };
                }
            }
        }
        DeleteDC(dc);
        if !info.hbmColor.is_null() {
            DeleteObject(info.hbmColor);
        }
        if !info.hbmMask.is_null() {
            DeleteObject(info.hbmMask);
        }
        DestroyIcon(hicon);
        if !ok {
            return None;
        }
        // BGRA -> RGBA
        for p in px.chunks_exact_mut(4) {
            p.swap(0, 2);
        }
        Some((size, size, px))
    }
}

/// Verify a file's Authenticode signature offline. Returns the WinVerifyTrust
/// result (0 = valid) and the signer's display name when it can be read.
pub fn authenticode(path: &str) -> (i32, Option<String>) {
    use windows_sys::Win32::Security::WinTrust::{
        WinVerifyTrust, WINTRUST_ACTION_GENERIC_VERIFY_V2, WINTRUST_DATA, WINTRUST_DATA_0,
        WINTRUST_FILE_INFO, WTD_CACHE_ONLY_URL_RETRIEVAL, WTD_CHOICE_FILE, WTD_REVOKE_NONE,
        WTD_STATEACTION_CLOSE, WTD_STATEACTION_VERIFY, WTD_UI_NONE,
    };
    let wpath = wide(path);
    let code = unsafe {
        let mut file: WINTRUST_FILE_INFO = std::mem::zeroed();
        file.cbStruct = std::mem::size_of::<WINTRUST_FILE_INFO>() as u32;
        file.pcwszFilePath = wpath.as_ptr();
        let mut data: WINTRUST_DATA = std::mem::zeroed();
        data.cbStruct = std::mem::size_of::<WINTRUST_DATA>() as u32;
        data.dwUIChoice = WTD_UI_NONE;
        data.fdwRevocationChecks = WTD_REVOKE_NONE;
        data.dwUnionChoice = WTD_CHOICE_FILE;
        data.Anonymous = WINTRUST_DATA_0 { pFile: &mut file };
        data.dwStateAction = WTD_STATEACTION_VERIFY;
        data.dwProvFlags = WTD_CACHE_ONLY_URL_RETRIEVAL;
        let mut action = WINTRUST_ACTION_GENERIC_VERIFY_V2;
        let rc = WinVerifyTrust(
            std::ptr::null_mut(),
            &mut action,
            &mut data as *mut _ as *mut _,
        );
        data.dwStateAction = WTD_STATEACTION_CLOSE;
        WinVerifyTrust(
            std::ptr::null_mut(),
            &mut action,
            &mut data as *mut _ as *mut _,
        );
        rc
    };
    (code, signer_name(&wpath))
}

/// Read the signer's display name from a file's embedded signature.
fn signer_name(wpath: &[u16]) -> Option<String> {
    signer_attribute(wpath, false)
}

/// Embedded signing certificate organization; caller must also verify trust.
pub fn authenticode_organization(path: &str) -> Option<String> {
    signer_attribute(&wide(path), true)
}

fn signer_attribute(wpath: &[u16], organization: bool) -> Option<String> {
    use windows_sys::Win32::Security::Cryptography::{
        szOID_ORGANIZATION_NAME, CertCloseStore, CertFindCertificateInStore,
        CertFreeCertificateContext, CertGetNameStringW, CryptMsgClose, CryptMsgGetParam,
        CryptQueryObject, CERT_FIND_SUBJECT_CERT, CERT_INFO, CERT_NAME_ATTR_TYPE,
        CERT_NAME_SIMPLE_DISPLAY_TYPE, CERT_QUERY_CONTENT_FLAG_PKCS7_SIGNED_EMBED,
        CERT_QUERY_FORMAT_FLAG_BINARY, CERT_QUERY_OBJECT_FILE, CMSG_SIGNER_INFO,
        CMSG_SIGNER_INFO_PARAM, PKCS_7_ASN_ENCODING, X509_ASN_ENCODING,
    };
    unsafe {
        let mut encoding = 0u32;
        let mut content = 0u32;
        let mut format = 0u32;
        let mut store = std::ptr::null_mut();
        let mut msg = std::ptr::null_mut();
        let ok = CryptQueryObject(
            CERT_QUERY_OBJECT_FILE,
            wpath.as_ptr() as *const _,
            CERT_QUERY_CONTENT_FLAG_PKCS7_SIGNED_EMBED,
            CERT_QUERY_FORMAT_FLAG_BINARY,
            0,
            &mut encoding,
            &mut content,
            &mut format,
            &mut store,
            &mut msg,
            std::ptr::null_mut(),
        );
        if ok == 0 {
            return None;
        }
        let mut name = None;
        let mut size = 0u32;
        if CryptMsgGetParam(
            msg,
            CMSG_SIGNER_INFO_PARAM,
            0,
            std::ptr::null_mut(),
            &mut size,
        ) != 0
            && size > 0
            && size <= 1024 * 1024
        {
            // u64 buffer keeps the struct properly aligned.
            let mut buf = vec![0u64; (size as usize).div_ceil(8)];
            if CryptMsgGetParam(
                msg,
                CMSG_SIGNER_INFO_PARAM,
                0,
                buf.as_mut_ptr() as *mut _,
                &mut size,
            ) != 0
            {
                let info = &*(buf.as_ptr() as *const CMSG_SIGNER_INFO);
                let mut cert_info: CERT_INFO = std::mem::zeroed();
                cert_info.Issuer = info.Issuer;
                cert_info.SerialNumber = info.SerialNumber;
                let cert = CertFindCertificateInStore(
                    store,
                    X509_ASN_ENCODING | PKCS_7_ASN_ENCODING,
                    0,
                    CERT_FIND_SUBJECT_CERT,
                    &cert_info as *const _ as *const _,
                    std::ptr::null(),
                );
                if !cert.is_null() {
                    let mut out = [0u16; 256];
                    let n = CertGetNameStringW(
                        cert,
                        if organization {
                            CERT_NAME_ATTR_TYPE
                        } else {
                            CERT_NAME_SIMPLE_DISPLAY_TYPE
                        },
                        0,
                        if organization {
                            szOID_ORGANIZATION_NAME.cast()
                        } else {
                            std::ptr::null()
                        },
                        out.as_mut_ptr(),
                        out.len() as u32,
                    );
                    if n > 1 {
                        name = Some(String::from_utf16_lossy(&out[..(n as usize - 1)]));
                    }
                    CertFreeCertificateContext(cert);
                }
            }
        }
        CryptMsgClose(msg);
        CertCloseStore(store, 0);
        name
    }
}

/// True while a Windows Installer (MSI) operation is running anywhere on the
/// system. Only one can run at a time; starting another fails with 1618.
pub fn installer_busy() -> bool {
    use windows_sys::Win32::System::Threading::OpenMutexW;
    const SYNCHRONIZE: u32 = 0x0010_0000;
    let name: Vec<u16> = "Global\\_MSIExecute\0".encode_utf16().collect();
    unsafe { OwnedHandle::new(OpenMutexW(SYNCHRONIZE, 0, name.as_ptr())).is_some() }
}

/// Whether a process with this id is still running.
pub fn process_alive(pid: u32) -> bool {
    use windows_sys::Win32::System::Threading::{
        GetExitCodeProcess, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
    };
    const STILL_ACTIVE: u32 = 259;
    unsafe {
        let Some(h) = OwnedHandle::new(OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid))
        else {
            return false;
        };
        let mut code = 0u32;
        GetExitCodeProcess(h.get(), &mut code) != 0 && code == STILL_ACTIVE
    }
}

/// Every running process this user can see: program file, memory in use
/// and total CPU time. Used for the resource-use history.
pub fn process_samples() -> Vec<crate::usage::ProcSample> {
    use windows_sys::Win32::Foundation::FILETIME;
    use windows_sys::Win32::System::ProcessStatus::{
        K32EnumProcesses, K32GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS,
    };
    use windows_sys::Win32::System::Threading::{
        GetProcessTimes, OpenProcess, QueryFullProcessImageNameW, PROCESS_QUERY_LIMITED_INFORMATION,
    };
    // Grow the buffer until the list fits (a full buffer may be truncated).
    let mut pids = vec![0u32; 4096];
    loop {
        let mut needed = 0u32;
        let bytes = (pids.len() * 4) as u32;
        if unsafe { K32EnumProcesses(pids.as_mut_ptr(), bytes, &mut needed) } == 0 {
            return Vec::new();
        }
        if needed < bytes || pids.len() >= 1 << 20 {
            pids.truncate(needed as usize / 4);
            break;
        }
        pids = vec![0u32; pids.len() * 2];
    }
    let ft = |f: FILETIME| ((f.dwHighDateTime as u64) << 32) | f.dwLowDateTime as u64;
    let mut out = Vec::new();
    for pid in pids.into_iter().filter(|&p| p > 4) {
        unsafe {
            let Some(owned) =
                OwnedHandle::new(OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid))
            else {
                continue;
            };
            let h = owned.get();
            let mut buf = [0u16; 1024];
            let mut len = buf.len() as u32;
            let path = if QueryFullProcessImageNameW(h, 0, buf.as_mut_ptr(), &mut len) != 0 {
                String::from_utf16_lossy(&buf[..(len as usize).min(buf.len())])
            } else {
                String::new()
            };
            let mut mem: PROCESS_MEMORY_COUNTERS = std::mem::zeroed();
            mem.cb = std::mem::size_of::<PROCESS_MEMORY_COUNTERS>() as u32;
            let ws = if K32GetProcessMemoryInfo(h, &mut mem, mem.cb) != 0 {
                mem.WorkingSetSize as u64
            } else {
                0
            };
            let (mut c, mut e, mut k, mut u): (FILETIME, FILETIME, FILETIME, FILETIME) =
                std::mem::zeroed();
            let cpu = if GetProcessTimes(h, &mut c, &mut e, &mut k, &mut u) != 0 {
                (ft(k) + ft(u)) / 10_000
            } else {
                0
            };
            drop(owned);
            if !path.is_empty() {
                out.push(crate::usage::ProcSample {
                    path,
                    working_set: ws,
                    cpu_ms: cpu,
                });
            }
        }
    }
    out
}

/// Run this process at background priority (low CPU and disk priority).
pub fn background_priority() {
    use windows_sys::Win32::System::Threading::{
        GetCurrentProcess, SetPriorityClass, PROCESS_MODE_BACKGROUND_BEGIN,
    };
    unsafe {
        SetPriorityClass(GetCurrentProcess(), PROCESS_MODE_BACKGROUND_BEGIN);
    }
}

/// Block until `stop` is set, calling `on_change` whenever anything under the
/// registry key `path` changes (keys added or removed, values written).
/// A manual-reset kernel event that can be shared between threads.
pub struct ManualEvent(HANDLE);

unsafe impl Send for ManualEvent {}
unsafe impl Sync for ManualEvent {}

impl ManualEvent {
    pub fn new() -> Self {
        use windows_sys::Win32::System::Threading::CreateEventW;
        ManualEvent(unsafe { CreateEventW(std::ptr::null(), 1, 0, std::ptr::null()) })
    }
    pub fn set(&self) {
        use windows_sys::Win32::System::Threading::SetEvent;
        if !self.0.is_null() {
            unsafe { SetEvent(self.0) };
        }
    }
    pub fn raw(&self) -> HANDLE {
        self.0
    }
}

impl Default for ManualEvent {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for ManualEvent {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe { CloseHandle(self.0) };
        }
    }
}

/// Wait for `event` or the stop signal, with no timeout. True if `event`
/// fired; false if stopped (or the wait failed).
fn wait_or_stop(event: HANDLE, stop: &super::StopSignal) -> bool {
    use windows_sys::Win32::System::Threading::{WaitForMultipleObjects, INFINITE};
    if stop.is_stopped() {
        return false;
    }
    let handles = [event, stop.event.raw()];
    let rc = unsafe { WaitForMultipleObjects(2, handles.as_ptr(), 0, INFINITE) };
    rc == WAIT_OBJECT_0 && !stop.is_stopped()
}

/// Call `on_change` whenever something under `path` changes, until stopped.
/// Blocks in the kernel between changes.
pub fn watch_registry(
    path: &str,
    stop: &super::StopSignal,
    mut on_change: impl FnMut(),
) -> Result<()> {
    use windows_sys::Win32::System::Registry::{
        RegNotifyChangeKeyValue, REG_NOTIFY_CHANGE_LAST_SET, REG_NOTIFY_CHANGE_NAME,
    };
    use windows_sys::Win32::System::Threading::CreateEventW;
    let key = open(path, KEY_NOTIFY)?;
    let raw = key.raw_handle() as windows_sys::Win32::System::Registry::HKEY;
    let Some(owned) =
        OwnedHandle::new(unsafe { CreateEventW(std::ptr::null(), 0, 0, std::ptr::null()) })
    else {
        return Err(anyhow!("could not create an event"));
    };
    loop {
        let rc = unsafe {
            RegNotifyChangeKeyValue(
                raw,
                1,
                REG_NOTIFY_CHANGE_NAME | REG_NOTIFY_CHANGE_LAST_SET,
                owned.get(),
                1,
            )
        };
        if rc != 0 {
            return Err(anyhow!(
                "registry notifications unavailable for {path} ({rc})"
            ));
        }
        if !wait_or_stop(owned.get(), stop) {
            return Ok(());
        }
        on_change();
    }
}

/// What happened to a file, from `ReadDirectoryChangesW`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FolderEvent {
    Added,
    Removed,
    Modified,
    Renamed,
}

/// Report every change under `dir` until stopped, through one 64 KB
/// buffer and one path buffer reused for every notification. Blocks in
/// the kernel between changes.
pub fn watch_folder(
    dir: &Path,
    recursive: bool,
    stop: &super::StopSignal,
    on_event: &mut dyn FnMut(FolderEvent, &str),
) -> Result<()> {
    use windows_sys::Win32::Storage::FileSystem::{
        CreateFileW, ReadDirectoryChangesW, FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OVERLAPPED,
        FILE_LIST_DIRECTORY, FILE_NOTIFY_CHANGE_CREATION, FILE_NOTIFY_CHANGE_DIR_NAME,
        FILE_NOTIFY_CHANGE_FILE_NAME, FILE_NOTIFY_CHANGE_LAST_WRITE, FILE_NOTIFY_CHANGE_SIZE,
        FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
    };
    use windows_sys::Win32::System::Threading::CreateEventW;
    use windows_sys::Win32::System::IO::{CancelIoEx, GetOverlappedResult, OVERLAPPED};
    let root = dir.to_string_lossy().trim_end_matches('\\').to_string();
    let w = wide(&root);
    let Some(h) = OwnedHandle::new(unsafe {
        CreateFileW(
            w.as_ptr(),
            FILE_LIST_DIRECTORY,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            std::ptr::null(),
            OPEN_EXISTING,
            FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OVERLAPPED,
            std::ptr::null_mut(),
        )
    }) else {
        return Err(anyhow!(
            "cannot watch {root}: {}",
            std::io::Error::last_os_error()
        ));
    };
    let Some(ev) =
        OwnedHandle::new(unsafe { CreateEventW(std::ptr::null(), 1, 0, std::ptr::null()) })
    else {
        return Err(anyhow!("could not create an event"));
    };
    // DWORD-aligned, as the API requires.
    let mut buf = vec![0u32; 16 * 1024];
    let mut path = String::with_capacity(512);
    let filter = FILE_NOTIFY_CHANGE_FILE_NAME
        | FILE_NOTIFY_CHANGE_DIR_NAME
        | FILE_NOTIFY_CHANGE_SIZE
        | FILE_NOTIFY_CHANGE_LAST_WRITE
        | FILE_NOTIFY_CHANGE_CREATION;
    loop {
        let mut ov: OVERLAPPED = unsafe { std::mem::zeroed() };
        ov.hEvent = ev.get();
        let ok = unsafe {
            ReadDirectoryChangesW(
                h.get(),
                buf.as_mut_ptr() as *mut core::ffi::c_void,
                (buf.len() * 4) as u32,
                recursive as i32,
                filter,
                std::ptr::null_mut(),
                &mut ov,
                None,
            )
        };
        if ok == 0 {
            return Err(anyhow!(
                "watching {root} failed: {}",
                std::io::Error::last_os_error()
            ));
        }
        if !wait_or_stop(ev.get(), stop) {
            unsafe {
                CancelIoEx(h.get(), &ov);
                let mut n = 0u32;
                GetOverlappedResult(h.get(), &ov, &mut n, 1);
            }
            return Ok(());
        }
        let mut n = 0u32;
        if unsafe { GetOverlappedResult(h.get(), &ov, &mut n, 0) } == 0 {
            return Err(anyhow!(
                "watching {root} failed: {}",
                std::io::Error::last_os_error()
            ));
        }
        // n == 0: too many changes at once for the buffer; they're lost
        // from the live view only (the final comparison has them all).
        let bytes = unsafe { std::slice::from_raw_parts(buf.as_ptr() as *const u8, n as usize) };
        let mut at = 0usize;
        while at + 12 <= bytes.len() {
            let rd = |o: usize| u32::from_le_bytes(bytes[at + o..at + o + 4].try_into().unwrap());
            let (next, action, len) = (rd(0) as usize, rd(4), rd(8) as usize);
            let name_end = (at + 12 + len).min(bytes.len());
            let name = &bytes[at + 12..name_end];
            path.clear();
            path.push_str(&root);
            path.push('\\');
            path.extend(
                char::decode_utf16(
                    name.chunks_exact(2)
                        .map(|c| u16::from_le_bytes([c[0], c[1]])),
                )
                .map(|r| r.unwrap_or('\u{FFFD}')),
            );
            let kind = match action {
                1 => Some(FolderEvent::Added),
                2 => Some(FolderEvent::Removed),
                3 => Some(FolderEvent::Modified),
                4 | 5 => Some(FolderEvent::Renamed),
                _ => None,
            };
            if let Some(k) = kind {
                on_event(k, &path);
            }
            if next == 0 {
                break;
            }
            at += next;
        }
    }
}

/// Stop Windows showing "There is no disk in the drive" and similar
/// dialogs when SetupScope touches an empty card reader or removed drive.
pub fn quiet_critical_errors() {
    #[link(name = "kernel32")]
    extern "system" {
        fn SetErrorMode(mode: u32) -> u32;
    }
    const SEM_FAILCRITICALERRORS: u32 = 0x0001;
    const SEM_NOOPENFILEERRORBOX: u32 = 0x8000;
    unsafe {
        SetErrorMode(SEM_FAILCRITICALERRORS | SEM_NOOPENFILEERRORBOX);
    }
}

/// Lower the current thread's CPU priority a little, so background work
/// never makes the window or other programs stutter.
pub fn lower_thread_priority() {
    use windows_sys::Win32::System::Threading::{
        GetCurrentThread, SetThreadPriority, THREAD_PRIORITY_BELOW_NORMAL,
    };
    unsafe {
        SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_BELOW_NORMAL);
    }
}

/// Delete one System Restore point (srclient.dll's SRRemoveRestorePoint).
pub fn remove_restore_point(sequence: u32) -> Result<()> {
    use windows_sys::Win32::Foundation::FreeLibrary;
    use windows_sys::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};
    let name = wide("srclient.dll");
    unsafe {
        let lib = LoadLibraryW(name.as_ptr());
        if lib.is_null() {
            return Err(anyhow!("System Restore is not available"));
        }
        let Some(f) = GetProcAddress(lib, b"SRRemoveRestorePoint\0".as_ptr()) else {
            FreeLibrary(lib);
            return Err(anyhow!("System Restore is not available"));
        };
        let remove: unsafe extern "system" fn(u32) -> u32 = std::mem::transmute(f);
        let rc = remove(sequence);
        FreeLibrary(lib);
        match rc {
            0 => Ok(()),
            code => Err(anyhow!(
                "Windows could not delete the restore point (error {code})"
            )),
        }
    }
}

/// Total size of a drive in bytes.
pub fn drive_size(root: &str) -> Option<u64> {
    use windows_sys::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;
    let w = wide(root);
    let (mut a, mut t, mut f) = (0u64, 0u64, 0u64);
    unsafe { (GetDiskFreeSpaceExW(w.as_ptr(), &mut a, &mut t, &mut f) != 0).then_some(t) }
}

// ---------------------------------------------------------------------------
// Files held open: Restart Manager
// ---------------------------------------------------------------------------

use windows_sys::Win32::System::RestartManager::{
    RmEndSession, RmGetList, RmRegisterResources, RmRestart, RmShutdown, RmStartSession,
    RM_PROCESS_INFO, RM_UNIQUE_PROCESS,
};

const ERROR_MORE_DATA: u32 = 234;
const RM_SESSION_KEY_LEN: usize = 32;
const RM_FORCE_SHUTDOWN: u32 = 0x1;
// RM_APP_TYPE values.
const RM_SERVICE: i32 = 3;
const RM_EXPLORER: i32 = 4;
const RM_CRITICAL: i32 = 1000;
const RM_MAIN_WINDOW: i32 = 1;
const RM_OTHER_WINDOW: i32 = 2;
/// Paths registered with one Restart Manager session at most.
const RM_MAX_FILES: usize = 4096;

/// A Restart Manager session, ended when dropped.
struct RmSession(u32);

impl RmSession {
    fn start() -> Option<Self> {
        let mut handle = 0u32;
        let mut key = [0u16; RM_SESSION_KEY_LEN + 1];
        let rc = unsafe { RmStartSession(&mut handle, 0, key.as_mut_ptr()) };
        (rc == 0).then_some(RmSession(handle))
    }

    fn register_files(&self, paths: &[&Path]) -> bool {
        let wides: Vec<Vec<u16>> = paths
            .iter()
            .take(RM_MAX_FILES)
            .map(|p| wide(&p.to_string_lossy()))
            .collect();
        let ptrs: Vec<*const u16> = wides.iter().map(|w| w.as_ptr()).collect();
        let rc = unsafe {
            RmRegisterResources(
                self.0,
                ptrs.len() as u32,
                ptrs.as_ptr(),
                0,
                std::ptr::null(),
                0,
                std::ptr::null(),
            )
        };
        rc == 0
    }

    fn list(&self) -> Vec<RM_PROCESS_INFO> {
        let mut infos: Vec<RM_PROCESS_INFO> = Vec::new();
        // The list can grow between calls; retry a few times.
        for _ in 0..4 {
            let mut needed = 0u32;
            let mut count = infos.capacity() as u32;
            let mut reasons = 0u32;
            let rc = unsafe {
                RmGetList(
                    self.0,
                    &mut needed,
                    &mut count,
                    infos.as_mut_ptr(),
                    &mut reasons,
                )
            };
            match rc {
                0 => {
                    unsafe { infos.set_len(count as usize) };
                    return infos;
                }
                ERROR_MORE_DATA => {
                    infos = Vec::with_capacity(needed as usize + 4);
                }
                _ => return Vec::new(),
            }
        }
        Vec::new()
    }
}

impl Drop for RmSession {
    fn drop(&mut self) {
        unsafe { RmEndSession(self.0) };
    }
}

fn from_wide_buf(buf: &[u16]) -> String {
    let end = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    String::from_utf16_lossy(&buf[..end])
}

fn holder_from(info: &RM_PROCESS_INFO) -> crate::locks::Holder {
    use crate::locks::{Holder, HolderKind};
    let name = from_wide_buf(&info.strAppName);
    let service = from_wide_buf(&info.strServiceShortName);
    let kind = match info.ApplicationType {
        RM_EXPLORER => HolderKind::Explorer,
        RM_SERVICE => HolderKind::Service,
        RM_CRITICAL => HolderKind::System,
        _ => HolderKind::Program,
    };
    Holder {
        pid: info.Process.dwProcessId,
        name,
        service: (!service.is_empty()).then_some(service),
        kind,
    }
}

pub fn file_holders(paths: &[&Path]) -> Vec<crate::locks::Holder> {
    let Some(s) = RmSession::start() else {
        return Vec::new();
    };
    if !s.register_files(paths) {
        return Vec::new();
    }
    s.list().iter().map(holder_from).collect()
}

/// Windows Explorer shut down through the Restart Manager; restarted on drop.
pub struct ExplorerPause {
    session: RmSession,
}

impl Drop for ExplorerPause {
    fn drop(&mut self) {
        let rc = unsafe { RmRestart(self.session.0, 0, None) };
        if rc != 0 {
            // Restart Manager couldn't bring it back; start it ourselves so
            // the user isn't left without a taskbar.
            let _ = std::process::Command::new(expand_windir("explorer.exe")).spawn();
        }
    }
}

fn expand_windir(file: &str) -> String {
    match std::env::var("SystemRoot") {
        Ok(root) => format!("{root}\\{file}"),
        Err(_) => file.to_string(),
    }
}

pub fn pause_explorer(paths: &[&Path]) -> Option<ExplorerPause> {
    let query = RmSession::start()?;
    if !query.register_files(paths) {
        return None;
    }
    let explorers: Vec<RM_UNIQUE_PROCESS> = query
        .list()
        .iter()
        .filter(|i| i.ApplicationType == RM_EXPLORER)
        .map(|i| i.Process)
        .collect();
    drop(query);
    if explorers.is_empty() {
        return None;
    }
    // A second session that knows only Explorer, so nothing else that has
    // the files open is ever shut down.
    let session = RmSession::start()?;
    let rc = unsafe {
        RmRegisterResources(
            session.0,
            0,
            std::ptr::null(),
            explorers.len() as u32,
            explorers.as_ptr(),
            0,
            std::ptr::null(),
        )
    };
    if rc != 0 {
        return None;
    }
    let rc = unsafe { RmShutdown(session.0, RM_FORCE_SHUTDOWN, None) };
    if rc != 0 {
        // Partly shut down is possible: restart whatever did close.
        unsafe { RmRestart(session.0, 0, None) };
        return None;
    }
    Some(ExplorerPause { session })
}

/// End a process at once.
fn end_process(pid: u32) {
    use windows_sys::Win32::System::Threading::{OpenProcess, TerminateProcess, PROCESS_TERMINATE};
    if let Some(h) = OwnedHandle::new(unsafe { OpenProcess(PROCESS_TERMINATE, 0, pid) }) {
        unsafe { TerminateProcess(h.get(), 1) };
    }
}

/// The full path of a process's program file.
fn process_image(pid: u32) -> Option<String> {
    use windows_sys::Win32::System::Threading::{
        OpenProcess, QueryFullProcessImageNameW, PROCESS_QUERY_LIMITED_INFORMATION,
    };
    let h = OwnedHandle::new(unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) })?;
    let mut buf = vec![0u16; 32_768];
    let mut len = buf.len() as u32;
    let ok = unsafe { QueryFullProcessImageNameW(h.get(), 0, buf.as_mut_ptr(), &mut len) };
    (ok != 0).then(|| String::from_utf16_lossy(&buf[..len as usize]))
}

/// One spelling for a path whatever form it was given in (short 8.3 names,
/// `..`, case), for comparing two paths to the same file.
fn same_file_key(p: &Path) -> String {
    fold(
        &std::fs::canonicalize(p)
            .unwrap_or_else(|_| p.to_path_buf())
            .to_string_lossy(),
    )
}

pub fn close_programs(exes: &[&Path]) -> crate::locks::Closed {
    use std::collections::HashSet;
    use std::time::{Duration, Instant};
    let mut out = crate::locks::Closed::default();
    let Some(query) = RmSession::start() else {
        return out;
    };
    if !query.register_files(exes) {
        return out;
    }
    let wanted: HashSet<String> = exes.iter().map(|p| same_file_key(p)).collect();
    // Programs with a window are asked to close, so they can save. Console
    // and windowless ones are ended directly: the Restart Manager would
    // signal their whole console, which can include whoever started them
    // (SetupScope's own command line, a terminal).
    let mut procs: Vec<RM_UNIQUE_PROCESS> = Vec::new();
    let mut end_directly: Vec<u32> = Vec::new();
    let mut names: Vec<(u32, String)> = Vec::new();
    for i in query.list() {
        if matches!(i.ApplicationType, RM_SERVICE | RM_EXPLORER | RM_CRITICAL) {
            continue;
        }
        let pid = i.Process.dwProcessId;
        // Only the program itself: its own .exe is being removed.
        let Some(image) = process_image(pid) else {
            continue;
        };
        if !wanted.contains(&same_file_key(Path::new(&image))) {
            continue;
        }
        if names.iter().any(|(p, _)| *p == pid) {
            continue;
        }
        let mut name = from_wide_buf(&i.strAppName);
        if name.is_empty() {
            name = crate::platform::leaf_name(&image).to_string();
        }
        if matches!(i.ApplicationType, RM_MAIN_WINDOW | RM_OTHER_WINDOW) {
            procs.push(i.Process);
        } else {
            end_directly.push(pid);
        }
        names.push((pid, format!("{name} (process {pid})")));
    }
    drop(query);
    if names.is_empty() {
        return out;
    }
    for pid in &end_directly {
        end_process(*pid);
    }
    let shut = procs.is_empty()
        || RmSession::start().is_some_and(|s| {
            let rc = unsafe {
                RmRegisterResources(
                    s.0,
                    0,
                    std::ptr::null(),
                    procs.len() as u32,
                    procs.as_ptr(),
                    0,
                    std::ptr::null(),
                )
            };
            rc == 0 && unsafe { RmShutdown(s.0, RM_FORCE_SHUTDOWN, None) } == 0
        });
    let deadline = Instant::now() + Duration::from_secs(if shut { 10 } else { 0 });
    for (pid, name) in names {
        while process_alive(pid) && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(100));
        }
        if process_alive(pid) {
            out.still_running.push(name);
        } else {
            out.closed.push(name);
        }
    }
    // A process is reported gone a moment before Windows unmaps its
    // program file; wait (briefly) until the files can really be changed.
    if !out.closed.is_empty() {
        let until = Instant::now() + Duration::from_secs(5);
        for exe in exes {
            while Instant::now() < until && image_in_use(exe) {
                std::thread::sleep(Duration::from_millis(50));
            }
        }
    }
    out
}

/// Whether a program file is still mapped by a process (it then refuses
/// to be opened for writing).
fn image_in_use(path: &Path) -> bool {
    const ERROR_SHARING_VIOLATION: i32 = 32;
    match std::fs::OpenOptions::new().write(true).open(path) {
        Ok(_) => false,
        Err(e) => e.raw_os_error() == Some(ERROR_SHARING_VIOLATION),
    }
}

// ---------------------------------------------------------------------------
// Services: stop and wait
// ---------------------------------------------------------------------------

use windows_sys::Win32::System::Services::{
    CloseServiceHandle, ControlService, OpenSCManagerW, OpenServiceW, QueryServiceStatusEx,
    SC_HANDLE, SERVICE_STATUS, SERVICE_STATUS_PROCESS,
};

const SC_MANAGER_CONNECT: u32 = 0x0001;
const SERVICE_QUERY_STATUS: u32 = 0x0004;
const SERVICE_STOP_ACCESS: u32 = 0x0020;
const SERVICE_CONTROL_STOP: u32 = 1;
const SC_STATUS_PROCESS_INFO: i32 = 0;
const STATE_STOPPED: u32 = 1;
const WIN32_OWN_PROCESS: u32 = 0x10;
const ERROR_SERVICE_DOES_NOT_EXIST: u32 = 1060;

struct ScHandle(SC_HANDLE);

impl ScHandle {
    fn new(h: SC_HANDLE) -> Option<Self> {
        (!h.is_null()).then_some(ScHandle(h))
    }
}

impl Drop for ScHandle {
    fn drop(&mut self) {
        unsafe { CloseServiceHandle(self.0) };
    }
}

fn service_status(svc: &ScHandle) -> Result<SERVICE_STATUS_PROCESS> {
    let mut st: SERVICE_STATUS_PROCESS = unsafe { std::mem::zeroed() };
    let mut needed = 0u32;
    let ok = unsafe {
        QueryServiceStatusEx(
            svc.0,
            SC_STATUS_PROCESS_INFO as _,
            &mut st as *mut _ as *mut u8,
            std::mem::size_of::<SERVICE_STATUS_PROCESS>() as u32,
            &mut needed,
        )
    };
    if ok == 0 {
        return Err(anyhow!(
            "query service status: {}",
            std::io::Error::last_os_error()
        ));
    }
    Ok(st)
}

pub fn stop_service(name: &str, timeout: std::time::Duration) -> Result<crate::locks::ServiceStop> {
    use crate::locks::ServiceStop;
    use std::time::{Duration, Instant};
    let scm = ScHandle::new(unsafe {
        OpenSCManagerW(std::ptr::null(), std::ptr::null(), SC_MANAGER_CONNECT)
    })
    .ok_or_else(|| anyhow!("open service manager: {}", std::io::Error::last_os_error()))?;
    let wname = wide(name);
    let Some(svc) = ScHandle::new(unsafe {
        OpenServiceW(
            scm.0,
            wname.as_ptr(),
            SERVICE_STOP_ACCESS | SERVICE_QUERY_STATUS,
        )
    }) else {
        let err = unsafe { GetLastError() };
        // A service exists exactly when its key does; don't depend on which
        // error code this Windows build reports for a missing one.
        let exists = open(
            &format!("HKLM\\SYSTEM\\CurrentControlSet\\Services\\{name}"),
            KEY_READ,
        )
        .is_ok();
        if err == ERROR_SERVICE_DOES_NOT_EXIST || !exists {
            return Ok(ServiceStop::NotFound);
        }
        return Err(anyhow!(
            "open service {name}: {}",
            std::io::Error::from_raw_os_error(err as i32)
        ));
    };
    let first = service_status(&svc)?;
    if first.dwCurrentState == STATE_STOPPED {
        return Ok(ServiceStop::WasNotRunning);
    }
    let pid = first.dwProcessId;
    let own_process = first.dwServiceType & WIN32_OWN_PROCESS != 0;
    let deadline = Instant::now() + timeout;
    let mut asked = false;
    loop {
        if !asked {
            let mut st: SERVICE_STATUS = unsafe { std::mem::zeroed() };
            // Refused while starting or already stopping: keep polling and
            // ask again, the service may accept it shortly.
            asked = unsafe { ControlService(svc.0, SERVICE_CONTROL_STOP, &mut st) } != 0;
        }
        let st = service_status(&svc)?;
        if st.dwCurrentState == STATE_STOPPED {
            break;
        }
        if Instant::now() >= deadline {
            anyhow::bail!("{name} did not stop within {} s", timeout.as_secs());
        }
        std::thread::sleep(Duration::from_millis(250));
    }
    // Stopped is reported before the process has finished exiting; its
    // open files are released only when it has.
    if own_process && pid != 0 {
        while process_alive(pid) {
            if Instant::now() >= deadline {
                anyhow::bail!(
                    "{name} stopped, but its process {pid} was still running after {} s",
                    timeout.as_secs()
                );
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    }
    Ok(ServiceStop::Stopped)
}

// ---------------------------------------------------------------------------
// Registry change notification for several keys at once
// ---------------------------------------------------------------------------

/// What [`RegistryWatch::wait`] saw.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WatchEvent {
    /// Something under one of the keys changed.
    Changed,
    /// The timeout passed with no change.
    Timeout,
}

struct OwnedKey(windows_sys::Win32::System::Registry::HKEY);

impl Drop for OwnedKey {
    fn drop(&mut self) {
        unsafe { windows_sys::Win32::System::Registry::RegCloseKey(self.0) };
    }
}

/// Keys watched by the kernel: waiting costs nothing until one changes.
pub struct RegistryWatch {
    keys: Vec<OwnedKey>,
    events: Vec<OwnedHandle>,
}

impl RegistryWatch {
    /// Watch these keys and everything beneath them (64-bit view; reach
    /// the 32-bit view through `WOW6432Node`). Keys that don't exist are
    /// skipped; fails only if none could be watched.
    pub fn new(paths: &[&str]) -> Result<Self> {
        use windows_sys::Win32::System::Registry::{
            RegOpenKeyExW, HKEY, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, HKEY_USERS, KEY_NOTIFY,
            KEY_WOW64_64KEY,
        };
        use windows_sys::Win32::System::Threading::CreateEventW;
        let mut w = RegistryWatch {
            keys: Vec::with_capacity(paths.len()),
            events: Vec::with_capacity(paths.len()),
        };
        for path in paths {
            let Some((hive, rest)) = super::split_hive(path) else {
                continue;
            };
            let root: HKEY = match hive {
                "HKLM" => HKEY_LOCAL_MACHINE,
                "HKCU" => HKEY_CURRENT_USER,
                "HKU" => HKEY_USERS,
                _ => continue,
            };
            let sub = wide(rest);
            let mut key: HKEY = std::ptr::null_mut();
            let rc = unsafe {
                RegOpenKeyExW(
                    root,
                    sub.as_ptr(),
                    0,
                    KEY_NOTIFY | KEY_WOW64_64KEY,
                    &mut key,
                )
            };
            if rc != 0 {
                continue;
            }
            let key = OwnedKey(key);
            let Some(event) =
                OwnedHandle::new(unsafe { CreateEventW(std::ptr::null(), 0, 0, std::ptr::null()) })
            else {
                continue;
            };
            if Self::arm(&key, &event) {
                w.keys.push(key);
                w.events.push(event);
            }
        }
        if w.keys.is_empty() {
            return Err(anyhow!("none of the registry keys could be watched"));
        }
        Ok(w)
    }

    fn arm(key: &OwnedKey, event: &OwnedHandle) -> bool {
        use windows_sys::Win32::System::Registry::{
            RegNotifyChangeKeyValue, REG_NOTIFY_CHANGE_LAST_SET, REG_NOTIFY_CHANGE_NAME,
        };
        unsafe {
            RegNotifyChangeKeyValue(
                key.0,
                1,
                REG_NOTIFY_CHANGE_NAME | REG_NOTIFY_CHANGE_LAST_SET,
                event.get(),
                1,
            ) == 0
        }
    }

    /// Block until a watched key changes or `timeout` passes (`None` waits
    /// for ever). The key that fired is watched again before returning.
    pub fn wait(&self, timeout: Option<std::time::Duration>) -> Result<WatchEvent> {
        use windows_sys::Win32::System::Threading::{WaitForMultipleObjects, INFINITE};
        const WAIT_TIMEOUT: u32 = 258;
        const MAX: usize = 64; // MAXIMUM_WAIT_OBJECTS
        let mut handles: [HANDLE; MAX] = [std::ptr::null_mut(); MAX];
        let n = self.events.len().min(MAX);
        for (slot, e) in handles.iter_mut().zip(&self.events) {
            *slot = e.get();
        }
        let ms = timeout
            .map(|t| t.as_millis().min(u128::from(INFINITE - 1)) as u32)
            .unwrap_or(INFINITE);
        let rc = unsafe { WaitForMultipleObjects(n as u32, handles.as_ptr(), 0, ms) };
        match rc {
            WAIT_TIMEOUT => Ok(WatchEvent::Timeout),
            WAIT_FAILED => Err(anyhow!(
                "waiting for registry changes failed: {}",
                std::io::Error::last_os_error()
            )),
            i if (i as usize) < n => {
                let i = i as usize;
                if !Self::arm(&self.keys[i], &self.events[i]) {
                    return Err(anyhow!("registry notifications stopped"));
                }
                Ok(WatchEvent::Changed)
            }
            other => Err(anyhow!("unexpected wait result {other}")),
        }
    }
}

// ---------------------------------------------------------------------------
// Snapshot walks: files and registry, reusing buffers
// ---------------------------------------------------------------------------

/// What a walk does with the entry it just reported.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Walk {
    /// Record it, and enter it if it is a folder.
    Continue,
    /// Leave it (and everything inside) out.
    Skip,
    /// End the walk now.
    Stop,
}

struct FindHandle(HANDLE);

impl Drop for FindHandle {
    fn drop(&mut self) {
        unsafe { windows_sys::Win32::Storage::FileSystem::FindClose(self.0) };
    }
}

/// Seconds since 1970 from a FILETIME (100 ns ticks since 1601).
fn filetime_secs(ft: &windows_sys::Win32::Foundation::FILETIME) -> i64 {
    let t = ((ft.dwHighDateTime as u64) << 32) | ft.dwLowDateTime as u64;
    (t as i64 - 116_444_736_000_000_000) / 10_000_000
}

/// Append UTF-16 `w` (up to its NUL) to `s` without allocating beyond growth.
fn push_utf16(s: &mut String, w: &[u16]) {
    let end = w.iter().position(|&c| c == 0).unwrap_or(w.len());
    s.extend(char::decode_utf16(w[..end].iter().copied()).map(|r| r.unwrap_or('\u{FFFD}')));
}

/// Walk every file and folder under `root` (not the root itself), without
/// following links or junctions, through one `\\?\` path buffer and one
/// display buffer that grow once and are reused for every entry. Each entry
/// is reported as (path, is_dir, size, mtime); `unreadable` gets folders
/// that couldn't be listed.
pub fn walk_files(
    root: &str,
    recursive: bool,
    visit: &mut dyn FnMut(&str, bool, u64, i64) -> Walk,
    unreadable: &mut dyn FnMut(&str),
) {
    use windows_sys::Win32::Storage::FileSystem::{
        FindExInfoBasic, FindExSearchNameMatch, FindFirstFileExW, FindNextFileW,
        FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_REPARSE_POINT, FIND_FIRST_EX_LARGE_FETCH,
        WIN32_FIND_DATAW,
    };
    let root = root.trim_end_matches('\\');
    // Long-path form for the API; the plain form for reporting.
    let mut wide_path: Vec<u16> = Vec::with_capacity(1024);
    if let Some(unc) = root.strip_prefix("\\\\").filter(|r| !r.starts_with("?\\")) {
        wide_path.extend("\\\\?\\UNC\\".encode_utf16());
        wide_path.extend(unc.encode_utf16());
    } else if root.starts_with("\\\\?\\") {
        wide_path.extend(root.encode_utf16());
    } else {
        wide_path.extend("\\\\?\\".encode_utf16());
        wide_path.extend(root.encode_utf16());
    }
    let mut display = String::with_capacity(1024);
    display.push_str(root);

    struct Level {
        handle: FindHandle,
        wide_len: usize,
        display_len: usize,
    }
    let open = |wide_path: &mut Vec<u16>, data: &mut WIN32_FIND_DATAW| -> Option<FindHandle> {
        let base = wide_path.len();
        wide_path.extend("\\*\0".encode_utf16());
        let h = unsafe {
            FindFirstFileExW(
                wide_path.as_ptr(),
                FindExInfoBasic,
                data as *mut _ as *mut core::ffi::c_void,
                FindExSearchNameMatch,
                std::ptr::null(),
                FIND_FIRST_EX_LARGE_FETCH,
            )
        };
        wide_path.truncate(base);
        (h != INVALID_HANDLE).then_some(FindHandle(h))
    };

    let mut data: WIN32_FIND_DATAW = unsafe { std::mem::zeroed() };
    let mut stack: Vec<Level> = Vec::with_capacity(64);
    match open(&mut wide_path, &mut data) {
        Some(h) => stack.push(Level {
            handle: h,
            wide_len: wide_path.len(),
            display_len: display.len(),
        }),
        None => {
            unreadable(root);
            return;
        }
    }
    // `data` holds the current entry of the top level.
    loop {
        let top_wide = stack.last().unwrap().wide_len;
        let top_disp = stack.last().unwrap().display_len;
        let name = &data.cFileName;
        let is_dot =
            name[0] == b'.' as u16 && (name[1] == 0 || (name[1] == b'.' as u16 && name[2] == 0));
        let attrs = data.dwFileAttributes;
        let mut entered = false;
        if !is_dot && attrs & FILE_ATTRIBUTE_REPARSE_POINT == 0 {
            wide_path.truncate(top_wide);
            wide_path.push(b'\\' as u16);
            let end = name.iter().position(|&c| c == 0).unwrap_or(name.len());
            wide_path.extend_from_slice(&name[..end]);
            display.truncate(top_disp);
            display.push('\\');
            push_utf16(&mut display, name);
            let is_dir = attrs & FILE_ATTRIBUTE_DIRECTORY != 0;
            let size = if is_dir {
                0
            } else {
                ((data.nFileSizeHigh as u64) << 32) | data.nFileSizeLow as u64
            };
            match visit(&display, is_dir, size, filetime_secs(&data.ftLastWriteTime)) {
                Walk::Stop => return,
                Walk::Skip => {}
                Walk::Continue if is_dir && recursive => match open(&mut wide_path, &mut data) {
                    Some(h) => {
                        stack.push(Level {
                            handle: h,
                            wide_len: wide_path.len(),
                            display_len: display.len(),
                        });
                        entered = true;
                    }
                    None => unreadable(&display),
                },
                Walk::Continue => {}
            }
        }
        if entered {
            continue; // `data` now holds the new folder's first entry
        }
        // Next entry, climbing out of finished folders.
        loop {
            let Some(top) = stack.last() else { return };
            if unsafe { FindNextFileW(top.handle.0, &mut data) } != 0 {
                break;
            }
            stack.pop();
            if stack.is_empty() {
                return;
            }
        }
    }
}

impl WinRegistry {
    /// Zero-copy-per-entry registry walk for snapshots: one open key per
    /// level, name, value-name and data buffers reused throughout.
    fn capture_native(
        &self,
        root: &str,
        excl: &crate::util::Exclusions,
        snap: &mut crate::model::Snapshot,
        progress: &crate::snapshot::Progress,
    ) {
        use windows_sys::Win32::System::Registry::{
            RegEnumKeyExW, RegEnumValueW, RegOpenKeyExW, RegQueryInfoKeyW, HKEY, HKEY_CLASSES_ROOT,
            HKEY_CURRENT_CONFIG, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, HKEY_USERS, KEY_READ,
            KEY_WOW64_64KEY,
        };
        const ERROR_NO_MORE_ITEMS: u32 = 259;
        const ERROR_MORE_DATA_REG: u32 = 234;
        let Some((hive, rest)) = super::split_hive(root) else {
            return;
        };
        let base: HKEY = match hive {
            "HKLM" => HKEY_LOCAL_MACHINE,
            "HKCU" => HKEY_CURRENT_USER,
            "HKU" => HKEY_USERS,
            "HKCR" => HKEY_CLASSES_ROOT,
            _ => HKEY_CURRENT_CONFIG,
        };
        let mut path = String::with_capacity(512);
        path.push_str(hive);
        if !rest.is_empty() {
            path.push('\\');
            path.push_str(rest);
        }
        let mut folded = String::with_capacity(512);
        let mut chars: Vec<char> = Vec::with_capacity(512);
        let mut name_buf: Vec<u16> = vec![0; 256];
        let mut sub_buf: Vec<u16> = vec![0; 256];
        let mut data_buf: Vec<u8> = vec![0; 4096];
        let mut text = String::with_capacity(256);

        let open_key = |parent: HKEY, sub: &[u16]| -> Option<OwnedKey> {
            let mut k: HKEY = std::ptr::null_mut();
            let rc = unsafe {
                RegOpenKeyExW(parent, sub.as_ptr(), 0, KEY_READ | KEY_WOW64_64KEY, &mut k)
            };
            (rc == 0).then_some(OwnedKey(k))
        };
        let rest_w: Vec<u16> = rest.encode_utf16().chain(std::iter::once(0)).collect();

        struct Level {
            key: OwnedKey,
            path_len: usize,
            next_sub: u32,
        }
        let mut stack: Vec<Level> = Vec::with_capacity(32);

        // Record one opened key (its values), returning whether it was recorded.
        let record = |key: &OwnedKey,
                      path: &str,
                      snap: &mut crate::model::Snapshot,
                      name_buf: &mut Vec<u16>,
                      sub_buf: &mut Vec<u16>,
                      data_buf: &mut Vec<u8>,
                      text: &mut String| {
            if progress.cancelled() {
                return;
            }
            let (mut max_sub, mut max_name, mut max_data) = (0u32, 0u32, 0u32);
            unsafe {
                RegQueryInfoKeyW(
                    key.0,
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    std::ptr::null(),
                    std::ptr::null_mut(),
                    &mut max_sub,
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    &mut max_name,
                    &mut max_data,
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                )
            };
            if sub_buf.len() < max_sub as usize + 1 {
                sub_buf.resize(max_sub as usize + 1, 0);
            }
            if name_buf.len() < max_name as usize + 1 {
                name_buf.resize(max_name as usize + 1, 0);
            }
            // Bounded regression captures reject oversized values before allocating
            // their buffers. Existing unbounded capture behavior is preserved.
            if progress.bounded_capture() && max_data > 8 * 1024 * 1024 {
                progress.exceed_byte_limit();
                return;
            }
            if !progress.reserve_capture_bytes(path.len() as u64 + 16) {
                return;
            }
            if data_buf.len() < max_data as usize {
                data_buf.resize(max_data as usize, 0);
            }
            snap.begin_key(path);
            let mut i = 0u32;
            loop {
                if progress.cancelled() {
                    return;
                }
                let mut nlen = name_buf.len() as u32;
                let mut dlen = data_buf.len() as u32;
                let mut kind = 0u32;
                let rc = unsafe {
                    RegEnumValueW(
                        key.0,
                        i,
                        name_buf.as_mut_ptr(),
                        &mut nlen,
                        std::ptr::null(),
                        &mut kind,
                        data_buf.as_mut_ptr(),
                        &mut dlen,
                    )
                };
                if rc == ERROR_MORE_DATA_REG {
                    // Changed while we read it: grow and retry this index.
                    let n = (name_buf.len() * 2).max(nlen as usize + 1);
                    name_buf.resize(n, 0);
                    let d = (data_buf.len() * 2).max(dlen as usize);
                    if progress.bounded_capture() && d > 8 * 1024 * 1024 {
                        progress.exceed_byte_limit();
                        return;
                    }
                    data_buf.resize(d, 0);
                    continue;
                }
                if rc != 0 {
                    if rc != ERROR_NO_MORE_ITEMS {
                        snap.warnings.push(format!(
                            "Could not enumerate values in {path} (Windows error {rc})"
                        ));
                    }
                    break; // ERROR_NO_MORE_ITEMS or the key went away
                }
                text.clear();
                push_utf16(text, &name_buf[..nlen as usize]);
                if !progress.reserve_capture_bytes(text.len() as u64 + dlen as u64 + 20) {
                    return;
                }
                snap.push_value(text, kind, &data_buf[..dlen as usize]);
                if progress.bounded_capture() {
                    progress.tick();
                }
                i += 1;
            }
        };

        let Some(rootkey) = (if rest.is_empty() {
            None
        } else {
            open_key(base, &rest_w)
        })
        .or_else(|| {
            rest.is_empty().then(|| {
                // A whole hive: open it with an empty subkey.
                open_key(base, &[0]).unwrap_or(OwnedKey(std::ptr::null_mut()))
            })
        })
        .filter(|k| !k.0.is_null()) else {
            snap.warnings.push(format!("Could not read {path}"));
            return;
        };
        crate::util::fold_into(&path, &mut folded);
        if excl.matches_folded(&folded, &mut chars) {
            return;
        }
        record(
            &rootkey,
            &path,
            snap,
            &mut name_buf,
            &mut sub_buf,
            &mut data_buf,
            &mut text,
        );
        progress.tick();
        stack.push(Level {
            key: rootkey,
            path_len: path.len(),
            next_sub: 0,
        });

        while let Some(top) = stack.last_mut() {
            if progress.cancelled() {
                return;
            }
            let mut slen = sub_buf.len() as u32;
            let rc = unsafe {
                RegEnumKeyExW(
                    top.key.0,
                    top.next_sub,
                    sub_buf.as_mut_ptr(),
                    &mut slen,
                    std::ptr::null(),
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                )
            };
            if rc == ERROR_MORE_DATA_REG {
                let n = sub_buf.len() * 2;
                sub_buf.resize(n, 0);
                continue;
            }
            if rc != 0 {
                if rc != ERROR_NO_MORE_ITEMS {
                    snap.warnings.push(format!(
                        "Could not enumerate subkeys in {path} (Windows error {rc})"
                    ));
                }
                stack.pop();
                continue;
            }
            top.next_sub += 1;
            let parent_len = top.path_len;
            let parent = top.key.0;
            path.truncate(parent_len);
            path.push('\\');
            push_utf16(&mut path, &sub_buf[..slen as usize]);
            if progress.bounded_capture() {
                progress.tick();
                if progress.cancelled() || !progress.reserve_capture_bytes(path.len() as u64 + 64) {
                    return;
                }
            }
            crate::util::fold_into(&path, &mut folded);
            if excl.matches_folded(&folded, &mut chars) {
                continue;
            }
            sub_buf[slen as usize] = 0;
            match open_key(parent, &sub_buf[..=slen as usize]) {
                Some(child) => {
                    record(
                        &child,
                        &path,
                        snap,
                        &mut name_buf,
                        &mut sub_buf,
                        &mut data_buf,
                        &mut text,
                    );
                    progress.tick();
                    stack.push(Level {
                        key: child,
                        path_len: path.len(),
                        next_sub: 0,
                    });
                }
                None => snap.warnings.push(format!("Could not read {path}")),
            }
        }
    }
}

/// Local date and time for a Unix timestamp, with the daylight-saving
/// rules in force on that date.
pub fn local_time_of(ts: i64) -> Option<crate::time::DateTime> {
    use windows_sys::Win32::Foundation::{FILETIME, SYSTEMTIME};
    use windows_sys::Win32::System::Time::{FileTimeToSystemTime, SystemTimeToTzSpecificLocalTime};
    let ticks = (ts.checked_add(11_644_473_600)?).checked_mul(10_000_000)?;
    if ticks < 0 {
        return None;
    }
    let ft = FILETIME {
        dwLowDateTime: ticks as u32,
        dwHighDateTime: (ticks >> 32) as u32,
    };
    let mut utc: SYSTEMTIME = unsafe { std::mem::zeroed() };
    let mut local: SYSTEMTIME = unsafe { std::mem::zeroed() };
    unsafe {
        if FileTimeToSystemTime(&ft, &mut utc) == 0
            || SystemTimeToTzSpecificLocalTime(std::ptr::null(), &utc, &mut local) == 0
        {
            return None;
        }
    }
    Some(crate::time::DateTime {
        date: crate::time::Date::from_ymd(
            local.wYear as i32,
            local.wMonth as u32,
            local.wDay as u32,
        )?,
        hour: local.wHour as u32,
        minute: local.wMinute as u32,
        second: local.wSecond as u32,
    })
}
