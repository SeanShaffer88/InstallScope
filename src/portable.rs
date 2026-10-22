//! Turn a comparison of two snapshots into the settings sections of a
//! PortableApps.com Launcher `launcher.ini` (`[RegistryKeys]`,
//! `[DirectoriesMove]`, `[FilesMove]` and the matching clean-up sections).
//!
//! Typical use: snapshot, run the program and close it, snapshot again,
//! compare, and export. Everything the program left behind is mapped to the
//! Launcher's own variables (`%APPDATA%`, `%LOCALAPPDATA%`, …) so the
//! portable package moves it in and out of its Data folder.

use crate::model::{ChangeSet, FileEntry, KeyData};
use std::collections::BTreeSet;

/// Folders the Launcher has variables for, with their real paths.
#[derive(Clone, Debug, Default)]
pub struct Folders {
    /// (real path, Launcher variable), most specific first.
    pub map: Vec<(String, String)>,
}

impl Folders {
    /// The folders of the current user on this PC.
    pub fn current() -> Self {
        let env = |k: &str| std::env::var(k).unwrap_or_default();
        Self::new(
            &env("USERPROFILE"),
            &env("APPDATA"),
            &env("LOCALAPPDATA"),
            &env("ProgramData"),
            &env("TEMP"),
        )
    }

    pub fn new(profile: &str, appdata: &str, local: &str, programdata: &str, temp: &str) -> Self {
        let mut map = Vec::new();
        let mut add = |path: String, var: &str| {
            let path = path.trim_end_matches('\\').to_string();
            if !path.is_empty() {
                map.push((path, var.to_string()));
            }
        };
        add(temp.to_string(), "%TEMP%");
        add(local.to_string(), "%LOCALAPPDATA%");
        add(appdata.to_string(), "%APPDATA%");
        if !profile.is_empty() {
            add(
                format!("{profile}\\AppData\\LocalLow"),
                "%USERPROFILE%\\AppData\\LocalLow",
            );
            add(format!("{profile}\\Documents"), "%DOCUMENTS%");
        }
        add(programdata.to_string(), "%ALLUSERSAPPDATA%");
        add(profile.to_string(), "%USERPROFILE%");
        // Longest path first, so %LOCALAPPDATA% wins over %USERPROFILE%.
        map.sort_by_key(|(p, _)| std::cmp::Reverse(p.len()));
        Folders { map }
    }

    /// `C:\Users\Sean\AppData\Roaming\Foo` → `%APPDATA%\Foo`.
    pub fn to_var(&self, path: &str) -> Option<String> {
        let lower = path.to_lowercase();
        for (real, var) in &self.map {
            let r = real.to_lowercase();
            if lower == r {
                return Some(var.clone());
            }
            if lower.starts_with(&r) && lower.as_bytes().get(r.len()) == Some(&b'\\') {
                return Some(format!("{var}{}", &path[real.len()..]));
            }
        }
        None
    }
}

/// Parent of a `\`-separated path.
fn parent(p: &str) -> Option<&str> {
    p.rfind('\\').map(|i| &p[..i])
}

fn leaf(p: &str) -> &str {
    p.rsplit('\\').next().unwrap_or(p)
}

/// Whether `p` lies inside `dir` (not equal).
fn inside(p: &str, dir: &str) -> bool {
    p.len() > dir.len()
        && p.as_bytes()[dir.len()] == b'\\'
        && p[..dir.len()].eq_ignore_ascii_case(dir)
}

/// Collapse each new tree to the folder (or key) that holds the data: a new
/// `Publisher` folder holding nothing but a new `AppName` folder becomes
/// `Publisher\AppName`, with `Publisher` cleaned up when it is left empty.
///
/// `items` are all the new paths; `has_content(p)` says whether `p` itself
/// holds data (files in a folder, values in a key). Returns (move, clean-up
/// if empty) lists.
fn collapse(items: &[String], has_content: impl Fn(&str) -> bool) -> (Vec<String>, Vec<String>) {
    let set: BTreeSet<String> = items.iter().map(|s| s.to_lowercase()).collect();
    let is_new = |p: &str| set.contains(&p.to_lowercase());
    let mut moves = Vec::new();
    let mut cleanup = Vec::new();
    for top in items {
        // Only the top of each new tree.
        if parent(top).is_some_and(is_new) {
            continue;
        }
        let mut cur = top.clone();
        loop {
            let children: Vec<&String> = items
                .iter()
                .filter(|c| parent(c).is_some_and(|p| p.eq_ignore_ascii_case(&cur)))
                .collect();
            if children.len() == 1 && !has_content(&cur) {
                cleanup.push(cur.clone());
                cur = children[0].clone();
            } else {
                break;
            }
        }
        moves.push(cur);
    }
    // Innermost first, so nested empty folders are removed bottom-up.
    cleanup.reverse();
    (moves, cleanup)
}

