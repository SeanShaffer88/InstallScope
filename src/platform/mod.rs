//! Operating-system access, kept behind small traits so the core logic can be
//! unit-tested anywhere with an in-memory registry.

use crate::model::{fold, KeyData, RegValue};
use anyhow::{anyhow, Result};
use std::collections::BTreeMap;
use std::sync::Mutex;

#[cfg(windows)]
pub mod windows;

#[cfg(windows)]
pub use windows::{RegistryWatch, WatchEvent};

/// Tells watcher threads to finish. On Windows it is also a kernel event,
/// so a thread blocked waiting for changes wakes the moment it is set
/// instead of checking a flag on a timer.
pub struct StopSignal {
    flag: std::sync::atomic::AtomicBool,
    #[cfg(windows)]
    pub(crate) event: windows::ManualEvent,
}

impl Default for StopSignal {
    fn default() -> Self {
        Self::new()
    }
}

impl StopSignal {
    pub fn new() -> Self {
        StopSignal {
            flag: std::sync::atomic::AtomicBool::new(false),
            #[cfg(windows)]
            event: windows::ManualEvent::new(),
        }
    }
    pub fn stop(&self) {
        self.flag.store(true, std::sync::atomic::Ordering::SeqCst);
        #[cfg(windows)]
        self.event.set();
    }
    pub fn is_stopped(&self) -> bool {
        self.flag.load(std::sync::atomic::Ordering::SeqCst)
    }
}

/// Off Windows there is no registry to watch: every wait just times out.
#[cfg(not(windows))]
mod watch_stub {
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub enum WatchEvent {
        Changed,
        Timeout,
    }
    pub struct RegistryWatch;
    impl RegistryWatch {
        pub fn new(_paths: &[&str]) -> anyhow::Result<Self> {
            Ok(RegistryWatch)
        }
        pub fn wait(&self, timeout: Option<std::time::Duration>) -> anyhow::Result<WatchEvent> {
            std::thread::sleep(timeout.unwrap_or(std::time::Duration::from_secs(30)));
            Ok(WatchEvent::Timeout)
        }
    }
}
#[cfg(not(windows))]
pub use watch_stub::{RegistryWatch, WatchEvent};

/// Minimal registry API used by snapshots, uninstall and repair code.
/// Paths look like `HKLM\SOFTWARE\Vendor` and are case-insensitive.
/// Implementations always use the 64-bit view; the 32-bit view is reached
/// explicitly through `WOW6432Node` paths.
pub trait Registry: Send + Sync {
    fn exists(&self, path: &str) -> bool;
    /// The key's own values, or `None` if it can't be opened.
    fn read_key(&self, path: &str) -> Option<KeyData>;
    /// Names (not full paths) of the key's direct subkeys.
    fn subkeys(&self, path: &str) -> Vec<String>;
    fn create_key(&self, path: &str) -> Result<()>;
    fn set_value(&self, key: &str, value: &RegValue) -> Result<()>;
    fn delete_value(&self, key: &str, name: &str) -> Result<()>;
    /// Delete a key and everything beneath it.
    fn delete_tree(&self, path: &str) -> Result<()>;

    /// Record `root` and every key beneath it (except excluded branches)
    /// into a snapshot. The Windows registry overrides this with a walk
    /// that reuses its buffers; this version suits in-memory registries.
    fn capture(
        &self,
        root: &str,
        excl: &crate::util::Exclusions,
        snap: &mut crate::model::Snapshot,
        progress: &crate::snapshot::Progress,
    ) {
        let mut stack = vec![root.to_string()];
        while let Some(path) = stack.pop() {
            if progress.cancelled() {
                return;
            }
            if excl.matches(&path) {
                continue;
            }
            match self.read_key(&path) {
                Some(k) => {
                    snap.insert_key(k);
                    progress.tick();
                }
                None => {
                    snap.warnings.push(format!("Could not read {path}"));
                    continue;
                }
            }
            for sub in self.subkeys(&path) {
                stack.push(format!("{path}\\{sub}"));
            }
        }
    }
}

/// Canonical short hive names.
pub const HIVES: [(&str, &str); 5] = [
    ("HKLM", "HKEY_LOCAL_MACHINE"),
    ("HKCU", "HKEY_CURRENT_USER"),
    ("HKU", "HKEY_USERS"),
    ("HKCR", "HKEY_CLASSES_ROOT"),
    ("HKCC", "HKEY_CURRENT_CONFIG"),
];

