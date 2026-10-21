//! Small helpers: command-line splitting, glob matching, IDs, and the safety
//! rails that stop InstallScope from ever touching critical system locations.

use crate::model::fold;
use crate::platform::{expand_env, normalize_reg_path};

/// Current local time as a sortable, readable string.
pub fn now_string() -> String {
    crate::time::DateTime::now_local().ymd_hms()
}

/// A unique, file-name-safe identifier based on the current time.
pub fn new_id() -> String {
    use std::sync::atomic::{AtomicU32, Ordering};
    static COUNTER: AtomicU32 = AtomicU32::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    format!(
        "{}-{:03}",
        crate::time::DateTime::now_local().compact(),
        n % 1000
    )
}

/// Make a string safe to use as a file name.
pub fn safe_file_name(s: &str) -> String {
    let cleaned: String = s
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || " -_.()".contains(c) {
                c
            } else {
                '_'
            }
        })
        .collect();
    let t = cleaned.trim().trim_matches('.').to_string();
    if t.is_empty() {
        "unnamed".into()
    } else {
        t.chars().take(80).collect()
    }
}

/// Split a Windows command line into (program, arguments). Handles quoted
/// paths and the common unquoted `C:\Program Files\X\unins000.exe /x` form.
pub fn split_command_line(cmd: &str) -> (String, String) {
    let cmd = cmd.trim();
    if let Some(rest) = cmd.strip_prefix('"') {
        if let Some(end) = rest.find('"') {
            return (rest[..end].to_string(), rest[end + 1..].trim().to_string());
        }
        return (rest.to_string(), String::new());
    }
    let lower = cmd.to_lowercase();
    for ext in [".exe", ".com", ".bat", ".cmd", ".msi"] {
        let mut search_from = 0;
        while let Some(pos) = lower[search_from..].find(ext) {
            let end = search_from + pos + ext.len();
            if end == cmd.len() || cmd.as_bytes()[end] == b' ' {
                return (cmd[..end].to_string(), cmd[end..].trim().to_string());
            }
            search_from = end;
        }
    }
    match cmd.split_once(' ') {
        Some((p, a)) => (p.to_string(), a.trim().to_string()),
        None => (cmd.to_string(), String::new()),
    }
}

/// Case-insensitive glob with `*` (any run of characters, including `\`)
/// and `?` (any single character).
pub fn glob_match(pattern: &str, text: &str) -> bool {
    let p: Vec<char> = pattern.to_lowercase().chars().collect();
    let t: Vec<char> = text.to_lowercase().chars().collect();
    glob_chars(&p, &t)
}

/// [`glob_match`] on already lower-cased characters.
fn glob_chars(p: &[char], t: &[char]) -> bool {
    let (mut pi, mut ti) = (0usize, 0usize);
    let (mut star, mut mark) = (None::<usize>, 0usize);
    while ti < t.len() {
        if pi < p.len() && (p[pi] == '?' || p[pi] == t[ti]) {
            pi += 1;
            ti += 1;
        } else if pi < p.len() && p[pi] == '*' {
            star = Some(pi);
            mark = ti;
            pi += 1;
        } else if let Some(s) = star {
            pi = s + 1;
            mark += 1;
            ti = mark;
        } else {
            return false;
        }
    }
    while pi < p.len() && p[pi] == '*' {
        pi += 1;
    }
    pi == p.len()
}

enum Pat {
    /// No wildcards: the location itself and everything beneath it.
    Path(String),
    /// `*.ext`-style: any path ending with the suffix (`dir` is the
    /// suffix followed by `\`, for locations beneath a match).
    Suffix { s: String, dir: String },
    /// General glob. `prefix` (text before the first wildcard) must start
    /// the path, which rejects most paths without running the glob.
    Glob {
        prefix: String,
        exact: Vec<char>,
        below: Vec<char>,
    },
}

/// Exclusion patterns compiled once and matched many times (snapshots test
/// every registry key and file against them).
pub struct Exclusions {
    pats: Vec<Pat>,
}

impl Exclusions {
    pub fn new(patterns: &[String]) -> Self {
        let mut pats = Vec::new();
        for raw in patterns {
            let pat = raw.trim();
            if pat.is_empty() || pat.starts_with('#') {
                continue;
            }
            let pat = fold(&expand_env(pat));
            let pat = pat.trim_end_matches('\\').to_string();
            if pat.is_empty() {
                continue;
            }
            let wild = pat.find(['*', '?']);
            match wild {
                None => pats.push(Pat::Path(pat)),
                Some(0) if !pat[1..].contains(['*', '?', '\\']) => pats.push(Pat::Suffix {
                    s: pat[1..].to_string(),
                    dir: format!("{}\\", &pat[1..]),
                }),
                Some(i) => pats.push(Pat::Glob {
                    prefix: pat[..i].to_string(),
                    exact: pat.chars().collect(),
                    below: format!("{pat}\\*").chars().collect(),
                }),
            }
        }
        Exclusions { pats }
    }

    pub fn is_empty(&self) -> bool {
        self.pats.is_empty()
    }

    /// True if `path` matches a pattern or lies beneath a matching location.
    pub fn matches(&self, path: &str) -> bool {
        if self.pats.is_empty() {
            return false;
        }
        let mut chars = Vec::new();
        self.matches_folded(&fold(path), &mut chars)
    }

