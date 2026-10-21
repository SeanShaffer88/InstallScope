//! Automatic exclusions: volatile files and registry areas that change on
//! their own (temporary files, logs, caches) and would otherwise bury an
//! installer's real changes.
//!
//! Two layers:
//! - a built-in list, applied whenever "automatic exclusions" is on; it is not
//!   stored in the settings file, so it improves with every SetupSweep update;
//! - *learned* noise: two snapshots taken a minute apart with nothing
//!   installed in between show what this particular PC changes by itself.

use crate::model::{fold, ChangeSet};
use crate::platform::expand_env;
use crate::util::Exclusions;
use std::collections::BTreeSet;

/// Volatile files and folders, any browser or profile. `*` matches across
/// folder levels; a pattern also covers everything beneath it.
pub const FILE_PATTERNS: &[&str] = &[
    // Temporary files
    "%TEMP%",
    "%LOCALAPPDATA%\\Temp",
    "%SystemRoot%\\Temp",
    "*.tmp",
    // Windows logs, traces and diagnostics
    "%SystemRoot%\\Logs",
    "%SystemRoot%\\Prefetch",
    "%SystemRoot%\\SoftwareDistribution",
    "%SystemRoot%\\ServiceProfiles",
    "%SystemRoot%\\System32\\LogFiles",
    "%SystemRoot%\\System32\\winevt",
    "%SystemRoot%\\System32\\sru",
    "*.etl",
    "*.evtx",
    "*.blf",
    "*.regtrans-ms",
    "*\\ntuser.dat.log*",
    "*\\UsrClass.dat*",
    "%ProgramData%\\Microsoft\\Windows\\WER",
    "%ProgramData%\\Microsoft\\Windows Defender",
    "%ProgramData%\\Microsoft\\Windows Security Health",
    "%LOCALAPPDATA%\\Microsoft\\Windows\\WER",
    "%LOCALAPPDATA%\\CrashDumps",
    "%ProgramData%\\Microsoft\\Diagnosis",
    "%ProgramData%\\Microsoft\\Windows\\DeliveryOptimization",
    "%ProgramData%\\USOPrivate",
    "%ProgramData%\\USOShared",
    "%ProgramData%\\Microsoft\\Search",
    "%LOCALAPPDATA%\\Microsoft\\OneDrive\\logs",
    "%LOCALAPPDATA%\\Microsoft\\OneDrive\\setup\\logs",
    // Windows shell caches
    "%LOCALAPPDATA%\\Microsoft\\Windows\\Explorer",
    "%LOCALAPPDATA%\\Microsoft\\Windows\\Caches",
    "%LOCALAPPDATA%\\Microsoft\\Windows\\INetCache",
    "%LOCALAPPDATA%\\Microsoft\\Windows\\WebCache",
    "%LOCALAPPDATA%\\Microsoft\\Windows\\History",
    "%LOCALAPPDATA%\\Microsoft\\Windows\\Notifications",
    "%LOCALAPPDATA%\\Microsoft\\Windows\\ActionCenterCache",
    "%LOCALAPPDATA%\\ConnectedDevicesPlatform",
    "%APPDATA%\\Microsoft\\Windows\\Recent",
    // Graphics shader caches
    "%LOCALAPPDATA%\\D3DSCache",
    "%LOCALAPPDATA%\\NVIDIA\\DXCache",
    "%LOCALAPPDATA%\\NVIDIA\\GLCache",
    "%LOCALAPPDATA%\\AMD\\DxCache",
    "%LOCALAPPDATA%\\AMD\\DxcCache",
    "%LOCALAPPDATA%\\AMD\\GLCache",
    "%LOCALAPPDATA%\\Intel\\ShaderCache",
    "%ProgramData%\\NVIDIA Corporation\\NV_Cache",
    // Chromium browsers (Chrome, Edge, Brave, Vivaldi, Opera…) and Electron apps
    "%LOCALAPPDATA%\\*\\Cache",
    "%LOCALAPPDATA%\\*\\Code Cache",
    "%LOCALAPPDATA%\\*\\GPUCache",
    "%LOCALAPPDATA%\\*\\GrShaderCache",
    "%LOCALAPPDATA%\\*\\ShaderCache",
    "%LOCALAPPDATA%\\*\\DawnCache",
    "%LOCALAPPDATA%\\*\\DawnGraphiteCache",
    "%LOCALAPPDATA%\\*\\DawnWebGPUCache",
    "%LOCALAPPDATA%\\*\\Service Worker\\CacheStorage",
    "%LOCALAPPDATA%\\*\\Service Worker\\ScriptCache",
    "%LOCALAPPDATA%\\*\\Crashpad",
    "%LOCALAPPDATA%\\*\\User Data\\*\\Sessions",
    "%LOCALAPPDATA%\\*\\User Data\\*\\History*",
    "%LOCALAPPDATA%\\*\\User Data\\*\\Network",
    "%LOCALAPPDATA%\\*\\User Data\\*\\Local Storage",
    "%LOCALAPPDATA%\\*\\User Data\\*\\Session Storage",
    "%LOCALAPPDATA%\\*\\User Data\\*\\IndexedDB",
    "%LOCALAPPDATA%\\*\\User Data\\*\\*.ldb",
    "%LOCALAPPDATA%\\*\\User Data\\*\\*-journal",
    "%APPDATA%\\*\\Cache",
    "%APPDATA%\\*\\Code Cache",
    "%APPDATA%\\*\\GPUCache",
    "%APPDATA%\\*\\DawnCache",
    "%APPDATA%\\*\\DawnGraphiteCache",
    "%APPDATA%\\*\\DawnWebGPUCache",
    "%APPDATA%\\*\\Service Worker\\CacheStorage",
    "%APPDATA%\\*\\Crashpad",
    // Firefox and other Gecko browsers
    "%LOCALAPPDATA%\\*\\Profiles\\*\\cache2",
    "%LOCALAPPDATA%\\*\\Profiles\\*\\startupCache",
    "%LOCALAPPDATA%\\*\\Profiles\\*\\thumbnails",
    "%APPDATA%\\*\\Profiles\\*\\sessionstore-backups",
    "%APPDATA%\\*\\Profiles\\*\\datareporting",
    "%APPDATA%\\*\\Profiles\\*\\crashes",
    "%APPDATA%\\*\\Profiles\\*\\storage\\default",
    "%APPDATA%\\*\\Profiles\\*\\*.sqlite-wal",
    "%APPDATA%\\*\\Profiles\\*\\*.sqlite-shm",
    "%APPDATA%\\*\\Profiles\\*\\sessionstore.jsonlz4",
    "%APPDATA%\\*\\Profiles\\*\\recovery.*",
    // Folder view settings
    "*\\desktop.ini",
];

