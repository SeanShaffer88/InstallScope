//! Junk cleaner: temporary files, caches, crash dumps, Recycle Bin.
//!
//! Only well-known, regenerable locations are touched, and only their
//! *contents*: the folders themselves stay. Files in use are skipped.

use crate::platform::expand_env;
use crate::util::glob_match;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Category {
    /// Stable id, also the translation key (`clean.<id>`).
    pub id: &'static str,
    pub patterns: Vec<&'static str>,
    pub needs_admin: bool,
    pub default_on: bool,
}
crate::json_struct!(Category, write_only, { id, patterns, needs_admin, default_on });

pub fn categories() -> Vec<Category> {
    let c = |id, patterns: Vec<&'static str>, needs_admin, default_on| Category {
        id,
        patterns,
        needs_admin,
        default_on,
    };
    vec![
        c("user_temp", vec!["%TEMP%"], false, true),
        c("windows_temp", vec!["%SystemRoot%\\Temp"], true, true),
        c(
            "crash_dumps",
            vec![
                "%LOCALAPPDATA%\\CrashDumps",
                "%ProgramData%\\Microsoft\\Windows\\WER\\ReportArchive",
                "%ProgramData%\\Microsoft\\Windows\\WER\\ReportQueue",
                "%LOCALAPPDATA%\\Microsoft\\Windows\\WER\\ReportArchive",
            ],
            false,
            true,
        ),
        c(
            "thumbnails",
            vec!["%LOCALAPPDATA%\\Microsoft\\Windows\\Explorer\\thumbcache_*.db"],
            false,
            false,
        ),
        c(
            "shader_cache",
            vec![
                "%LOCALAPPDATA%\\D3DSCache",
                "%LOCALAPPDATA%\\NVIDIA\\DXCache",
                "%LOCALAPPDATA%\\AMD\\DxCache",
            ],
            false,
            true,
        ),
        c(
            "update_cache",
            vec!["%SystemRoot%\\SoftwareDistribution\\Download"],
            true,
            false,
        ),
        c(
            "chrome_cache",
            vec![
                "%LOCALAPPDATA%\\Google\\Chrome\\User Data\\*\\Cache",
                "%LOCALAPPDATA%\\Google\\Chrome\\User Data\\*\\Code Cache",
                "%LOCALAPPDATA%\\Google\\Chrome\\User Data\\*\\GPUCache",
            ],
            false,
            false,
        ),
        c(
            "edge_cache",
            vec![
                "%LOCALAPPDATA%\\Microsoft\\Edge\\User Data\\*\\Cache",
                "%LOCALAPPDATA%\\Microsoft\\Edge\\User Data\\*\\Code Cache",
                "%LOCALAPPDATA%\\Microsoft\\Edge\\User Data\\*\\GPUCache",
            ],
            false,
            false,
        ),
        c(
            "firefox_cache",
            vec!["%LOCALAPPDATA%\\Mozilla\\Firefox\\Profiles\\*\\cache2"],
            false,
            false,
        ),
        c(
            "windows_logs",
            vec![
                "%SystemRoot%\\Logs\\CBS",
                "%SystemRoot%\\Logs\\DISM",
                "%SystemRoot%\\Logs\\MoSetup",
                "%SystemRoot%\\Logs\\WindowsUpdate",
                "%SystemRoot%\\Panther",
                "%SystemRoot%\\inf\\setupapi.*.log",
            ],
            true,
            false,
        ),
        c(
            "memory_dumps",
            vec!["%SystemRoot%\\MEMORY.DMP", "%SystemRoot%\\Minidump", "%SystemRoot%\\LiveKernelReports"],
            true,
            true,
        ),
        c(
            "delivery_optimization",
            vec!["%SystemRoot%\\ServiceProfiles\\NetworkService\\AppData\\Local\\Microsoft\\Windows\\DeliveryOptimization\\Cache"],
            true,
            true,
        ),
        c("prefetch", vec!["%SystemRoot%\\Prefetch\\*.pf"], true, false),
        c("browser_cookies", browser_patterns().0.clone(), false, false),
        c("browser_history", browser_patterns().1.clone(), false, false),
        c("browser_site_data", browser_patterns().2.clone(), false, false),
        c("browser_sessions", browser_patterns().3.clone(), false, false),
        c(
            "font_cache",
            vec![
                "%SystemRoot%\\ServiceProfiles\\LocalService\\AppData\\Local\\FontCache\\*.dat",
                "%SystemRoot%\\System32\\FNTCACHE.DAT",
            ],
            true,
            false,
        ),
        c("recycle_bin", vec![], false, false),
    ]
}