    /// [`Exclusions::matches`] for a path already folded with
    /// [`fold_into`], reusing `chars` as scratch space: allocates nothing
    /// once the buffers have grown, for use in snapshot walks.
    pub fn matches_folded(&self, f: &str, chars: &mut Vec<char>) -> bool {
        let mut filled = false;
        self.pats.iter().any(|p| match p {
            Pat::Path(p) => {
                f.starts_with(p.as_str()) && (f.len() == p.len() || f.as_bytes()[p.len()] == b'\\')
            }
            Pat::Suffix { s, dir } => f.ends_with(s.as_str()) || f.contains(dir.as_str()),
            Pat::Glob {
                prefix,
                exact,
                below,
            } => {
                if !f.starts_with(prefix.as_str()) {
                    return false;
                }
                if !filled {
                    chars.clear();
                    chars.extend(f.chars());
                    filled = true;
                }
                glob_chars(exact, chars) || glob_chars(below, chars)
            }
        })
    }
}

/// Case-fold `src` into `dst` (replacing its contents) exactly as
/// [`fold`] does, reusing `dst`'s allocation.
pub fn fold_into(src: &str, dst: &mut String) {
    dst.clear();
    if src.is_ascii() {
        dst.push_str(src);
        dst.make_ascii_lowercase();
    } else {
        dst.push_str(&src.to_lowercase());
    }
}

/// True if `path` is excluded by any pattern. A pattern also excludes
/// everything beneath the location it names. Compiles the patterns on every
/// call: use [`Exclusions`] in loops.
pub fn is_excluded(path: &str, patterns: &[String]) -> bool {
    Exclusions::new(patterns).matches(path)
}

/// A file path the way Windows resolves it, for comparisons: `/` becomes
/// `\`, the `\\?\` and `\??\` prefixes go, `.` and `..` are resolved,
/// trailing dots and spaces are dropped from each name (Windows ignores
/// them), repeated separators collapse, and case is folded. Purely lexical:
/// junctions and short (8.3) names are handled by [`guard_dir_delete`].
pub fn lexical_path(p: &str) -> String {
    let mut s = p.trim().replace('/', "\\");
    for prefix in ["\\\\?\\UNC\\", "\\??\\UNC\\"] {
        if s.len() >= prefix.len() && s[..prefix.len()].eq_ignore_ascii_case(prefix) {
            s = format!("\\\\{}", &s[prefix.len()..]);
        }
    }
    for prefix in ["\\\\?\\", "\\??\\", "\\\\.\\"] {
        if let Some(rest) = s.strip_prefix(prefix) {
            s = rest.to_string();
        }
    }
    let unc = s.starts_with("\\\\");
    let mut out: Vec<String> = Vec::new();
    for raw in s.split('\\') {
        match raw.trim_end_matches(' ') {
            "" | "." => {}
            ".." => {
                // Never climb above the drive or share root.
                if out.len() > if unc { 2 } else { 1 } {
                    out.pop();
                }
            }
            seg => {
                let seg = seg.trim_end_matches(['.', ' ']);
                if !seg.is_empty() {
                    out.push(seg.to_string());
                }
            }
        }
    }
    let body = out.join("\\");
    let joined = if unc {
        format!("\\\\{body}")
    } else if out.len() == 1 && body.ends_with(':') {
        format!("{body}\\")
    } else {
        body
    };
    fold(&joined)
}

fn norm_file(p: &str) -> String {
    lexical_path(p)
}

/// Folders InstallScope must never delete as a whole, whatever a log or heuristic
/// says. Files *inside* them may still be removed individually. An entry
/// ending in `\*` also protects every folder directly inside it.
pub fn protected_dirs() -> Vec<String> {
    protected_dirs_with(&expand_env)
}