/// A safe INI key for a folder or key name: letters, digits, `_`, `-`, `.`.
fn ini_name(name: &str, used: &mut BTreeSet<String>) -> String {
    let mut base: String = name
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || "_-.".contains(c) {
                c
            } else {
                '_'
            }
        })
        .collect();
    // Data\settings holds the registry files, so DirectoriesMove must not use it.
    if base.is_empty() || base.eq_ignore_ascii_case("settings") {
        base = format!("{base}data");
    }
    let mut n = base.clone();
    let mut i = 2;
    while !used.insert(n.to_lowercase()) {
        n = format!("{base}{i}");
        i += 1;
    }
    n
}

/// Registry paths written as HKCU/HKLM, with 32-bit redirection removed
/// (the Launcher and most portable programs are 32-bit and see it the same way).
fn short_key(k: &str) -> (String, bool) {
    let mut k = k
        .replace("HKEY_CURRENT_USER\\", "HKCU\\")
        .replace("HKEY_LOCAL_MACHINE\\", "HKLM\\");
    let mut redirected = false;
    for wow in [
        "\\SOFTWARE\\WOW6432Node\\",
        "\\Software\\WOW6432Node\\",
        "\\SOFTWARE\\Wow6432Node\\",
    ] {
        if k.contains(wow) {
            k = k.replacen(wow, "\\Software\\", 1);
            redirected = true;
        }
    }
    (k, redirected)
}

/// The result: the INI sections, counts for the summary line, and what is
/// moved (so a package builder can copy the captured settings as defaults).
#[derive(Debug, Default, Clone)]
pub struct LauncherIni {
    /// Only the settings sections (no header, no [Launch]).
    pub sections: String,
    /// Commented review notes.
    pub notes: String,
    /// Header + sections + notes, ready to paste.
    pub text: String,
    pub directories: usize,
    pub files: usize,
    pub keys: usize,
    /// [DirectoriesMove] key and the real folder it captures.
    pub dir_moves: Vec<(String, String)>,
    /// Real paths of the files in [FilesMove].
    pub file_moves: Vec<String>,
    /// [RegistryKeys] file name and the registry key as captured.
    pub reg_moves: Vec<(String, String)>,
}

/// Places installers write that never belong in a portable app's settings:
/// shortcuts, the shell's own caches and the installer bookkeeping.
const SKIP_VARS: &[&str] = &[
    "%APPDATA%\\Microsoft\\Windows\\Start Menu",
    "%APPDATA%\\Microsoft\\Windows\\Recent",
    "%APPDATA%\\Microsoft\\Windows\\SendTo",
    "%APPDATA%\\Microsoft\\Internet Explorer\\Quick Launch",
    "%ALLUSERSAPPDATA%\\Microsoft\\Windows\\Start Menu",
    "%ALLUSERSAPPDATA%\\Package Cache",
    "%LOCALAPPDATA%\\Microsoft\\Windows",
    "%LOCALAPPDATA%\\Package Cache",
    "%USERPROFILE%\\Desktop",
    "%USERPROFILE%\\ntuser",
];
const SKIP_KEYS: &[&str] = &[
    "\\currentversion\\uninstall\\",
    "\\currentversion\\app paths\\",
    "\\currentversion\\installer\\",
    "\\classes\\installer\\",
    "\\microsoft\\installer\\",
    "\\currentversion\\explorer\\",
];

fn skip_key(k: &str) -> bool {
    let l = format!("{}\\", k.to_lowercase());
    SKIP_KEYS.iter().any(|s| l.contains(s))
}