/// Profile folders of Chromium-based browsers (each holds `Default`,
/// `Profile 1`, …) and Firefox-based browsers (each holds random-named
/// profiles).
const CHROMIUM: &[&str] = &[
    "%LOCALAPPDATA%\\Google\\Chrome\\User Data\\*",
    "%LOCALAPPDATA%\\Microsoft\\Edge\\User Data\\*",
    "%LOCALAPPDATA%\\BraveSoftware\\Brave-Browser\\User Data\\*",
    "%LOCALAPPDATA%\\Vivaldi\\User Data\\*",
    "%LOCALAPPDATA%\\Chromium\\User Data\\*",
    "%APPDATA%\\Opera Software\\Opera Stable",
    "%APPDATA%\\Opera Software\\Opera GX Stable",
];
const FIREFOX: &[&str] = &[
    "%APPDATA%\\Mozilla\\Firefox\\Profiles\\*",
    "%APPDATA%\\librewolf\\Profiles\\*",
    "%APPDATA%\\Waterfox\\Profiles\\*",
    "%APPDATA%\\Floorp\\Profiles\\*",
];

type Patterns = Vec<&'static str>;

/// Cookies, history, site data and sessions for every supported browser and
/// profile. Firefox history lives in the same file as bookmarks, so only
/// its form history is removed.
fn browser_patterns() -> &'static (Patterns, Patterns, Patterns, Patterns) {
    static P: std::sync::OnceLock<(Patterns, Patterns, Patterns, Patterns)> =
        std::sync::OnceLock::new();
    P.get_or_init(|| {
        let leak = |s: String| -> &'static str { Box::leak(s.into_boxed_str()) };
        let mut cookies = Vec::new();
        let mut history = Vec::new();
        let mut site = Vec::new();
        let mut sessions = Vec::new();
        for base in CHROMIUM {
            for f in [
                "Network\\Cookies",
                "Network\\Cookies-journal",
                "Cookies",
                "Cookies-journal",
            ] {
                cookies.push(leak(format!("{base}\\{f}")));
            }
            for f in [
                "History",
                "History-journal",
                "Visited Links",
                "Top Sites",
                "Top Sites-journal",
                "Shortcuts",
                "Shortcuts-journal",
            ] {
                history.push(leak(format!("{base}\\{f}")));
            }
            for f in [
                "Local Storage",
                "Session Storage",
                "IndexedDB",
                "Service Worker",
                "File System",
                "databases",
            ] {
                site.push(leak(format!("{base}\\{f}")));
            }
            for f in [
                "Sessions",
                "Current Session",
                "Current Tabs",
                "Last Session",
                "Last Tabs",
            ] {
                sessions.push(leak(format!("{base}\\{f}")));
            }
        }
        for base in FIREFOX {
            for f in ["cookies.sqlite", "cookies.sqlite-wal", "cookies.sqlite-shm"] {
                cookies.push(leak(format!("{base}\\{f}")));
            }
            for f in ["formhistory.sqlite", "formhistory.sqlite-wal"] {
                history.push(leak(format!("{base}\\{f}")));
            }
            for f in [
                "storage\\default",
                "webappsstore.sqlite",
                "webappsstore.sqlite-wal",
            ] {
                site.push(leak(format!("{base}\\{f}")));
            }
            for f in ["sessionstore.jsonlz4", "sessionstore-backups"] {
                sessions.push(leak(format!("{base}\\{f}")));
            }
        }
        (cookies, history, site, sessions)
    })
}