/// Split `HKLM\SOFTWARE\X` into (`HKLM`, `SOFTWARE\X`). Long hive names are
/// normalised to their short form.
pub fn split_hive(path: &str) -> Option<(&'static str, &str)> {
    let (head, rest) = match path.split_once('\\') {
        Some((h, r)) => (h, r),
        None => (path, ""),
    };
    let h = head.to_ascii_uppercase();
    HIVES
        .iter()
        .find(|(short, long)| h == *short || h == *long)
        .map(|(short, _)| (*short, rest.trim_matches('\\')))
}

/// Normalise a registry path to short-hive form with single separators.
pub fn normalize_reg_path(path: &str) -> String {
    let p = path.trim().trim_matches('\\');
    match split_hive(p) {
        Some((h, "")) => h.to_string(),
        Some((h, rest)) => {
            let parts: Vec<&str> = rest.split('\\').filter(|s| !s.is_empty()).collect();
            format!("{h}\\{}", parts.join("\\"))
        }
        None => p.to_string(),
    }
}

pub fn parent_path(path: &str) -> Option<&str> {
    path.rsplit_once('\\').map(|(p, _)| p)
}

pub fn leaf_name(path: &str) -> &str {
    path.rsplit_once('\\').map(|(_, l)| l).unwrap_or(path)
}

/// In-memory registry used by tests and by non-Windows development builds.
#[derive(Default)]
pub struct MemRegistry {
    keys: Mutex<BTreeMap<String, KeyData>>,
}