/// Build the settings sections for a program whose own files are in
/// `program_dirs` (those are left out: they go in App\\AppName instead).
pub fn launcher_ini(cs: &ChangeSet, title: &str, folders: &Folders) -> LauncherIni {
    launcher_ini_for(cs, title, folders, &[])
}

pub fn launcher_ini_for(
    cs: &ChangeSet,
    title: &str,
    folders: &Folders,
    program_dirs: &[String],
) -> LauncherIni {
    let skip_path = |p: &str| {
        program_dirs
            .iter()
            .any(|d| p.eq_ignore_ascii_case(d) || inside(p, d))
            || folders.to_var(p).is_some_and(|v| {
                let v = v.to_lowercase();
                SKIP_VARS.iter().any(|s| v.starts_with(&s.to_lowercase()))
            })
    };
    let files_added: Vec<FileEntry> = cs
        .files_added
        .iter()
        .filter(|f| !skip_path(&f.path))
        .cloned()
        .collect();
    let files_modified: Vec<_> = cs
        .files_modified
        .iter()
        .filter(|f| !skip_path(&f.after.path))
        .cloned()
        .collect();
    let keys_added: Vec<KeyData> = cs
        .keys_added
        .iter()
        .filter(|k| !skip_key(&k.path))
        .cloned()
        .collect();
    let filtered = ChangeSet {
        files_added,
        files_modified,
        keys_added,
        values_added: cs
            .values_added
            .iter()
            .filter(|v| !skip_key(&v.key))
            .cloned()
            .collect(),
        values_modified: cs
            .values_modified
            .iter()
            .filter(|v| !skip_key(&v.key))
            .cloned()
            .collect(),
        ..Default::default()
    };
    let cs = &filtered;
    let mut out = LauncherIni::default();
    let mut notes: Vec<String> = Vec::new();
    let mut used = BTreeSet::new();

    // ---- Files and folders --------------------------------------------
    let added: Vec<&FileEntry> = cs.files_added.iter().collect();
    let new_dirs: Vec<String> = added
        .iter()
        .filter(|f| f.is_dir)
        .map(|f| f.path.clone())
        .collect();
    let new_files: Vec<&FileEntry> = added.iter().copied().filter(|f| !f.is_dir).collect();
    let dir_has_files = |d: &str| {
        new_files
            .iter()
            .any(|f| parent(&f.path).is_some_and(|p| p.eq_ignore_ascii_case(d)))
            || cs
                .files_modified
                .iter()
                .any(|f| parent(&f.after.path).is_some_and(|p| p.eq_ignore_ascii_case(d)))
    };
    let (dir_moves, dir_cleanup) = collapse(&new_dirs, dir_has_files);

    let mut dirs_move: Vec<String> = Vec::new();
    let mut dirs_cleanup: Vec<String> = Vec::new();
    let mut dirs_temp: Vec<String> = Vec::new();
    let mut outside: Vec<String> = Vec::new();
    for d in &dir_moves {
        match folders.to_var(d) {
            Some(v) if v.starts_with("%TEMP%") => dirs_temp.push(v),
            Some(v) => {
                let key = ini_name(leaf(d), &mut used);
                dirs_move.push(format!("{key}={v}"));
                out.dir_moves.push((key, d.clone()));
                out.directories += 1;
            }
            None => outside.push(d.clone()),
        }
    }
    for d in &dir_cleanup {
        if let Some(v) = folders.to_var(d) {
            if !v.starts_with("%TEMP%") {
                dirs_cleanup.push(v);
            }
        }
    }
    // Single files written into folders that already existed.
    let in_new_dir = |p: &str| new_dirs.iter().any(|d| inside(p, d));
    let mut files_move: Vec<String> = Vec::new();
    for f in &new_files {
        if in_new_dir(&f.path) {
            continue;
        }
        match (
            folders.to_var(&f.path),
            parent(&f.path).and_then(|p| folders.to_var(p)),
        ) {
            (Some(v), _) if v.starts_with("%TEMP%") => {}
            (Some(_), Some(dir)) => {
                files_move.push(format!("settings\\{}={dir}", leaf(&f.path)));
                out.file_moves.push(f.path.clone());
                out.files += 1;
            }
            _ => outside.push(f.path.clone()),
        }
    }
    let mut files_changed: Vec<String> = Vec::new();
    for f in &cs.files_modified {
        let p = &f.after.path;
        if in_new_dir(p) {
            continue;
        }
        if let (Some(v), Some(dir)) = (folders.to_var(p), parent(p).and_then(|x| folders.to_var(x)))
        {
            if !v.starts_with("%TEMP%") {
                files_changed.push(format!("settings\\{}={dir}", leaf(p)));
            }
        }
    }

    // ---- Registry -----------------------------------------------------
    let key_paths: Vec<String> = cs.keys_added.iter().map(|k| k.path.clone()).collect();
    let key_has_values = |p: &str| {
        cs.keys_added
            .iter()
            .find(|k: &&KeyData| k.path.eq_ignore_ascii_case(p))
            .is_some_and(|k| !k.values.is_empty())
    };
    let (key_moves, key_cleanup) = collapse(&key_paths, key_has_values);
    // Registry files live in Data\settings, apart from the moved folders.
    let mut used = BTreeSet::new();
    let mut reg_keys: Vec<String> = Vec::new();
    let mut reg_keys_machine: Vec<String> = Vec::new();
    let mut reg_cleanup: Vec<String> = Vec::new();
    let mut any_redirected = false;
    for real in &key_moves {
        let (k, redirected) = short_key(real);
        any_redirected |= redirected;
        let name = ini_name(leaf(&k), &mut used);
        if k.starts_with("HKCU\\") {
            reg_keys.push(format!("{name}={k}"));
            out.reg_moves.push((name, real.clone()));
            out.keys += 1;
        } else {
            reg_keys_machine.push(format!("{name}={k}"));
        }
    }
    for k in &key_cleanup {
        let (k, _) = short_key(k);
        if k.starts_with("HKCU\\") {
            reg_cleanup.push(k);
        }
    }
    let in_new_key = |k: &str| {
        key_paths
            .iter()
            .any(|n| n.eq_ignore_ascii_case(k) || inside(k, n))
    };
    let mut values_written: Vec<String> = Vec::new();
    for v in &cs.values_added {
        if !in_new_key(&v.key) {
            let (k, _) = short_key(&v.key);
            values_written.push(format!("{k}\\{}", v.value.name));
        }
    }
    for v in &cs.values_modified {
        let (k, _) = short_key(&v.key);
        values_written.push(format!("{k}\\{}", v.after.name));
    }

    // ---- Write the INI ------------------------------------------------
    let header = format!(
        "; PortableApps.com Launcher settings for {title}\n\
         ; Made by SetupScope from a snapshot comparison. Paste these sections into\n\
         ; App\\AppInfo\\Launcher\\<AppName>Portable.ini and review every line.\n\
         ; Lines starting with ';' are suggestions that need a decision.\n\n"
    );
    let mut s = String::new();
    if !reg_keys.is_empty() {
        s.push_str("[Activate]\nRegistry=true\n\n");
    }
    let section =
        |s: &mut String, name: &str, lines: &[String], numbered: bool, commented: bool| {
            if lines.is_empty() {
                return;
            }
            s.push_str(&format!("[{name}]\n"));
            for (i, l) in lines.iter().enumerate() {
                let line = if numbered {
                    format!("{}={l}", i + 1)
                } else {
                    l.clone()
                };
                if commented {
                    s.push(';');
                }
                s.push_str(&line);
                s.push('\n');
            }
            s.push('\n');
        };
    section(&mut s, "RegistryKeys", &reg_keys, false, false);
    section(&mut s, "RegistryCleanupIfEmpty", &reg_cleanup, true, false);
    section(&mut s, "DirectoriesMove", &dirs_move, false, false);
    section(
        &mut s,
        "DirectoriesCleanupIfEmpty",
        &dirs_cleanup,
        true,
        false,
    );
    section(&mut s, "FilesMove", &files_move, false, false);
    section(&mut s, "DirectoriesCleanupForce", &dirs_temp, true, false);

    if !reg_keys_machine.is_empty() {
        notes.push("Machine-wide registry keys (HKLM). Saving these needs administrator rights; most portable apps can skip them or write them with [RegistryValueWrite]:".into());
        notes.extend(reg_keys_machine.iter().map(|l| format!("  {l}")));
    }
    if any_redirected {
        notes.push("WOW6432Node was removed from 32-bit keys; use the ...DisableRedirect sections if the program is 64-bit.".into());
    }
    if !files_changed.is_empty() {
        notes.push("Files that already existed and were changed (shared files; move only if they belong to this program):".into());
        notes.extend(files_changed.iter().map(|l| format!("  [FilesMove] {l}")));
    }
    if !values_written.is_empty() {
        notes.push("Values written into keys that already existed (consider [RegistryValueBackupDelete] or [RegistryValueWrite]):".into());
        let n = values_written.len();
        notes.extend(values_written.iter().take(50).map(|l| format!("  {l}")));
        if n > 50 {
            notes.push(format!("  … and {} more", n - 50));
        }
    }
    if !outside.is_empty() {
        notes.push("Written outside the user's folders (usually the program's own files, which belong in App\\):".into());
        let n = outside.len();
        notes.extend(outside.iter().take(30).map(|l| format!("  {l}")));
        if n > 30 {
            notes.push(format!("  … and {} more", n - 30));
        }
    }
    let mut review = String::new();
    if !notes.is_empty() {
        review.push_str("; ---- Review ----\n");
        for n in notes {
            review.push_str(&format!("; {n}\n"));
        }
    }
    out.text = format!("{header}{s}{review}");
    out.sections = s;
    out.notes = review;
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{RegValue, ValueRef};

    fn dir(p: &str) -> FileEntry {
        FileEntry {
            path: p.into(),
            is_dir: true,
            size: 0,
            mtime: 0,
        }
    }
    fn file(p: &str) -> FileEntry {
        FileEntry {
            path: p.into(),
            is_dir: false,
            size: 1,
            mtime: 0,
        }
    }
    fn key(p: &str, values: bool) -> KeyData {
        let k = KeyData::new(p);
        if values {
            k.with(RegValue::string("Theme", "dark"))
        } else {
            k
        }
    }
    fn folders() -> Folders {
        Folders::new(
            "C:\\Users\\Sean",
            "C:\\Users\\Sean\\AppData\\Roaming",
            "C:\\Users\\Sean\\AppData\\Local",
            "C:\\ProgramData",
            "C:\\Users\\Sean\\AppData\\Local\\Temp",
        )
    }

    #[test]
    fn maps_folders_to_launcher_variables() {
        let f = folders();
        assert_eq!(
            f.to_var("C:\\Users\\Sean\\AppData\\Roaming\\Foo").unwrap(),
            "%APPDATA%\\Foo"
        );
        assert_eq!(
            f.to_var("c:\\users\\sean\\appdata\\local\\Foo\\x").unwrap(),
            "%LOCALAPPDATA%\\Foo\\x"
        );
        assert_eq!(
            f.to_var("C:\\Users\\Sean\\AppData\\LocalLow\\Foo").unwrap(),
            "%USERPROFILE%\\AppData\\LocalLow\\Foo"
        );
        assert_eq!(
            f.to_var("C:\\ProgramData\\Foo").unwrap(),
            "%ALLUSERSAPPDATA%\\Foo"
        );
        assert_eq!(
            f.to_var("C:\\Users\\Sean\\.foorc").unwrap(),
            "%USERPROFILE%\\.foorc"
        );
        assert!(f.to_var("C:\\Program Files\\Foo").is_none());
        assert!(f.to_var("C:\\Users\\Seanx\\Foo").is_none());
    }

    #[test]
    fn builds_launcher_sections() {
        let cs = ChangeSet {
            files_added: vec![
                dir("C:\\Users\\Sean\\AppData\\Roaming\\Pub"),
                dir("C:\\Users\\Sean\\AppData\\Roaming\\Pub\\App"),
                file("C:\\Users\\Sean\\AppData\\Roaming\\Pub\\App\\config.xml"),
                dir("C:\\Users\\Sean\\AppData\\Local\\App"),
                dir("C:\\Users\\Sean\\AppData\\Local\\App\\Cache"),
                file("C:\\Users\\Sean\\AppData\\Local\\App\\state.json"),
                file("C:\\Users\\Sean\\app.ini"),
                dir("C:\\Users\\Sean\\AppData\\Local\\Temp\\app-123"),
                file("C:\\Program Files\\App\\app.exe"),
            ],
            keys_added: vec![
                key("HKCU\\Software\\Pub", false),
                key("HKCU\\Software\\Pub\\App", true),
                key("HKCU\\Software\\Pub\\App\\Recent", true),
                key("HKLM\\SOFTWARE\\WOW6432Node\\App", true),
            ],
            values_added: vec![ValueRef {
                key: "HKCU\\Software\\Microsoft\\Windows\\CurrentVersion\\Run".into(),
                value: RegValue::string("App", "x"),
            }],
            ..Default::default()
        };
        let ini = launcher_ini(&cs, "App", &folders()).text;
        assert!(ini.contains("[Activate]\nRegistry=true"), "{ini}");
        assert!(
            ini.contains("[RegistryKeys]\nApp=HKCU\\Software\\Pub\\App\n"),
            "{ini}"
        );
        assert!(
            ini.contains("[RegistryCleanupIfEmpty]\n1=HKCU\\Software\\Pub\n"),
            "{ini}"
        );
        assert!(ini.contains("[DirectoriesMove]\n"), "{ini}");
        assert!(ini.contains("=%APPDATA%\\Pub\\App\n"), "{ini}");
        assert!(ini.contains("=%LOCALAPPDATA%\\App\n"), "{ini}");
        assert!(
            ini.contains("[DirectoriesCleanupIfEmpty]\n1=%APPDATA%\\Pub\n"),
            "{ini}"
        );
        assert!(
            ini.contains("[FilesMove]\nsettings\\app.ini=%USERPROFILE%\n"),
            "{ini}"
        );
        assert!(
            ini.contains("[DirectoriesCleanupForce]\n1=%TEMP%\\app-123\n"),
            "{ini}"
        );
        assert!(ini.contains(";   App2=HKLM\\Software\\App"), "{ini}");
        assert!(ini.contains("CurrentVersion\\Run\\App"), "{ini}");
        assert!(ini.contains("C:\\Program Files\\App\\app.exe"), "{ini}");
        // The two folders named App get distinct keys, and none is "settings".
        let dm: Vec<&str> = ini
            .split("[DirectoriesMove]\n")
            .nth(1)
            .unwrap()
            .lines()
            .take_while(|l| !l.is_empty())
            .collect();
        assert_eq!(dm.len(), 2);
        assert_ne!(dm[0].split('=').next(), dm[1].split('=').next());
    }

    #[test]
    fn leaves_out_program_files_shortcuts_and_installer_keys() {
        let cs = ChangeSet {
            files_added: vec![
                dir("C:\\Users\\Sean\\AppData\\Local\\Programs\\App"),
                file("C:\\Users\\Sean\\AppData\\Local\\Programs\\App\\app.exe"),
                dir("C:\\Users\\Sean\\AppData\\Roaming\\Microsoft\\Windows\\Start Menu\\Programs\\App"),
                file("C:\\Users\\Sean\\Desktop\\App.lnk"),
                file("C:\\Users\\Sean\\NTUSER.DAT.LOG1"),
                dir("C:\\Users\\Sean\\AppData\\Roaming\\App"),
            ],
            keys_added: vec![
                key("HKCU\\Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\App", true),
                key("HKCU\\Software\\App", true),
            ],
            ..Default::default()
        };
        let r = launcher_ini_for(
            &cs,
            "App",
            &folders(),
            &["C:\\Users\\Sean\\AppData\\Local\\Programs\\App".to_string()],
        );
        assert_eq!(
            r.dir_moves,
            vec![(
                "App".to_string(),
                "C:\\Users\\Sean\\AppData\\Roaming\\App".to_string()
            )]
        );
        assert_eq!(r.reg_moves.len(), 1);
        assert!(!r.text.contains("Programs\\App"), "{}", r.text);
        assert!(!r.text.contains("Desktop"), "{}", r.text);
        assert!(!r.text.contains("Uninstall"), "{}", r.text);
        assert!(!r.text.to_lowercase().contains("ntuser"), "{}", r.text);
    }

    #[test]
    fn never_uses_settings_as_a_directory_key() {
        let mut used = BTreeSet::new();
        assert_eq!(ini_name("settings", &mut used), "settingsdata");
        assert_eq!(ini_name("My App", &mut used), "My_App");
        assert_eq!(ini_name("My App", &mut used), "My_App2");
    }
}