/// Categories shown only in Advanced mode: cleaning them makes the PC slower
/// for a while (caches are rebuilt: games stutter, thumbnails and web pages
/// load slower) or touches Windows Update's working files.
pub fn is_advanced(id: &str) -> bool {
    matches!(
        id,
        "thumbnails"
            | "shader_cache"
            | "update_cache"
            | "chrome_cache"
            | "edge_cache"
            | "firefox_cache"
            | "prefetch"
            | "font_cache"
    )
}

/// Expand env vars and `*` wildcards (one or more path segments may contain them).
pub fn resolve_pattern(pattern: &str) -> Vec<PathBuf> {
    let expanded = expand_env(pattern);
    if expanded.contains('%') {
        return Vec::new();
    }
    // Accept both separators (patterns may mix them, e.g. %TEMP%/x on Windows).
    let mut parts = expanded
        .split(['\\', '/'])
        .filter(|s| !s.is_empty())
        .peekable();
    let mut current: Vec<PathBuf> = vec![];
    let first = parts.next().unwrap_or_default();
    current.push(if expanded.starts_with('/') {
        PathBuf::from(format!("/{first}"))
    } else if expanded.starts_with('\\') {
        PathBuf::from(format!("\\{first}"))
    } else {
        // Drive letter like "C:" needs its root separator.
        PathBuf::from(format!("{first}{}", std::path::MAIN_SEPARATOR))
    });
    for part in parts {
        let mut next = Vec::new();
        for base in &current {
            if part.contains('*') || part.contains('?') {
                if let Ok(rd) = std::fs::read_dir(base) {
                    for e in rd.flatten() {
                        if glob_match(part, &e.file_name().to_string_lossy()) {
                            next.push(e.path());
                        }
                    }
                }
            } else {
                next.push(base.join(part));
            }
        }
        current = next;
    }
    current.into_iter().filter(|p| p.exists()).collect()
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ScanResult {
    pub files: u64,
    pub bytes: u64,
}

/// One file a scan found, with its size at scan time.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CleanFile {
    pub path: PathBuf,
    pub bytes: u64,
}

/// The full result of scanning one category: exactly the files a clean
/// would delete, plus the files the user's protected list kept out.
/// The deletion set is retained so the user can review it before
/// anything is removed — a clean deletes this list, it does not
/// re-scan behind the user's back.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CategoryScan {
    /// What a clean would free. Equals the totals of `files`, except
    /// for the Recycle Bin, whose contents the shell reports only as
    /// totals (its `files` stay empty: bin items can't be listed).
    pub totals: ScanResult,
    /// The files that would be deleted, sorted by path.
    pub files: Vec<CleanFile>,
    /// Files the scan found but a protected path kept out of the
    /// deletion set. Shown to the user, never silently dropped.
    pub kept: Vec<CleanFile>,
}

impl CategoryScan {
    pub fn kept_totals(&self) -> ScanResult {
        totals_of(&self.kept)
    }
}

fn totals_of(files: &[CleanFile]) -> ScanResult {
    files.iter().fold(ScanResult::default(), |mut r, f| {
        r.files += 1;
        r.bytes += f.bytes;
        r
    })
}

fn walk_files(p: &Path) -> impl Iterator<Item = crate::fswalk::DirEntry> {
    crate::fswalk::WalkDir::new(p)
        .follow_links(false)
        .into_iter()
        .flatten()
        .filter(|e| e.file_type().is_file())
}

pub fn scan(cat: &Category) -> ScanResult {
    scan_detailed(cat, &[]).totals
}