/// Registry areas Windows rewrites constantly (most-recently-used lists,
/// folder views, usage statistics, caches).
pub const REGISTRY_PATTERNS: &[&str] = &[
    // Windows Search's own bookkeeping (indexer state, removed Store apps).
    "HKLM\\SOFTWARE\\Microsoft\\Windows Search",
    "HKLM\\SOFTWARE\\WOW6432Node\\Microsoft\\Windows Search",
    // Windows' own update and feature-delivery client state.
    "HKLM\\SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\Wosc",
    // WMI provider registrations, kept in memory and rewritten as WMI runs.
    "HKLM\\SOFTWARE\\Microsoft\\Wbem\\Transports\\Decoupled",
    // Windows text input statistics (typing insights, input personalization).
    "HKCU\\Software\\Microsoft\\input",
    "HKCU\\Software\\Microsoft\\InputPersonalization",
    "HKCU\\Software\\Microsoft\\Windows\\Shell\\BagMRU",
    "HKCU\\Software\\Microsoft\\Windows\\Shell\\Bags",
    "HKCU\\Software\\Classes\\Local Settings\\Software\\Microsoft\\Windows\\Shell",
    "HKCU\\Software\\Classes\\Local Settings\\MuiCache",
    "HKCU\\Software\\Microsoft\\Windows NT\\CurrentVersion\\AppCompatFlags\\Compatibility Assistant",
    "HKCU\\Software\\Microsoft\\Windows\\CurrentVersion\\Explorer\\RecentDocs",
    "HKCU\\Software\\Microsoft\\Windows\\CurrentVersion\\Explorer\\UserAssist",
    "HKCU\\Software\\Microsoft\\Windows\\CurrentVersion\\Explorer\\FeatureUsage",
    "HKCU\\Software\\Microsoft\\Windows\\CurrentVersion\\Explorer\\TypedPaths",
    "HKCU\\Software\\Microsoft\\Windows\\CurrentVersion\\Explorer\\RunMRU",
    "HKCU\\Software\\Microsoft\\Windows\\CurrentVersion\\Explorer\\ComDlg32",
    "HKCU\\Software\\Microsoft\\Windows\\CurrentVersion\\Explorer\\SessionInfo",
    "HKCU\\Software\\Microsoft\\Windows\\CurrentVersion\\Explorer\\TWinUI",
    "HKCU\\Software\\Microsoft\\Windows\\CurrentVersion\\Explorer\\Taskband",
    "HKCU\\Software\\Microsoft\\Windows\\CurrentVersion\\Explorer\\StuckRects3",
    "HKCU\\Software\\Microsoft\\Windows\\CurrentVersion\\Explorer\\WordWheelQuery",
    "HKCU\\Software\\Microsoft\\Windows\\CurrentVersion\\Search\\RecentApps",
    "HKCU\\Software\\Microsoft\\Windows\\CurrentVersion\\CloudStore",
    "HKCU\\Software\\Microsoft\\Windows\\CurrentVersion\\Notifications",
    "HKCU\\Software\\Microsoft\\Windows\\CurrentVersion\\PushNotifications",
    "HKCU\\Software\\Microsoft\\Windows\\CurrentVersion\\Store\\Cache",
    "HKCU\\Software\\Microsoft\\Windows\\CurrentVersion\\Internet Settings\\Connections",
    "HKCU\\Software\\Microsoft\\Windows\\CurrentVersion\\Internet Settings\\5.0\\Cache",
    "HKCU\\Software\\Microsoft\\Windows\\CurrentVersion\\CapabilityAccessManager\\ConsentStore",
    "HKCU\\Software\\Microsoft\\Windows\\CurrentVersion\\DeliveryOptimization",
    "HKCU\\Software\\Microsoft\\OneDrive\\Accounts",
    "HKCU\\Software\\Microsoft\\Edge\\*\\Stats",
    "HKCU\\Software\\Google\\Chrome\\*\\Stats",
    "HKLM\\SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\CapabilityAccessManager\\ConsentStore",
    "HKLM\\SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\DeliveryOptimization",
    "HKLM\\SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\Installer\\InProgress",
    "HKLM\\SOFTWARE\\Microsoft\\Windows NT\\CurrentVersion\\AppCompatFlags\\Compatibility Assistant",
    "HKLM\\SOFTWARE\\Microsoft\\Windows NT\\CurrentVersion\\Superfetch",
    "HKLM\\SOFTWARE\\Microsoft\\Windows NT\\CurrentVersion\\Prefetcher",
    "HKLM\\SOFTWARE\\Microsoft\\Windows Defender",
    "HKLM\\SOFTWARE\\Microsoft\\Windows Defender Security Center",
    "HKLM\\SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\Appx",
    "HKLM\\SOFTWARE\\*Image File Execution Options\\DefenderAgentScan.exe",
    "HKLM\\SOFTWARE\\Microsoft\\Windows Search",
    "HKLM\\SOFTWARE\\Microsoft\\Cryptography\\RNG",
    "HKLM\\SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\WindowsUpdate",
    "HKLM\\SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\Diagnostics",
    "HKLM\\SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\Reliability",
    "HKLM\\SYSTEM\\CurrentControlSet\\Services\\bam",
    "HKLM\\SYSTEM\\CurrentControlSet\\Services\\Tcpip\\Parameters\\Interfaces",
    // Shadow copy writers' diagnostics, written when Windows Installer
    // creates a restore point, and Storage Sense's own bookkeeping.
    "HKLM\\SYSTEM\\CurrentControlSet\\Services\\VSS\\Diag",
    "HKCU\\Software\\Microsoft\\Windows\\CurrentVersion\\StorageSense",
];