impl MemRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Insert a key, creating any missing parents.
    pub fn put(&self, key: KeyData) {
        let mut keys = self.keys.lock().unwrap();
        let mut p = key.path.as_str();
        while let Some(parent) = parent_path(p) {
            keys.entry(fold(parent))
                .or_insert_with(|| KeyData::new(parent));
            p = parent;
        }
        keys.insert(fold(&key.path), key);
    }

    pub fn len(&self) -> usize {
        self.keys.lock().unwrap().len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl Registry for MemRegistry {
    fn exists(&self, path: &str) -> bool {
        split_hive(path).map(|(_, r)| r.is_empty()).unwrap_or(false)
            || self.keys.lock().unwrap().contains_key(&fold(path))
    }

    fn read_key(&self, path: &str) -> Option<KeyData> {
        let keys = self.keys.lock().unwrap();
        keys.get(&fold(path)).cloned().or_else(|| {
            // Hive roots always exist.
            match split_hive(path) {
                Some((h, "")) => Some(KeyData::new(h)),
                _ => None,
            }
        })
    }

    fn subkeys(&self, path: &str) -> Vec<String> {
        let prefix = format!("{}\\", fold(path));
        let keys = self.keys.lock().unwrap();
        keys.range(prefix.clone()..)
            .take_while(|(k, _)| k.starts_with(&prefix))
            .filter(|(k, _)| !k[prefix.len()..].contains('\\'))
            .map(|(_, v)| leaf_name(&v.path).to_string())
            .collect()
    }

    fn create_key(&self, path: &str) -> Result<()> {
        if !self.exists(path) {
            self.put(KeyData::new(path));
        }
        Ok(())
    }

    fn set_value(&self, key: &str, value: &RegValue) -> Result<()> {
        self.create_key(key)?;
        let mut keys = self.keys.lock().unwrap();
        let k = keys
            .get_mut(&fold(key))
            .ok_or_else(|| anyhow!("key missing"))?;
        k.values.insert(fold(&value.name), value.clone());
        Ok(())
    }

    fn delete_value(&self, key: &str, name: &str) -> Result<()> {
        let mut keys = self.keys.lock().unwrap();
        let k = keys
            .get_mut(&fold(key))
            .ok_or_else(|| anyhow!("key not found: {key}"))?;
        k.values
            .remove(&fold(name))
            .ok_or_else(|| anyhow!("value not found: {name}"))?;
        Ok(())
    }

    fn delete_tree(&self, path: &str) -> Result<()> {
        // Same last line of defence as the real registry.
        if crate::util::is_protected_key(path) {
            return Err(anyhow!("refusing to delete protected key {path}"));
        }
        let f = fold(path);
        let prefix = format!("{f}\\");
        let mut keys = self.keys.lock().unwrap();
        if !keys.contains_key(&f) {
            return Err(anyhow!("key not found: {path}"));
        }
        keys.retain(|k, _| k != &f && !k.starts_with(&prefix));
        Ok(())
    }
}

/// Read a key and all of its descendants (depth-first, parents first).
pub fn read_tree(reg: &dyn Registry, path: &str) -> Vec<KeyData> {
    let mut out = Vec::new();
    let mut stack = vec![path.to_string()];
    while let Some(p) = stack.pop() {
        if let Some(k) = reg.read_key(&p) {
            let mut subs = reg.subkeys(&p);
            subs.sort();
            for s in subs.into_iter().rev() {
                stack.push(format!("{p}\\{s}"));
            }
            out.push(k);
        }
    }
    out
}

/// Recreate keys and values previously captured with [`read_tree`].
pub fn write_tree(reg: &dyn Registry, keys: &[KeyData]) -> Result<()> {
    for k in keys {
        reg.create_key(&k.path)?;
        for v in k.values.values() {
            reg.set_value(&k.path, v)?;
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// File-system helpers
// ---------------------------------------------------------------------------

/// Delete a file, clearing the read-only attribute first if needed.
pub fn delete_file(path: &std::path::Path) -> std::io::Result<()> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied => {
            if let Ok(meta) = std::fs::metadata(path) {
                let mut perm = meta.permissions();
                #[allow(clippy::permissions_set_readonly_false)]
                perm.set_readonly(false);
                let _ = std::fs::set_permissions(path, perm);
            }
            std::fs::remove_file(path)
        }
        Err(e) => Err(e),
    }
}

/// A file's icon as RGBA pixels (width, height, data), if it has one.
pub fn icon_rgba(path: &str, index: i32, size: u32) -> Option<(u32, u32, Vec<u8>)> {
    #[cfg(windows)]
    {
        windows::icon_rgba(path, index, size)
    }
    #[cfg(not(windows))]
    {
        let _ = (path, index, size);
        None
    }
}

/// Move a file or folder to the Recycle Bin.
pub fn recycle(path: &std::path::Path) -> Result<()> {
    #[cfg(windows)]
    {
        windows::recycle(path)
    }
    #[cfg(not(windows))]
    {
        Err(anyhow!(
            "the Recycle Bin is only available on Windows: {}",
            path.display()
        ))
    }
}

/// Ask Windows to delete a locked file at the next reboot.
pub fn delete_on_reboot(path: &std::path::Path) -> Result<()> {
    #[cfg(windows)]
    {
        windows::delete_on_reboot(path)
    }
    #[cfg(not(windows))]
    {
        Err(anyhow!(
            "delete-on-reboot is only available on Windows: {}",
            path.display()
        ))
    }
}

/// Expand `%VAR%` references using the process environment. Unknown
/// variables are left untouched.
pub fn expand_env(s: &str) -> String {
    let mut out = String::new();
    let mut rest = s;
    while let Some(start) = rest.find('%') {
        out.push_str(&rest[..start]);
        let after = &rest[start + 1..];
        match after.find('%') {
            Some(end) => {
                let name = &after[..end];
                match std::env::var(name) {
                    Ok(v) if !name.is_empty() => out.push_str(&v),
                    _ => {
                        out.push('%');
                        out.push_str(name);
                        out.push('%');
                    }
                }
                rest = &after[end + 1..];
            }
            None => {
                out.push_str(&rest[start..]);
                rest = "";
            }
        }
    }
    out.push_str(rest);
    out
}

/// Run a console program without showing a window; returns exit code and
/// combined output.
pub fn run_hidden(program: &str, args: &[&str]) -> Result<(i32, String)> {
    let mut c = std::process::Command::new(program);
    c.args(args);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        c.creation_flags(CREATE_NO_WINDOW);
    }
    let out = c
        .output()
        .map_err(|e| anyhow!("could not run {program}: {e}"))?;
    let mut text = String::from_utf8_lossy(&out.stdout).to_string();
    text.push_str(&String::from_utf8_lossy(&out.stderr));
    Ok((out.status.code().unwrap_or(-1), text.trim().to_string()))
}

/// First Windows 11 build. SetupSweep is built for Windows 11 only.
pub const MIN_WINDOWS_BUILD: u32 = 22000;

/// Windows build number (for example 26100), or 0 if unknown.
pub fn windows_build(reg: &dyn Registry) -> u32 {
    reg.read_key("HKLM\\SOFTWARE\\Microsoft\\Windows NT\\CurrentVersion")
        .and_then(|k| k.get_string("CurrentBuild"))
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(0)
}

/// The build number when this PC runs a Windows older than Windows 11.
/// (Non-Windows builds exist only as a developer test harness.)
pub fn unsupported_windows() -> Option<u32> {
    #[cfg(windows)]
    {
        let build = windows_build(&*system_registry());
        if build > 0 && build < MIN_WINDOWS_BUILD {
            return Some(build);
        }
    }
    None
}

/// Where a shortcut (.lnk) points; other paths are returned unchanged.
pub fn resolve_shortcut(path: &str) -> String {
    if !path.to_lowercase().ends_with(".lnk") {
        return path.to_string();
    }
    #[cfg(windows)]
    {
        let script = format!(
            "(New-Object -ComObject WScript.Shell).CreateShortcut('{}').TargetPath",
            path.replace('\'', "''")
        );
        if let Ok((0, out)) = run_hidden(
            "powershell",
            &["-NoProfile", "-NonInteractive", "-Command", &script],
        ) {
            let out = out.trim();
            if !out.is_empty() {
                return out.to_string();
            }
        }
    }
    path.to_string()
}

/// Whether a process with this id is still running.
pub fn process_alive(pid: u32) -> bool {
    if pid == 0 {
        return false;
    }
    #[cfg(windows)]
    {
        windows::process_alive(pid)
    }
    #[cfg(not(windows))]
    {
        std::path::Path::new(&format!("/proc/{pid}")).exists()
    }
}

/// Whether Windows' animation effects are on (SPI_GETCLIENTAREAANIMATION).
pub fn animations_enabled() -> bool {
    #[cfg(windows)]
    {
        windows::client_area_animation()
    }
    #[cfg(not(windows))]
    {
        true
    }
}

/// Show a message and wait until the user clicks OK.
pub fn ask_ok(title: &str, text: &str) {
    #[cfg(windows)]
    {
        windows::info_box(title, text);
    }
    #[cfg(not(windows))]
    {
        eprintln!("{title}: {text}");
        let mut s = String::new();
        let _ = std::io::stdin().read_line(&mut s);
    }
}

pub fn computer_name() -> String {
    std::env::var("COMPUTERNAME")
        .or_else(|_| std::env::var("HOSTNAME"))
        .unwrap_or_else(|_| "unknown".into())
}

pub fn user_name() -> String {
    std::env::var("USERNAME")
        .or_else(|_| std::env::var("USER"))
        .unwrap_or_else(|_| "unknown".into())
}

/// The live system registry on Windows; an empty in-memory one elsewhere.
pub fn system_registry() -> Box<dyn Registry> {
    #[cfg(windows)]
    {
        Box::new(windows::WinRegistry)
    }
    #[cfg(not(windows))]
    {
        let r = MemRegistry::new();
        if std::env::var("SETUPSWEEP_DEMO").is_ok() {
            demo_programs(&r);
        }
        Box::new(r)
    }
}

/// Sample programs for UI development on non-Windows machines.
#[cfg(not(windows))]
fn demo_programs(r: &MemRegistry) {
    // Right-click menu entries, one of them left behind by a removed program.
    for (key, value) in [
        (
            "HKLM\\SOFTWARE\\Classes\\*\\shellex\\ContextMenuHandlers\\7-Zip",
            "{23170F69-40C1-278A-1000-000100020000}",
        ),
        (
            "HKLM\\SOFTWARE\\Classes\\Directory\\shellex\\ContextMenuHandlers\\WinRAR",
            "{B41DB860-64E4-11D2-9906-E49FADC173CA}",
        ),
        (
            "HKLM\\SOFTWARE\\Classes\\*\\shellex\\ContextMenuHandlers\\OldToolbarShell",
            "{0F1E2D3C-4B5A-6978-8796-A5B4C3D2E1F0}",
        ),
    ] {
        r.put(KeyData::new(key).with(RegValue::string("", value)));
    }
    for (clsid, name, dll) in [
        (
            "{23170F69-40C1-278A-1000-000100020000}",
            "7-Zip Shell Extension",
            "C:\\Program Files\\7-Zip\\7-zip.dll",
        ),
        (
            "{B41DB860-64E4-11D2-9906-E49FADC173CA}",
            "WinRAR shell extension",
            "C:\\Program Files\\WinRAR\\rarext.dll",
        ),
    ] {
        r.put(
            KeyData::new(&format!("HKLM\\SOFTWARE\\Classes\\CLSID\\{clsid}"))
                .with(RegValue::string("", name)),
        );
        r.put(
            KeyData::new(&format!(
                "HKLM\\SOFTWARE\\Classes\\CLSID\\{clsid}\\InprocServer32"
            ))
            .with(RegValue::string("", dll)),
        );
    }
    r.put(
        KeyData::new("HKCU\\Software\\Classes\\Directory\\Background\\shell\\VSCode")
            .with(RegValue::string("", "Open with Code")),
    );
    r.put(
        KeyData::new("HKCU\\Software\\Classes\\Directory\\Background\\shell\\VSCode\\command")
            .with(RegValue::string(
                "",
                "\"C:\\Users\\demo\\AppData\\Local\\Programs\\Microsoft VS Code\\Code.exe\" \"%V\"",
            )),
    );

    let root = "HKLM\\SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\Uninstall";
    let apps = [
        (
            "7-Zip 24.08 (x64)",
            "Igor Pavlov",
            "24.08",
            "20260611",
            5_800,
        ),
        (
            "Audacity 3.7.1",
            "Audacity Team",
            "3.7.1",
            "20260702",
            98_000,
        ),
        ("Discord", "Discord Inc.", "1.0.9172", "20260915", 0),
        (
            "Git",
            "The Git Development Community",
            "2.47.0",
            "20260820",
            340_000,
        ),
        ("GIMP 3.0.2", "The GIMP Team", "3.0.2", "20260530", 410_000),
        (
            "Mozilla Firefox (x64 en-US)",
            "Mozilla",
            "131.0.3",
            "20261008",
            255_000,
        ),
        (
            "Notepad++ (64-bit x64)",
            "Notepad++ Team",
            "8.7",
            "20260901",
            15_000,
        ),
        ("OBS Studio", "OBS Project", "31.0.0", "20260722", 420_000),
        (
            "Old Toolbar Helper",
            "Unknown Corp",
            "1.2",
            "20190101",
            1_200,
        ),
        (
            "Python 3.13.0 (64-bit)",
            "Python Software Foundation",
            "3.13.0",
            "20261001",
            110_000,
        ),
        ("Steam", "Valve Corporation", "2.10.91.91", "20260415", 0),
        (
            "VLC media player",
            "VideoLAN",
            "3.0.21",
            "20260610",
            165_000,
        ),
        (
            "Visual Studio Code",
            "Microsoft Corporation",
            "1.94.2",
            "20261010",
            380_000,
        ),
        (
            "McAfee LiveSafe",
            "McAfee, LLC",
            "16.0",
            "20260301",
            310_000,
        ),
        (
            "Dell SupportAssist",
            "Dell Inc.",
            "4.4.2",
            "20260301",
            190_000,
        ),
        (
            "WinRAR 7.01 (64-bit)",
            "win.rar GmbH",
            "7.01.0",
            "20250412",
            9_000,
        ),
    ];
    for (i, (name, publisher, ver, date, kb)) in apps.iter().enumerate() {
        // Uninstall entries shaped like the real installer technologies, so
        // the unattended queue can be tried out.
        let (leaf, uninstall, msi) = match *name {
            n if n.starts_with("Audacity") || n.starts_with("GIMP") => (
                format!("App{i}_is1"),
                format!("\"C:\\Program Files\\App{i}\\unins000.exe\""),
                false,
            ),
            "Discord" => (
                "Discord".to_string(),
                "\"C:\\Users\\demo\\AppData\\Local\\Discord\\Update.exe\" --uninstall".to_string(),
                false,
            ),
            n if n.starts_with("Python") => (
                "{4D1B5C22-9C6A-4E2B-9E61-6A0F5A1E0001}".to_string(),
                "\"C:\\ProgramData\\Package Cache\\{4D1B5C22-9C6A-4E2B-9E61-6A0F5A1E0001}\\python-3.13.0-amd64.exe\" /uninstall".to_string(),
                false,
            ),
            n if n.starts_with("7-Zip") => (
                "{23170F69-40C1-2702-2408-000001000000}".to_string(),
                "MsiExec.exe /I{23170F69-40C1-2702-2408-000001000000}".to_string(),
                true,
            ),
            _ => (
                format!("App{i}"),
                format!("C:\\Program Files\\App{i}\\uninstall.exe"),
                false,
            ),
        };
        let mut k = KeyData::new(&format!("{root}\\{leaf}"))
            .with(RegValue::string("DisplayName", name))
            .with(RegValue::string("Publisher", publisher))
            .with(RegValue::string("DisplayVersion", ver))
            .with(RegValue::string("InstallDate", date))
            .with(RegValue::string("UninstallString", &uninstall));
        if msi {
            k = k.with(RegValue::dword("WindowsInstaller", 1));
        }
        if name.starts_with("Old") {
            k = k.with(RegValue::string(
                "InstallLocation",
                "/nonexistent/OldToolbar",
            ));
        }
        if *kb > 0 {
            k = k.with(RegValue::dword("EstimatedSize", *kb));
        }
        r.put(k);
    }
    // A few extra autostarts for the Startup page's "Other autostarts".
    r.put(
        KeyData::new("HKLM\\SOFTWARE\\Microsoft\\Active Setup\\Installed Components\\{9D1D3F1E-0000-4C11-9C3B-DEMO00000001}")
            .with(RegValue::string("", "Old Toolbar Helper"))
            .with(RegValue::string("StubPath", "\"C:\\Program Files\\OldToolbar\\setup.exe\" /peruser")),
    );
    r.put(
        KeyData::new("HKLM\\SOFTWARE\\Microsoft\\Windows NT\\CurrentVersion\\Image File Execution Options\\notepad.exe")
            .with(RegValue::string("Debugger", "\"C:\\Program Files\\Notepad++\\notepad++.exe\" -notepadStyleCmdline -z")),
    );
    // Typical Run-key autostarts for the Startup page.
    r.put(
        KeyData::new("HKCU\\Software\\Microsoft\\Windows\\CurrentVersion\\Run")
            .with(RegValue::string("Discord", "\"C:\\Users\\demo\\AppData\\Local\\Discord\\Update.exe\" --processStart Discord.exe"))
            .with(RegValue::string("OneDrive", "\"C:\\Program Files\\Microsoft OneDrive\\OneDrive.exe\" /background"))
            .with(RegValue::string("Spotify", "\"C:\\Users\\demo\\AppData\\Roaming\\Spotify\\Spotify.exe\" --autostart --minimized")),
    );
    r.put(
        KeyData::new("HKLM\\SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\Run")
            .with(RegValue::string("SecurityHealth", "%windir%\\system32\\SecurityHealthSystray.exe"))
            .with(RegValue::string("RtkAudUService", "\"C:\\Windows\\System32\\DriverStore\\FileRepository\\realtekservice.inf_amd64\\RtkAudUService64.exe\" -background")),
    );
    // A couple of privacy settings already changed, so the Anti-Telemetry
    // page shows a mix of states.
    r.put(
        KeyData::new("HKCU\\Software\\Microsoft\\Windows\\CurrentVersion\\AdvertisingInfo")
            .with(RegValue::dword("Enabled", 0)),
    );
    r.put(
        KeyData::new("HKCU\\Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize")
            .with(RegValue::dword("EnableTransparency", 0)),
    );
}

#[cfg(windows)]
pub use windows::{spawn_command_line, ChildProcess};

/// A started process that can be polled for completion.
#[cfg(not(windows))]
pub struct ChildProcess(Mutex<std::process::Child>);

#[cfg(not(windows))]
impl ChildProcess {
    pub fn try_wait(&self) -> Option<u32> {
        self.0
            .lock()
            .unwrap()
            .try_wait()
            .ok()
            .flatten()
            .map(|s| s.code().unwrap_or(0) as u32)
    }
    pub fn wait(&self) -> u32 {
        self.0
            .lock()
            .unwrap()
            .wait()
            .map(|s| s.code().unwrap_or(0) as u32)
            .unwrap_or(0)
    }
}

/// Run a raw command line (development stand-in for the Windows version).
#[cfg(not(windows))]
pub fn spawn_command_line(cmdline: &str) -> Result<ChildProcess> {
    let child = std::process::Command::new("sh")
        .arg("-c")
        .arg(cmdline)
        .spawn()?;
    Ok(ChildProcess(Mutex::new(child)))
}

/// Open a file, folder or URL with its default handler.
pub fn shell_open(target: &str) {
    #[cfg(windows)]
    windows::shell_open(target);
    #[cfg(not(windows))]
    {
        let _ = std::process::Command::new("xdg-open").arg(target).spawn();
    }
}

/// Show a file selected in Explorer (or open its folder).
pub fn reveal(path: &str) {
    #[cfg(windows)]
    {
        if std::path::Path::new(path).is_file() {
            let _ =
                windows::shell_execute_wait("explorer.exe", &format!("/select,\"{path}\""), false);
            return;
        }
    }
    shell_open(path);
}

pub fn open_in_regedit(path: &str) -> Result<()> {
    #[cfg(windows)]
    {
        windows::open_in_regedit(path)
    }
    #[cfg(not(windows))]
    {
        Err(anyhow!(
            "Registry Editor is only available on Windows ({path})"
        ))
    }
}

/// Restart elevated; true if the new instance started.
pub fn relaunch_elevated(args: &[String]) -> bool {
    #[cfg(windows)]
    {
        windows::relaunch_elevated(args)
    }
    #[cfg(not(windows))]
    {
        let _ = args;
        false
    }
}

pub fn apply_window_style(border_rgb: (u8, u8, u8), dark: bool) {
    #[cfg(windows)]
    windows::apply_window_style(border_rgb, dark);
    #[cfg(not(windows))]
    let _ = (border_rgb, dark);
}

pub fn is_elevated() -> bool {
    #[cfg(windows)]
    {
        windows::is_elevated()
    }
    #[cfg(not(windows))]
    {
        // Demo builds can pretend to be elevated, for screenshots.
        std::env::var_os("SETUPSWEEP_DEMO_ADMIN").is_some()
    }
}

/// True while a Windows Installer operation is running (always false off Windows).
pub fn installer_busy() -> bool {
    #[cfg(windows)]
    {
        windows::installer_busy()
    }
    #[cfg(not(windows))]
    {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hive_parsing() {
        assert_eq!(
            split_hive("HKEY_LOCAL_MACHINE\\Software"),
            Some(("HKLM", "Software"))
        );
        assert_eq!(
            normalize_reg_path("hkcu\\\\Software\\X\\"),
            "HKCU\\Software\\X"
        );
        assert_eq!(split_hive("Nope\\x"), None);
    }

    #[test]
    fn mem_registry_tree() {
        let r = MemRegistry::new();
        r.put(KeyData::new("HKLM\\SOFTWARE\\A\\B").with(RegValue::dword("x", 1)));
        assert!(r.exists("hklm\\software\\a"));
        assert_eq!(r.subkeys("HKLM\\SOFTWARE"), vec!["A".to_string()]);
        let t = read_tree(&r, "HKLM\\SOFTWARE\\A");
        assert_eq!(t.len(), 2);
        r.delete_tree("HKLM\\SOFTWARE\\A").unwrap();
        assert!(!r.exists("HKLM\\SOFTWARE\\A\\B"));
        write_tree(&r, &t).unwrap();
        assert_eq!(
            r.read_key("HKLM\\SOFTWARE\\A\\B").unwrap().get_u32("x"),
            Some(1)
        );
    }

    #[test]
    fn env_expansion() {
        std::env::set_var("SETUPSWEEP_TEST_VAR", "C:\\X");
        assert_eq!(expand_env("%SETUPSWEEP_TEST_VAR%\\y"), "C:\\X\\y");
        assert_eq!(expand_env("%NOPE_NOT_SET%\\y"), "%NOPE_NOT_SET%\\y");
        assert_eq!(expand_env("100%"), "100%");
    }
}