/// [`protected_dirs`] with a chosen `%VARIABLE%` expander (for tests).
pub fn protected_dirs_with(expand: &dyn Fn(&str) -> String) -> Vec<String> {
    let mut v: Vec<String> = [
        // Everything directly inside Windows, System32 and SysWOW64
        // (WinSxS, Installer, config, DriverStore, spool…).
        "%SystemRoot%\\*",
        "%SystemRoot%\\System32\\*",
        "%SystemRoot%\\SysWOW64\\*",
        "%SystemRoot%\\System32\\DriverStore\\FileRepository",
        // Folders at the top of the system drive that belong to Windows.
        "%SystemDrive%\\$Recycle.Bin",
        "%SystemDrive%\\System Volume Information",
        "%SystemDrive%\\Recovery",
        "%SystemDrive%\\Boot",
        "%SystemDrive%\\EFI",
        "%SystemDrive%\\PerfLogs",
        "%SystemDrive%\\Windows.old",
        "%SystemDrive%\\Config.Msi",
        // The Users folder and every profile in it.
        "%USERPROFILE%\\..\\*",
        "%ProgramFiles%\\WindowsApps",
        "%ProgramFiles%\\Windows Defender",
        "%ProgramFiles%\\Windows NT",
        "%ProgramFiles%\\Common Files\\Microsoft Shared",
        "%ProgramData%\\Microsoft\\Windows Defender",
        "%ProgramData%\\Packages",
        "%SystemDrive%\\",
        "%SystemRoot%",
        "%SystemRoot%\\System32",
        "%SystemRoot%\\SysWOW64",
        "%SystemRoot%\\System32\\drivers",
        "%SystemRoot%\\System32\\Tasks",
        "%SystemRoot%\\Fonts",
        "%SystemRoot%\\Temp",
        "%ProgramFiles%",
        "%ProgramFiles(x86)%",
        "%ProgramW6432%",
        "%CommonProgramFiles%",
        "%CommonProgramFiles(x86)%",
        "%CommonProgramW6432%",
        "%ProgramData%",
        "%ProgramData%\\Microsoft",
        "%ProgramData%\\Microsoft\\Windows",
        "%ProgramData%\\Microsoft\\Windows\\Start Menu",
        "%ProgramData%\\Microsoft\\Windows\\Start Menu\\Programs",
        "%ProgramData%\\Microsoft\\Windows\\Start Menu\\Programs\\Startup",
        "%PUBLIC%",
        "%PUBLIC%\\Desktop",
        "%PUBLIC%\\Documents",
        "%USERPROFILE%",
        "%USERPROFILE%\\..",
        "%USERPROFILE%\\Desktop",
        "%USERPROFILE%\\Documents",
        "%USERPROFILE%\\Downloads",
        "%USERPROFILE%\\Pictures",
        "%USERPROFILE%\\Music",
        "%USERPROFILE%\\Videos",
        "%USERPROFILE%\\AppData",
        "%USERPROFILE%\\AppData\\LocalLow",
        "%APPDATA%",
        "%APPDATA%\\Microsoft",
        "%APPDATA%\\Microsoft\\Windows",
        "%APPDATA%\\Microsoft\\Windows\\Start Menu",
        "%APPDATA%\\Microsoft\\Windows\\Start Menu\\Programs",
        "%APPDATA%\\Microsoft\\Windows\\Start Menu\\Programs\\Startup",
        "%LOCALAPPDATA%",
        "%LOCALAPPDATA%\\Microsoft",
        "%LOCALAPPDATA%\\Programs",
        "%LOCALAPPDATA%\\Temp",
        "%TEMP%",
        "%OneDrive%",
    ]
    .iter()
    .map(|s| expand(s))
    .filter(|s| !s.contains('%'))
    .map(|s| norm_file(&s))
    .collect();
    v.sort();
    v.dedup();
    v
}

/// Whether deleting this folder (recursively) would be unsafe.
pub fn is_protected_dir(path: &str, protected: &[String]) -> bool {
    let p = norm_file(path);
    // Drive roots and anything with no real depth.
    let depth = p
        .trim_end_matches('\\')
        .split('\\')
        .filter(|s| !s.is_empty())
        .count();
    // A network share's root (\\server\share) counts like a drive root.
    if depth <= 1 || (p.starts_with("\\\\") && depth <= 2) {
        return true;
    }
    let parent = p.rsplit_once('\\').map(|(a, _)| a).unwrap_or("");
    protected.iter().any(|d| match d.strip_suffix("\\*") {
        Some(dir) => dir == p || dir == parent,
        None => d == &p,
    })
}

/// The last check before a folder is deleted with everything in it: the
/// path as written and, when it exists, where it really leads (through
/// junctions, symbolic links and short 8.3 names) must both be safe.
pub fn guard_dir_delete(path: &std::path::Path, protected: &[String]) -> anyhow::Result<()> {
    let written = path.to_string_lossy();
    if is_protected_dir(&written, protected) {
        anyhow::bail!("refusing to delete protected folder {written}");
    }
    // A link is removed on its own (see `is_link`), so only where a real
    // folder leads matters.
    if is_link(path) {
        return Ok(());
    }
    if let Ok(real) = std::fs::canonicalize(path) {
        let real = real.to_string_lossy().to_string();
        if is_protected_dir(&real, protected) {
            anyhow::bail!("refusing to delete {written}: it leads to protected folder {real}");
        }
    }
    Ok(())
}

/// Whether `path` is a link to somewhere else (a symbolic link or a
/// junction). Deleting one must remove only the link.
pub fn is_link(path: &std::path::Path) -> bool {
    std::fs::symlink_metadata(path)
        .map(|m| m.file_type().is_symlink())
        .unwrap_or(false)
}