pub fn builtin_file_exclusions() -> Vec<String> {
    FILE_PATTERNS.iter().map(|s| s.to_string()).collect()
}

pub fn builtin_registry_exclusions() -> Vec<String> {
    REGISTRY_PATTERNS.iter().map(|s| s.to_string()).collect()
}

/// Header line written above learned patterns in the exclusion lists
/// (lines starting with `#` are comments).
pub const LEARNED_HEADER: &str = "# Learned background noise";

/// What changed between two snapshots taken with nothing installed in
/// between, generalized into exclusion patterns.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Learned {
    pub registry: Vec<String>,
    pub files: Vec<String>,
}

impl Learned {
    pub fn is_empty(&self) -> bool {
        self.registry.is_empty() && self.files.is_empty()
    }
}

fn parent(path: &str) -> Option<&str> {
    path.trim_end_matches('\\')
        .rsplit_once('\\')
        .map(|(p, _)| p)
}

/// Keep only the shortest patterns: drop any path inside another one.
fn collapse(set: BTreeSet<String>) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for p in set {
        // BTreeSet order puts a parent directly before its children
        // (case-folded keys), so checking the last kept entry is enough.
        if let Some(last) = out.last() {
            if fold(&p).starts_with(&format!("{}\\", fold(last))) {
                continue;
            }
        }
        out.push(p);
    }
    out
}