/// Scan a category, keeping every file it finds. Files matching one of
/// the `protect` patterns (the user's protected list — paths, folders,
/// or globs, the same syntax as the monitoring scope exclusions) are
/// reported in `kept` instead of entering the deletion set.
pub fn scan_detailed(cat: &Category, protect: &[String]) -> CategoryScan {
    if cat.id == "recycle_bin" {
        // The shell reports the Bin only as totals; there is no file
        // list to review or to protect individual items from.
        return CategoryScan {
            totals: recycle_bin_size(),
            files: Vec::new(),
            kept: Vec::new(),
        };
    }
    // Protection patterns live in the same canonical space as the match
    // strings built below: env-expanded, with `\` separators. Expanding
    // here first means a `/` inside an expanded value (or typed into a
    // pattern) is normalized too, not just the ones in the raw pattern.
    let protect: Vec<String> = protect
        .iter()
        .map(|p| expand_env(p).replace('/', "\\"))
        .collect();
    let excl = crate::util::Exclusions::new(&protect);
    let mut r = CategoryScan::default();
    for pat in &cat.patterns {
        for target in resolve_pattern(pat) {
            for e in walk_files(&target) {
                let f = CleanFile {
                    path: e.path().to_path_buf(),
                    bytes: e.metadata().map(|m| m.len()).unwrap_or(0),
                };
                // Protection patterns use Windows `\` separators while a
                // walked path uses the OS separator; normalize first so the
                // same patterns match on any platform.
                let for_match = f.path.to_string_lossy().replace('/', "\\");
                if excl.matches(&for_match) {
                    r.kept.push(f);
                } else {
                    r.files.push(f);
                }
            }
        }
    }
    r.files.sort_by(|a, b| a.path.cmp(&b.path));
    r.kept.sort_by(|a, b| a.path.cmp(&b.path));
    r.totals = totals_of(&r.files);
    r
}

/// Delete the category's contents. Returns what was actually freed.
pub fn clean(cat: &Category) -> ScanResult {
    if cat.id == "recycle_bin" {
        let before = recycle_bin_size();
        return if empty_recycle_bin() {
            before
        } else {
            ScanResult::default()
        };
    }
    let mut freed = ScanResult::default();
    for pat in &cat.patterns {
        for target in resolve_pattern(pat) {
            if target.is_file() {
                let len = target.metadata().map(|m| m.len()).unwrap_or(0);
                if crate::platform::delete_file(&target).is_ok() {
                    freed.files += 1;
                    freed.bytes += len;
                }
                continue;
            }
            for e in crate::fswalk::WalkDir::new(&target)
                .min_depth(1)
                .contents_first(true)
                .follow_links(false)
                .into_iter()
                .flatten()
            {
                if e.file_type().is_dir() {
                    let _ = std::fs::remove_dir(e.path());
                } else {
                    let len = e.metadata().map(|m| m.len()).unwrap_or(0);
                    if crate::platform::delete_file(e.path()).is_ok() {
                        freed.files += 1;
                        freed.bytes += len;
                    }
                }
            }
        }
    }
    freed
}

/// Delete exactly the files in `files` (a reviewed scan's deletion
/// set), then remove folders the deletions left empty, like [`clean`]
/// does. Files that can't be deleted (in use, already gone) are
/// skipped and not counted. The pattern targets themselves, and any
/// file that was never in the list — protected or simply unscanned —
/// are untouched.
pub fn clean_files(cat: &Category, files: &[CleanFile]) -> ScanResult {
    if cat.id == "recycle_bin" {
        // The Bin can't be emptied item by item through the shell API.
        return clean(cat);
    }
    let mut freed = ScanResult::default();
    for f in files {
        if crate::platform::delete_file(&f.path).is_ok() {
            freed.files += 1;
            freed.bytes += f.bytes;
        }
    }
    // Folder tidy-up, mirroring clean(): patterns' targets stay,
    // subfolders the deletions emptied go.
    for pat in &cat.patterns {
        for target in resolve_pattern(pat) {
            if !target.is_dir() {
                continue;
            }
            for e in crate::fswalk::WalkDir::new(&target)
                .min_depth(1)
                .contents_first(true)
                .follow_links(false)
                .into_iter()
                .flatten()
            {
                if e.file_type().is_dir() {
                    let _ = std::fs::remove_dir(e.path());
                }
            }
        }
    }
    freed
}

fn recycle_bin_size() -> ScanResult {
    #[cfg(windows)]
    {
        let (files, bytes) = crate::platform::windows::recycle_bin_info();
        ScanResult { files, bytes }
    }
    #[cfg(not(windows))]
    {
        ScanResult::default()
    }
}