/// Registry keys that must never be deleted as a whole.
const PROTECTED_KEY_SUFFIXES: &[&str] = &[
    "software",
    "software\\classes",
    "software\\classes\\*",
    "software\\classes\\clsid",
    "software\\classes\\interface",
    "software\\classes\\typelib",
    "software\\classes\\appid",
    "software\\classes\\directory",
    "software\\classes\\directory\\shell",
    "software\\classes\\directory\\background\\shell",
    "software\\classes\\folder",
    "software\\classes\\drive",
    "software\\classes\\installer",
    "software\\classes\\local settings",
    "software\\microsoft",
    "software\\microsoft\\windows",
    "software\\microsoft\\windows\\currentversion",
    "software\\microsoft\\windows\\currentversion\\run",
    "software\\microsoft\\windows\\currentversion\\runonce",
    "software\\microsoft\\windows\\currentversion\\uninstall",
    "software\\microsoft\\windows\\currentversion\\app paths",
    "software\\microsoft\\windows\\currentversion\\explorer",
    "software\\microsoft\\windows\\currentversion\\shell extensions",
    "software\\microsoft\\windows\\currentversion\\shell extensions\\approved",
    "software\\microsoft\\windows nt",
    "software\\microsoft\\windows nt\\currentversion",
    "software\\policies",
    "software\\registeredapplications",
    "software\\clients",
    "software\\wow6432node",
    "system",
    "system\\currentcontrolset",
    "system\\currentcontrolset\\services",
    "system\\currentcontrolset\\control",
    "system\\currentcontrolset\\control\\session manager",
    "system\\currentcontrolset\\control\\session manager\\environment",
    "system\\currentcontrolset\\enum",
    "system\\currentcontrolset\\control\\class",
    "system\\currentcontrolset\\control\\safeboot",
    "system\\currentcontrolset\\control\\lsa",
    "system\\currentcontrolset\\control\\nls",
    "system\\currentcontrolset\\control\\print",
    "system\\currentcontrolset\\control\\terminal server",
    "system\\mounteddevices",
    "system\\select",
    "system\\setup",
    "sam",
    "security",
    "hardware",
    "bcd00000000",
    "components",
    "drivers",
    "schema",
    "software\\classes\\systemfileassociations",
    "software\\classes\\exefile",
    "software\\classes\\lnkfile",
    "software\\classes\\.exe",
    "software\\classes\\.lnk",
    "software\\classes\\.dll",
    "software\\microsoft\\windows\\currentversion\\policies",
    "software\\microsoft\\windows\\currentversion\\shell extensions\\blocked",
    "software\\microsoft\\windows\\currentversion\\explorer\\shell folders",
    "software\\microsoft\\windows\\currentversion\\explorer\\user shell folders",
    "software\\microsoft\\windows\\currentversion\\installer",
    "software\\microsoft\\windows nt\\currentversion\\winlogon",
    "software\\microsoft\\windows nt\\currentversion\\image file execution options",
    "software\\microsoft\\windows nt\\currentversion\\profilelist",
    "software\\microsoft\\windows nt\\currentversion\\fonts",
    "software\\microsoft\\windows nt\\currentversion\\windows",
    "software\\microsoft\\cryptography",
    "software\\microsoft\\windows defender",
    "software\\microsoft\\.netframework",
    "software\\microsoft\\net framework setup",
    "software\\classes\\wow6432node",
    "control panel",
    "control panel\\desktop",
    "keyboard layout",
    "environment",
];