/// Turn background changes into exclusion suggestions.
///
/// Files are generalized to their folder, registry values to their key —
/// unless that folder or key is one of the capture roots (or above one):
/// excluding a whole root would hide real installations, so the individual
/// file is suggested instead (registry values directly in a root are skipped).
/// Anything already excluded is left out.
pub fn learn(
    cs: &ChangeSet,
    file_roots: &[String],
    registry_roots: &[String],
    file_excl: &[String],
    reg_excl: &[String],
) -> Learned {
    let froots: Vec<String> = file_roots
        .iter()
        .map(|r| fold(expand_env(r).trim_end_matches('\\')))
        .collect();
    let rroots: Vec<String> = registry_roots
        .iter()
        .map(|r| fold(r.trim_end_matches('\\')))
        .collect();
    // A path is too broad to exclude if it is a root, or contains one.
    let too_broad = |p: &str, roots: &[String]| {
        let f = fold(p);
        roots
            .iter()
            .any(|r| *r == f || r.starts_with(&format!("{f}\\")))
    };
    let file_excl = Exclusions::new(file_excl);
    let reg_excl = Exclusions::new(reg_excl);

    let mut files = BTreeSet::new();
    let changed_files = cs
        .files_added
        .iter()
        .chain(&cs.files_removed)
        .map(|f| f.path.as_str())
        .chain(cs.files_modified.iter().map(|m| m.after.path.as_str()));
    for path in changed_files {
        if file_excl.matches(path) {
            continue;
        }
        let pick = match parent(path) {
            Some(dir) if !too_broad(dir, &froots) => dir,
            _ => path,
        };
        files.insert(pick.to_string());
    }

    let mut registry = BTreeSet::new();
    let changed_keys = cs
        .keys_added
        .iter()
        .chain(&cs.keys_removed)
        .map(|k| k.path.as_str())
        .chain(cs.values_added.iter().map(|v| v.key.as_str()))
        .chain(cs.values_removed.iter().map(|v| v.key.as_str()))
        .chain(cs.values_modified.iter().map(|v| v.key.as_str()));
    for key in changed_keys {
        if reg_excl.matches(key) || too_broad(key, &rroots) {
            continue;
        }
        registry.insert(key.to_string());
    }

    Learned {
        registry: collapse(registry),
        files: collapse(files),
    }
}