fn empty_recycle_bin() -> bool {
    #[cfg(windows)]
    {
        crate::platform::windows::empty_recycle_bin()
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
    fn wildcard_resolution_and_clean() {
        let tmp = crate::util::tempdir().unwrap();
        for p in ["Profile 1/Cache/a", "Default/Cache", "Default/Keep"] {
            std::fs::create_dir_all(tmp.path().join(p)).unwrap();
        }
        std::fs::write(tmp.path().join("Profile 1/Cache/a/f1"), "1234").unwrap();
        std::fs::write(tmp.path().join("Default/Cache/f2"), "12").unwrap();
        std::fs::write(tmp.path().join("Default/Keep/f3"), "1").unwrap();
        std::env::set_var("INSTALLSCOPE_CLEAN_TEST", tmp.path());
        let pat: &'static str = "%INSTALLSCOPE_CLEAN_TEST%/*/Cache";
        let found = resolve_pattern(pat);
        assert_eq!(found.len(), 2, "{found:?}");
        let cat = Category {
            id: "t",
            patterns: vec![pat],
            needs_admin: false,
            default_on: true,
        };
        assert_eq!(scan(&cat), ScanResult { files: 2, bytes: 6 });
        assert_eq!(clean(&cat).bytes, 6);
        assert!(tmp.path().join("Default/Cache").exists());
        assert!(!tmp.path().join("Profile 1/Cache/a").exists());
        assert!(tmp.path().join("Default/Keep/f3").exists());
    }
}

#[cfg(test)]
mod browser_tests {
    use super::*;

    #[test]
    fn browser_items_cover_every_profile() {
        let d = std::env::temp_dir().join(format!("installscope-browsers-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        for prof in ["Default", "Profile 2"] {
            let p = d.join("Chrome").join("User Data").join(prof);
            std::fs::create_dir_all(p.join("Network")).unwrap();
            std::fs::write(p.join("Network").join("Cookies"), b"c").unwrap();
            std::fs::write(p.join("History"), b"h").unwrap();
            std::fs::write(p.join("Bookmarks"), b"keep").unwrap();
        }
        // The pattern machinery with the temp folder in place of %LOCALAPPDATA%.
        let (cookies, history, _, _) = browser_patterns();
        let resolve = |pats: &Vec<&str>| -> Vec<PathBuf> {
            pats.iter()
                .filter(|p| p.contains("Google\\Chrome"))
                .flat_map(|p| {
                    let p = p.replace("%LOCALAPPDATA%\\Google", &d.to_string_lossy());
                    resolve_pattern(&p)
                })
                .collect()
        };
        assert_eq!(resolve(cookies).len(), 2);
        assert_eq!(resolve(history).len(), 2);
        assert!(history
            .iter()
            .all(|p| !p.contains("places.sqlite") && !p.contains("Bookmarks")));
        let _ = std::fs::remove_dir_all(&d);
    }
}

#[cfg(test)]
mod review_tests {
    use super::*;

    fn test_category(var: &'static str, pattern: &'static str) -> Category {
        Category {
            id: "t",
            patterns: vec![pattern],
            needs_admin: false,
            default_on: true,
        }
    }

    #[test]
    fn detailed_scan_splits_protected_files() {
        let tmp = crate::util::tempdir().unwrap();
        let root = tmp.path();
        for p in ["Data/a", "Keep/b", "Cookies"] {
            std::fs::create_dir_all(root.join(p)).unwrap();
        }
        std::fs::write(root.join("Data/a/f1"), "1234").unwrap(); // deletable
        std::fs::write(root.join("Keep/b/f2"), "123").unwrap(); // folder protected
        std::fs::write(root.join("Cookies/f3"), "12").unwrap(); // exact file protected
        std::fs::write(root.join("Cookies/f4"), "1").unwrap(); // deletable: only f3 is named
        std::env::set_var("INSTALLSCOPE_CLEAN_PROT1", root);
        let pat: &'static str = "%INSTALLSCOPE_CLEAN_PROT1%";
        let cat = test_category("t", pat);
        let protect = vec![
            format!("%INSTALLSCOPE_CLEAN_PROT1%\\Keep"),
            format!("%INSTALLSCOPE_CLEAN_PROT1%\\Cookies\\f3"),
        ];
        let scan = scan_detailed(&cat, &protect);
        // Keep is a folder: everything beneath it is kept.
        let kept_names: Vec<_> = scan
            .kept
            .iter()
            .map(|f| f.path.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        // Kept files come back sorted by path: Cookies\f3, then Keep\b\f2.
        assert_eq!(kept_names, vec!["f3".to_string(), "f2".to_string()]);
        assert_eq!(scan.totals, ScanResult { files: 2, bytes: 5 });
        assert_eq!(scan.kept_totals(), ScanResult { files: 2, bytes: 5 });
        // The detailed scan agrees with the plain one when nothing is protected.
        assert_eq!(
            scan_detailed(&cat, &[]).totals,
            ScanResult {
                files: 4,
                bytes: 10
            }
        );
    }

    #[test]
    fn protection_patterns_support_globs_case_and_comments() {
        let tmp = crate::util::tempdir().unwrap();
        let root = tmp.path();
        for p in ["Profiles/Default/Network", "Profiles/Other/Network"] {
            std::fs::create_dir_all(root.join(p)).unwrap();
        }
        std::fs::write(root.join("Profiles/Default/Network/Cookies"), "ccc").unwrap();
        std::fs::write(root.join("Profiles/Other/Network/Cookies"), "cc").unwrap();
        std::fs::write(root.join("Profiles/Default/Network/Cache"), "c").unwrap();
        std::env::set_var("INSTALLSCOPE_CLEAN_PROT2", root);
        let cat = test_category("t", "%INSTALLSCOPE_CLEAN_PROT2%");
        let protect = vec![
            // A glob over the profile segment protects every profile's
            // Cookies file; blanks and comments are ignored.
            "".to_string(),
            "# keep sign-in state".to_string(),
            "%INSTALLSCOPE_CLEAN_PROT2%\\Profiles\\*\\Network\\COOKIES".to_string(),
        ];
        let scan = scan_detailed(&cat, &protect);
        let kept_names: Vec<_> = scan
            .kept
            .iter()
            .map(|f| f.path.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        assert_eq!(
            kept_names,
            vec!["Cookies".to_string(), "Cookies".to_string()]
        );
        assert_eq!(scan.totals, ScanResult { files: 1, bytes: 1 });
    }

    #[test]
    fn clean_files_deletes_only_the_reviewed_list() {
        let tmp = crate::util::tempdir().unwrap();
        let root = tmp.path();
        std::fs::create_dir_all(root.join("Cache/sub")).unwrap();
        std::fs::create_dir_all(root.join("Saved")).unwrap();
        std::fs::write(root.join("Cache/f1"), "1234").unwrap();
        std::fs::write(root.join("Cache/sub/f2"), "12").unwrap();
        std::fs::write(root.join("Saved/f3"), "1").unwrap();
        std::env::set_var("INSTALLSCOPE_CLEAN_PROT3", root);
        let cat = test_category("t", "%INSTALLSCOPE_CLEAN_PROT3%");
        // Protect the Saved folder; clean only what the scan lists.
        let protect = vec!["%INSTALLSCOPE_CLEAN_PROT3%\\Saved".to_string()];
        let scan = scan_detailed(&cat, &protect);
        assert_eq!(scan.files.len(), 2);
        let freed = clean_files(&cat, &scan.files);
        assert_eq!(freed, ScanResult { files: 2, bytes: 6 });
        assert!(!root.join("Cache/f1").exists());
        assert!(!root.join("Cache/sub/f2").exists());
        // The protected file survives, and so does everything the scan
        // never listed.
        assert!(root.join("Saved/f3").exists());
        // Empty subfolders are tidied (the Cache folder itself goes once
        // empty, exactly as a pattern-based clean behaves); the scan
        // target — the temp root — always stays.
        assert!(!root.join("Cache/sub").exists());
        assert!(!root.join("Cache").exists());
        assert!(root.exists());
    }

    #[test]
    fn recycle_bin_has_totals_but_no_file_list() {
        let cat = Category {
            id: "recycle_bin",
            patterns: vec![],
            needs_admin: false,
            default_on: false,
        };
        let scan = scan_detailed(&cat, &["c:\\anything".to_string()]);
        assert!(scan.files.is_empty());
        assert!(scan.kept.is_empty());
        // On non-Windows the shell reports nothing; the shape is what
        // matters here.
        assert_eq!(scan.totals, recycle_bin_size());
    }
}