pub fn is_protected_key(path: &str) -> bool {
    let n = fold(&normalize_reg_path(path));
    let Some((hive, rest)) = n.split_once('\\') else {
        return true; // hive root
    };
    // HKU\<sid> and HKU\<sid>\Software are profile roots.
    let rest = if hive == "hku" {
        match rest.split_once('\\') {
            // HKU\<sid>_Classes is that user's Software\Classes.
            Some((sid, r)) if sid.ends_with("_classes") => format!("software\\classes\\{r}"),
            Some((_, r)) => r.to_string(),
            None => return true,
        }
    } else {
        rest.to_string()
    };
    // Treat the 32-bit view the same as the 64-bit one, and every numbered
    // control set (ControlSet001…) like CurrentControlSet.
    let rest = rest.replace("software\\wow6432node\\", "software\\");
    let rest = match rest.strip_prefix("system\\controlset") {
        Some(after) if after.len() >= 3 && after[..3].bytes().all(|b| b.is_ascii_digit()) => {
            format!("system\\currentcontrolset{}", &after[3..])
        }
        _ => rest,
    };
    // HKLM\SOFTWARE\Classes\Wow6432Node mirrors Classes.
    let rest = rest.replace("software\\classes\\wow6432node\\", "software\\classes\\");
    // HKCR is a merged view of Software\Classes.
    let rest = if hive == "hkcr" {
        format!("software\\classes\\{rest}")
    } else {
        rest
    };
    PROTECTED_KEY_SUFFIXES.contains(&rest.as_str())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_commands() {
        assert_eq!(
            split_command_line("\"C:\\Program Files\\A\\unins000.exe\" /SILENT"),
            (
                "C:\\Program Files\\A\\unins000.exe".into(),
                "/SILENT".into()
            )
        );
        assert_eq!(
            split_command_line("C:\\Program Files\\A B\\uninst.exe /x"),
            ("C:\\Program Files\\A B\\uninst.exe".into(), "/x".into())
        );
        assert_eq!(
            split_command_line("MsiExec.exe /X{1234}"),
            ("MsiExec.exe".into(), "/X{1234}".into())
        );
        assert_eq!(
            split_command_line("C:\\a\\b.exe"),
            ("C:\\a\\b.exe".into(), "".into())
        );
    }

    #[test]
    fn globs() {
        assert!(glob_match(
            "HKLM\\SOFTWARE\\Microsoft\\*",
            "hklm\\software\\microsoft\\x\\y"
        ));
        assert!(glob_match("*.etl", "C:\\a\\b.ETL"));
        assert!(glob_match("a?c", "abc"));
        assert!(!glob_match("a?c", "abbc"));
        assert!(is_excluded(
            "HKCU\\Software\\X\\Bags\\1",
            &["HKCU\\Software\\X\\Bags".into()]
        ));
        assert!(!is_excluded(
            "HKCU\\Software\\X\\BagsMore",
            &["HKCU\\Software\\X\\Bags".into()]
        ));
    }

    #[test]
    fn protected_keys() {
        assert!(is_protected_key("HKLM"));
        assert!(is_protected_key("HKLM\\SOFTWARE"));
        assert!(is_protected_key(
            "HKLM\\SOFTWARE\\WOW6432Node\\Microsoft\\Windows\\CurrentVersion\\Run"
        ));
        assert!(is_protected_key("HKCR\\CLSID"));
        assert!(is_protected_key("HKU\\S-1-5-21-1\\Software"));
        assert!(!is_protected_key("HKLM\\SOFTWARE\\Vendor\\App"));
        assert!(!is_protected_key(
            "HKCU\\Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\App"
        ));
    }

    #[test]
    fn protected_dirs_rules() {
        std::env::set_var("ProgramFiles", "C:\\Program Files");
        let p = protected_dirs();
        assert!(is_protected_dir("C:\\", &p));
        assert!(is_protected_dir("C:\\Program Files\\", &p));
        assert!(is_protected_dir("c:\\program files", &p));
        assert!(!is_protected_dir("C:\\Program Files\\Vendor", &p));
        assert!(is_protected_dir("D:", &p));
    }

    /// The variables a Windows 11 PC sets, for tests that must not depend
    /// on the machine running them.
    fn win_env(s: &str) -> String {
        let vars = [
            ("SystemDrive", "C:"),
            ("SystemRoot", "C:\\Windows"),
            ("ProgramFiles", "C:\\Program Files"),
            ("ProgramFiles(x86)", "C:\\Program Files (x86)"),
            ("ProgramW6432", "C:\\Program Files"),
            ("CommonProgramFiles", "C:\\Program Files\\Common Files"),
            (
                "CommonProgramFiles(x86)",
                "C:\\Program Files (x86)\\Common Files",
            ),
            ("CommonProgramW6432", "C:\\Program Files\\Common Files"),
            ("ProgramData", "C:\\ProgramData"),
            ("PUBLIC", "C:\\Users\\Public"),
            ("USERPROFILE", "C:\\Users\\Sean"),
            ("APPDATA", "C:\\Users\\Sean\\AppData\\Roaming"),
            ("LOCALAPPDATA", "C:\\Users\\Sean\\AppData\\Local"),
            ("TEMP", "C:\\Users\\Sean\\AppData\\Local\\Temp"),
            ("OneDrive", "C:\\Users\\Sean\\OneDrive"),
        ];
        let mut out = s.to_string();
        for (k, v) in vars {
            out = out.replace(&format!("%{k}%"), v);
        }
        out
    }

    /// Every way of writing a path that Windows treats as the same folder.
    fn spellings(p: &str) -> Vec<String> {
        let leaf = p.rsplit('\\').next().unwrap_or(p);
        vec![
            p.to_string(),
            p.to_uppercase(),
            format!("{p}\\"),
            format!("{p}\\\\"),
            p.replace('\\', "/"),
            format!("\\\\?\\{p}"),
            format!("\\??\\{p}"),
            format!("{p}."),
            format!("{p} "),
            format!("{p}\\."),
            format!("{p}\\x\\.."),
            format!("{p}\\..\\{leaf}"),
            p.replace('\\', "\\\\"),
            p.replacen('\\', "\\.\\", 1),
        ]
    }

    #[test]
    fn every_critical_folder_is_refused_however_it_is_written() {
        let prot = protected_dirs_with(&win_env);
        let critical = [
            "C:\\",
            "C:\\Windows",
            "C:\\Windows\\System32",
            "C:\\Windows\\SysWOW64",
            "C:\\Windows\\WinSxS",
            "C:\\Windows\\Installer",
            "C:\\Windows\\System32\\config",
            "C:\\Windows\\System32\\drivers",
            "C:\\Windows\\System32\\DriverStore",
            "C:\\Windows\\Fonts",
            "C:\\Program Files",
            "C:\\Program Files (x86)",
            "C:\\Program Files\\Common Files",
            "C:\\Program Files\\WindowsApps",
            "C:\\Program Files\\Windows Defender",
            "C:\\ProgramData",
            "C:\\ProgramData\\Microsoft",
            "C:\\Users",
            "C:\\Users\\Sean",
            "C:\\Users\\Someone Else",
            "C:\\Users\\Public",
            "C:\\Users\\Sean\\Documents",
            "C:\\Users\\Sean\\Desktop",
            "C:\\Users\\Sean\\AppData",
            "C:\\Users\\Sean\\AppData\\Roaming",
            "C:\\Users\\Sean\\AppData\\Local",
            "C:\\Users\\Sean\\AppData\\Local\\Temp",
            "C:\\Users\\Sean\\OneDrive",
            "C:\\$Recycle.Bin",
            "C:\\System Volume Information",
            "C:\\Recovery",
            "D:\\",
        ];
        for dir in critical {
            for s in spellings(dir) {
                assert!(is_protected_dir(&s, &prot), "{s} must be protected");
            }
        }
        // Climbing out of an ordinary folder lands on a protected one.
        assert!(is_protected_dir("C:\\Program Files\\Vendor\\..", &prot));
        assert!(is_protected_dir(
            "C:\\Program Files\\Vendor\\..\\..\\Windows",
            &prot
        ));
        assert!(is_protected_dir("C:\\..\\..\\Windows", &prot));
        assert!(is_protected_dir("\\\\?\\UNC\\server\\share", &prot));
    }

    #[test]
    fn ordinary_program_folders_can_still_be_removed() {
        let prot = protected_dirs_with(&win_env);
        for dir in [
            "C:\\Program Files\\Vendor\\App",
            "C:\\Program Files\\Vendor",
            "C:\\Program Files (x86)\\Vendor",
            "C:\\ProgramData\\Vendor",
            "C:\\Users\\Sean\\AppData\\Roaming\\Vendor",
            "C:\\Users\\Sean\\AppData\\Local\\Programs\\App",
            "C:\\Users\\Sean\\AppData\\Local\\Temp\\setup-123",
            "C:\\Windows\\Installer\\{12345678-ABCD}",
            "C:\\Tools\\App",
            "C:\\Python312",
            "D:\\Games\\Game",
        ] {
            assert!(!is_protected_dir(dir, &prot), "{dir} should be removable");
        }
    }

    #[test]
    fn lexical_paths() {
        assert_eq!(
            lexical_path("C:/Program Files/./A/../B. "),
            "c:\\program files\\b"
        );
        assert_eq!(lexical_path("\\\\?\\C:\\Windows\\"), "c:\\windows");
        assert_eq!(lexical_path("C:\\..\\.."), "c:\\");
        assert_eq!(
            lexical_path("\\\\?\\UNC\\srv\\share\\a\\..\\..\\.."),
            "\\\\srv\\share"
        );
    }

    #[test]
    fn every_critical_key_is_refused_however_it_is_written() {
        let keys = [
            "HKLM",
            "HKEY_LOCAL_MACHINE",
            "HKLM\\SOFTWARE",
            "HKLM\\SOFTWARE\\Microsoft",
            "HKLM\\SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\Run",
            "HKLM\\SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\Uninstall",
            "HKLM\\SOFTWARE\\Microsoft\\Windows NT\\CurrentVersion\\Winlogon",
            "HKLM\\SOFTWARE\\Microsoft\\Windows NT\\CurrentVersion\\Image File Execution Options",
            "HKLM\\SOFTWARE\\Classes",
            "HKLM\\SOFTWARE\\Classes\\CLSID",
            "HKLM\\SOFTWARE\\Classes\\exefile",
            "HKLM\\SOFTWARE\\Classes\\.exe",
            "HKLM\\SOFTWARE\\WOW6432Node",
            "HKLM\\SOFTWARE\\WOW6432Node\\Microsoft",
            "HKLM\\SOFTWARE\\Classes\\WOW6432Node",
            "HKLM\\SOFTWARE\\Classes\\WOW6432Node\\CLSID",
            "HKLM\\SYSTEM",
            "HKLM\\SYSTEM\\CurrentControlSet\\Services",
            "HKLM\\SYSTEM\\ControlSet001\\Services",
            "HKLM\\SYSTEM\\ControlSet002\\Control",
            "HKLM\\SYSTEM\\CurrentControlSet\\Enum",
            "HKLM\\SYSTEM\\MountedDevices",
            "HKLM\\SAM",
            "HKLM\\SECURITY",
            "HKLM\\HARDWARE",
            "HKLM\\BCD00000000",
            "HKCR",
            "HKCR\\CLSID",
            "HKCR\\*",
            "HKCR\\Directory\\shell",
            "HKCU",
            "HKCU\\Software",
            "HKCU\\Software\\Classes",
            "HKCU\\Environment",
            "HKCU\\Control Panel",
            "HKCU\\Software\\Microsoft\\Windows\\CurrentVersion\\Policies",
            "HKU",
            "HKU\\.DEFAULT",
            "HKU\\S-1-5-21-1-2-3-1001",
            "HKU\\S-1-5-21-1-2-3-1001\\Software",
            "HKU\\S-1-5-21-1-2-3-1001_Classes",
            "HKU\\S-1-5-21-1-2-3-1001_Classes\\CLSID",
        ];
        for k in keys {
            for s in [
                k.to_string(),
                k.to_lowercase(),
                format!("{k}\\"),
                format!("\\{k}"),
                k.replace('\\', "\\\\"),
            ] {
                assert!(is_protected_key(&s), "{s} must be protected");
            }
        }
        for ok in [
            "HKLM\\SOFTWARE\\Vendor",
            "HKLM\\SOFTWARE\\WOW6432Node\\Vendor",
            "HKCU\\Software\\Vendor\\App",
            "HKLM\\SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\App",
            "HKLM\\SYSTEM\\CurrentControlSet\\Services\\VendorService",
            "HKCR\\CLSID\\{12345678-1234-1234-1234-123456789012}",
            "HKU\\S-1-5-21-1-2-3-1001_Classes\\.vendorfile",
        ] {
            assert!(!is_protected_key(ok), "{ok} should be removable");
        }
    }

    #[test]
    fn the_registry_itself_refuses_protected_keys() {
        use crate::model::KeyData;
        use crate::platform::{MemRegistry, Registry};
        let r = MemRegistry::new();
        r.put(KeyData::new("HKLM\\SOFTWARE\\Microsoft"));
        r.put(KeyData::new("HKLM\\SOFTWARE\\Vendor"));
        assert!(r.delete_tree("HKLM\\SOFTWARE\\Microsoft").is_err());
        assert!(r.delete_tree("HKLM\\software\\microsoft\\").is_err());
        assert!(r.exists("HKLM\\SOFTWARE\\Microsoft"));
        assert!(r.delete_tree("HKLM\\SOFTWARE\\Vendor").is_ok());
    }

    #[test]
    fn links_are_removed_without_touching_their_target() {
        let base = std::env::temp_dir().join(format!("installscope-link-{}", std::process::id()));
        let target = base.join("target");
        std::fs::create_dir_all(&target).unwrap();
        std::fs::write(target.join("keep.txt"), "keep").unwrap();
        let link = base.join("link");
        #[cfg(unix)]
        std::os::unix::fs::symlink(&target, &link).unwrap();
        #[cfg(windows)]
        if std::os::windows::fs::symlink_dir(&target, &link).is_err() {
            // Creating symbolic links needs Developer Mode or elevation.
            let _ = std::fs::remove_dir_all(&base);
            return;
        }
        assert!(is_link(&link));
        assert!(!is_link(&target));
        let prot = protected_dirs_with(&win_env);
        assert!(guard_dir_delete(&link, &prot).is_ok());
        let _ = std::fs::remove_dir_all(&base);
    }
}