/// Append learned patterns to an exclusion list under a dated header,
/// skipping ones already present.
pub fn append_learned(list: &mut Vec<String>, patterns: &[String], date: &str) {
    let existing: BTreeSet<String> = list.iter().map(|s| fold(s.trim())).collect();
    let new: Vec<&String> = patterns
        .iter()
        .filter(|p| !existing.contains(&fold(p.trim())))
        .collect();
    if new.is_empty() {
        return;
    }
    list.push(format!("{LEARNED_HEADER} ({date})"));
    list.extend(new.into_iter().cloned());
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{FileEntry, RegValue, ValueChange};
    use crate::util::is_excluded;

    fn vc(key: &str) -> ValueChange {
        ValueChange {
            key: key.into(),
            before: RegValue::string("Count", "1"),
            after: RegValue::string("Count", "2"),
        }
    }

    fn file(path: &str) -> FileEntry {
        FileEntry {
            path: path.into(),
            is_dir: false,
            size: 1,
            mtime: 0,
        }
    }

    #[test]
    fn builtin_patterns_cover_browser_caches_and_logs() {
        let excl: Vec<String> = builtin_file_exclusions()
            .iter()
            .map(|s| s.replace("%LOCALAPPDATA%", "C:\\Users\\a\\AppData\\Local"))
            .map(|s| s.replace("%APPDATA%", "C:\\Users\\a\\AppData\\Roaming"))
            .map(|s| s.replace("%SystemRoot%", "C:\\Windows"))
            .collect();
        for p in [
            "C:\\Users\\a\\AppData\\Local\\Google\\Chrome\\User Data\\Default\\Cache\\Cache_Data\\f_0001",
            "C:\\Users\\a\\AppData\\Local\\Vivaldi\\User Data\\Profile 2\\Code Cache\\js\\index",
            "C:\\Users\\a\\AppData\\Local\\BraveSoftware\\Brave-Browser\\User Data\\Default\\Network\\Cookies",
            "C:\\Users\\a\\AppData\\Roaming\\Slack\\Cache\\Cache_Data\\data_1",
            "C:\\Users\\a\\AppData\\Local\\Mozilla\\Firefox\\Profiles\\x.default\\cache2\\entries\\AB",
            "C:\\Windows\\Logs\\CBS\\CBS.log",
            "C:\\Windows\\System32\\winevt\\Logs\\Application.evtx",
            "C:\\Program Files\\Contoso\\setup.tmp",
        ] {
            assert!(is_excluded(p, &excl), "{p}");
        }
        for p in [
            "C:\\Program Files\\Contoso\\contoso.exe",
            "C:\\Users\\a\\AppData\\Roaming\\Contoso\\settings.json",
            "C:\\Users\\a\\AppData\\Local\\Google\\Chrome\\User Data\\Default\\Extensions\\abc\\manifest.json",
        ] {
            assert!(!is_excluded(p, &excl), "{p}");
        }
        let reg = builtin_registry_exclusions();
        assert!(is_excluded(
            "HKCU\\Software\\Microsoft\\Windows\\Shell\\Bags\\12\\Shell",
            &reg
        ));
        assert!(!is_excluded("HKCU\\Software\\Contoso", &reg));
    }

    #[test]
    fn learns_folders_and_keys_but_never_whole_roots() {
        let mut cs = ChangeSet::default();
        cs.files_added = vec![
            file("C:\\Users\\a\\AppData\\Local\\Contoso\\Telemetry\\1.json"),
            file("C:\\Users\\a\\AppData\\Local\\Contoso\\Telemetry\\2.json"),
            file("C:\\Users\\a\\AppData\\Local\\IconCache.db"),
            file("C:\\Users\\a\\AppData\\Local\\Temp\\x.tmp"),
        ];
        cs.values_modified = vec![
            vc("HKCU\\Software\\Contoso\\Stats"),
            vc("HKCU\\Software\\Contoso\\Stats\\Daily"),
            vc("HKCU\\Software"),
        ];
        let l = learn(
            &cs,
            &["C:\\Users\\a\\AppData\\Local".into()],
            &["HKCU\\Software".into()],
            &["C:\\Users\\a\\AppData\\Local\\Temp".into()],
            &[],
        );
        assert_eq!(
            l.files,
            vec![
                "C:\\Users\\a\\AppData\\Local\\Contoso\\Telemetry".to_string(),
                "C:\\Users\\a\\AppData\\Local\\IconCache.db".to_string(),
            ]
        );
        assert_eq!(
            l.registry,
            vec!["HKCU\\Software\\Contoso\\Stats".to_string()]
        );

        let mut list = vec!["%TEMP%".to_string()];
        append_learned(&mut list, &l.files, "2026-10-14");
        assert_eq!(list.len(), 4);
        assert!(list[1].starts_with(LEARNED_HEADER));
        append_learned(&mut list, &l.files, "2026-10-15"); // nothing new
        assert_eq!(list.len(), 4);
    }
}