/// SHA-256 of `data` (FIPS 180-4), as 32 bytes.
pub fn sha256(data: &[u8]) -> [u8; 32] {
    sha256_reader(std::io::Cursor::new(data)).expect("reading a byte slice cannot fail")
}

/// SHA-256 of a stream with bounded memory; read errors are never hidden.
/// Uses the same compression function as `sha256`, not a separate algorithm.
pub fn sha256_reader(input: impl std::io::Read) -> std::io::Result<[u8; 32]> {
    use std::io::Read;
    let mut input = std::io::BufReader::with_capacity(64 * 1024, input);
    const K: [u32; 64] = [
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4,
        0xab1c5ed5, 0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe,
        0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f,
        0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7,
        0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc,
        0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b,
        0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116,
        0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
        0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7,
        0xc67178f2,
    ];
    let mut h: [u32; 8] = [
        0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab,
        0x5be0cd19,
    ];
    let mut compress = |block: &[u8]| {
        let mut w = [0u32; 64];
        for (i, c) in block.chunks_exact(4).enumerate() {
            w[i] = u32::from_be_bytes([c[0], c[1], c[2], c[3]]);
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16]
                .wrapping_add(s0)
                .wrapping_add(w[i - 7])
                .wrapping_add(s1);
        }
        let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut hh] = h;
        for i in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ (!e & g);
            let t1 = hh
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(K[i])
                .wrapping_add(w[i]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let t2 = s0.wrapping_add(maj);
            hh = g;
            g = f;
            f = e;
            e = d.wrapping_add(t1);
            d = c;
            c = b;
            b = a;
            a = t1.wrapping_add(t2);
        }
        for (x, y) in h.iter_mut().zip([a, b, c, d, e, f, g, hh]) {
            *x = x.wrapping_add(y);
        }
    };
    let mut block = [0u8; 64];
    let mut used = 0;
    let mut length = 0u64;
    loop {
        match input.read(&mut block[used..]) {
            Ok(0) => break,
            Ok(n) => {
                length = length
                    .checked_add(n as u64)
                    .ok_or_else(|| std::io::Error::other("SHA-256 input length overflow"))?;
                used += n;
                if used == 64 {
                    compress(&block);
                    used = 0;
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e),
        }
    }
    // Padding: 0x80, zeros, then the bit length, in one or two blocks.
    let mut tail = [0u8; 128];
    tail[..used].copy_from_slice(&block[..used]);
    tail[used] = 0x80;
    let n = if used < 56 { 64 } else { 128 };
    let bits = length
        .checked_mul(8)
        .ok_or_else(|| std::io::Error::other("SHA-256 input too long"))?;
    tail[n - 8..n].copy_from_slice(&bits.to_be_bytes());
    for block in tail[..n].chunks_exact(64) {
        compress(block);
    }
    let mut out = [0u8; 32];
    for (o, v) in out.chunks_exact_mut(4).zip(h) {
        o.copy_from_slice(&v.to_be_bytes());
    }
    Ok(out)
}

/// First position of `needle` in `hay` (Boyer-Moore-Horspool; the skip
/// table lives on the stack, nothing is allocated).
pub fn find_bytes(hay: &[u8], needle: &[u8]) -> Option<usize> {
    let n = needle.len();
    if n == 0 {
        return Some(0);
    }
    if n > hay.len() {
        return None;
    }
    if n == 1 {
        return hay.iter().position(|&b| b == needle[0]);
    }
    let mut skip = [n; 256];
    for (i, &b) in needle[..n - 1].iter().enumerate() {
        skip[b as usize] = n - 1 - i;
    }
    let last = needle[n - 1];
    let mut at = 0usize;
    while at + n <= hay.len() {
        let c = hay[at + n - 1];
        if c == last && hay[at..at + n - 1] == needle[..n - 1] {
            return Some(at);
        }
        at += skip[c as usize];
    }
    None
}

/// A folder that is deleted when dropped, for tests.
#[cfg(test)]
pub struct TestDir(std::path::PathBuf);

#[cfg(test)]
impl TestDir {
    pub fn path(&self) -> &std::path::Path {
        &self.0
    }
}

#[cfg(test)]
impl Drop for TestDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// A new empty folder under the system's temporary folder.
#[cfg(test)]
pub fn tempdir() -> std::io::Result<TestDir> {
    use std::sync::atomic::{AtomicU64, Ordering};
    static N: AtomicU64 = AtomicU64::new(0);
    let base = std::env::temp_dir();
    loop {
        let p = base.join(format!(
            "installscope-test-{}-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::Relaxed),
            crate::time::unix_now()
        ));
        match std::fs::create_dir(&p) {
            Ok(()) => return Ok(TestDir(p)),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e),
        }
    }
}

#[cfg(test)]
mod hash_search_tests {
    use super::*;

    fn hex(b: &[u8]) -> String {
        b.iter().map(|x| format!("{x:02x}")).collect()
    }

    #[test]
    fn sha256_matches_the_standard_vectors() {
        assert_eq!(
            hex(&sha256(b"")),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(
            hex(&sha256(b"abc")),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(
            hex(&sha256(
                b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq"
            )),
            "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1"
        );
        assert_eq!(
            hex(&sha256(&vec![b'a'; 1_000_000])),
            "cdc76e5c9914fb9281a1c7e284d73e67f1809a48a497200e046d39ccc7112cd0"
        );
        // Lengths around the padding boundary.
        for n in [55, 56, 63, 64, 65, 119, 120] {
            assert_eq!(sha256(&vec![7u8; n]), sha256(&vec![7u8; n]));
        }
    }

    #[test]
    fn find_bytes_agrees_with_a_naive_search() {
        let hay = b"xxOpenCandyxxopencandyOpenCandy\0O\0p";
        for needle in [
            &b"OpenCandy"[..],
            b"opencandy",
            b"x",
            b"O\0p",
            b"nope",
            b"",
            b"yOpen",
        ] {
            let naive = (0..=hay.len().saturating_sub(needle.len()))
                .find(|&i| hay[i..].starts_with(needle));
            let naive = if needle.len() > hay.len() {
                None
            } else {
                naive
            };
            assert_eq!(find_bytes(hay, needle), naive, "{needle:?}");
        }
        assert_eq!(find_bytes(b"ab", b"abc"), None);
    }
}

#[cfg(test)]
mod streaming_hash_tests {
    use super::*;
    use std::io::{self, Read};

    struct Fragmented {
        bytes: std::io::Cursor<Vec<u8>>,
        interrupt: bool,
    }
    impl Read for Fragmented {
        fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
            if self.interrupt {
                self.interrupt = false;
                return Err(io::ErrorKind::Interrupted.into());
            }
            let n = out.len().min(7);
            self.bytes.read(&mut out[..n])
        }
    }
    #[test]
    fn fragmented_reads_and_padding_boundaries_match() {
        for n in [0, 1, 55, 56, 63, 64, 65, 127, 128, 129, 65535, 65536, 65537] {
            let data = vec![0x7f; n];
            let reader = Fragmented {
                bytes: std::io::Cursor::new(data.clone()),
                interrupt: true,
            };
            assert_eq!(sha256_reader(reader).unwrap(), sha256(&data));
        }
    }
    #[test]
    fn stream_padding_matches_independent_hashlib_vectors() {
        assert_eq!(
            sha256_reader(std::io::Cursor::new(vec![0x7f; 55])).unwrap(),
            [
                28, 23, 180, 3, 197, 157, 100, 206, 188, 43, 137, 140, 90, 236, 210, 221, 109, 151,
                193, 24, 25, 182, 147, 109, 97, 72, 13, 176, 177, 227, 108, 131
            ]
        );
        assert_eq!(
            sha256_reader(std::io::Cursor::new(vec![0x7f; 56])).unwrap(),
            [
                140, 95, 72, 90, 57, 98, 56, 195, 26, 140, 225, 94, 214, 97, 46, 151, 192, 131,
                154, 111, 76, 220, 123, 13, 26, 23, 12, 18, 167, 188, 131, 164
            ]
        );
        assert_eq!(
            sha256_reader(std::io::Cursor::new(vec![0x7f; 63])).unwrap(),
            [
                18, 28, 13, 7, 233, 66, 136, 195, 60, 11, 95, 209, 88, 51, 66, 109, 70, 165, 191,
                221, 244, 212, 167, 136, 55, 225, 16, 8, 226, 153, 245, 78
            ]
        );
        assert_eq!(
            sha256_reader(std::io::Cursor::new(vec![0x7f; 64])).unwrap(),
            [
                224, 32, 85, 25, 246, 188, 222, 66, 8, 253, 94, 154, 236, 231, 47, 81, 142, 180,
                206, 88, 121, 202, 24, 50, 166, 203, 34, 29, 83, 74, 45, 126
            ]
        );
        assert_eq!(
            sha256_reader(std::io::Cursor::new(vec![0x7f; 65])).unwrap(),
            [
                52, 248, 75, 21, 93, 122, 83, 157, 194, 57, 183, 206, 253, 178, 129, 52, 210, 226,
                70, 179, 181, 63, 202, 117, 221, 36, 17, 103, 236, 199, 188, 81
            ]
        );
        assert_eq!(
            sha256_reader(std::io::Cursor::new(vec![0x7f; 65537])).unwrap(),
            [
                118, 184, 192, 251, 99, 163, 96, 74, 238, 84, 173, 163, 1, 228, 68, 252, 62, 170,
                45, 21, 151, 120, 88, 176, 22, 14, 91, 135, 133, 122, 139, 102
            ]
        );
    }

    #[test]
    fn stream_error_is_propagated() {
        struct Broken;
        impl Read for Broken {
            fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
                Err(io::ErrorKind::PermissionDenied.into())
            }
        }
        assert_eq!(
            sha256_reader(Broken).unwrap_err().kind(),
            io::ErrorKind::PermissionDenied
        );
    }
}
