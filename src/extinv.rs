//! Browser extension inventory (ROADMAP "Next").
//!
//! Lists every installed extension, grouped by browser and profile, for the
//! Chromium family (Chrome, Edge, Brave, Vivaldi, Chromium, Opera) and the
//! Firefox family (Firefox, LibreWolf, Waterfox, Floorp), and can disable an
//! extension in a way that is fully reversible and never touches the rest of
//! the profile.
//!
//! How disable/restore works (and why it is safe):
//!
//! - Chromium extensions live in `<profile>\Extensions\<id>\<version>\`.
//!   Firefox-family add-ons live in `<profile>\extensions\<id>.xpi` (or an
//!   unpacked `<id>\` folder). The only step both browser families honor
//!   without editing their internal databases is "the files are not there":
//!   on the next start the browser treats the extension as not installed.
//! - So disabling *moves* the extension's folder/file into SetupSweep's
//!   own data folder (`<data>\extinv\disabled\…`) — nothing is ever deleted —
//!   and records the exact original path in `<data>\extinv\records.json`.
//!   Restoring moves it back to exactly that path. The profile's other data
//!   (bookmarks, history, settings) is never read for writing and never
//!   modified.
//! - Moves are refused unless the source sits inside the profile folder the
//!   inventory found it in, the destination in the profile does not already
//!   exist on restore, and the extension is not already in the requested
//!   state. A user should close the browser first: a running browser may
//!   re-create or lock files; the page says so before every action.

use crate::json;
use crate::json::ToJson;
use anyhow::{anyhow, Context, Result};
use std::path::{Path, PathBuf};

/// Which browser family an extension (or a disabled record) belongs to.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Family {
    Chromium,
    Firefox,
}

impl Family {
    fn label(self) -> &'static str {
        match self {
            Family::Chromium => "chromium",
            Family::Firefox => "firefox",
        }
    }
}

/// One extension found on this PC (enabled, browser-disabled, or held by
/// SetupSweep after a reversible disable).
#[derive(Clone, Debug)]
pub struct InstalledExtension {
    /// Display name of the browser, e.g. "Chrome".
    pub browser: String,
    pub family: Family,
    /// Display name of the profile, e.g. "Default" or "Profile 2".
    pub profile: String,
    /// The profile folder on disk.
    pub profile_dir: PathBuf,
    /// The extension's stable id.
    pub id: String,
    /// Display name. For Chromium this is the manifest `name`, with
    /// `__MSG_…__` localization references resolved from the extension's
    /// `_locales` folder where possible; when resolution is impossible the
    /// name is `"<id> (name unavailable)"` rather than the raw token.
    pub name: String,
    /// Version string, empty when it could not be determined.
    pub version: String,
    /// Whether the browser currently has it enabled. `None` = unknown
    /// (browser data missing or unreadable). Held extensions report `None`.
    pub enabled: Option<bool>,
    /// Where the extension's files live on disk: for Chromium the
    /// `Extensions\<id>` folder; for Firefox the `.xpi` file (or unpacked
    /// folder) under `extensions\`.
    pub source_path: PathBuf,
    /// `Some` when SetupSweep holds this extension (it was disabled
    /// through this module); the record is the way back.
    pub held: Option<DisabledRecord>,
}

/// The receipt for one reversible disable: exactly what moved, and from
/// where, so restoring is exact. Stored in `<data>\extinv\records.json`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DisabledRecord {
    pub id: String,
    pub browser: String,
    /// "chromium" or "firefox".
    pub family: String,
    pub profile: String,
    pub profile_dir: String,
    pub name: String,
    pub version: String,
    /// Original location of the extension's folder/file.
    pub source_path: String,
    /// Where it is held now, inside SetupSweep's data folder.
    pub held_path: String,
    /// When it was disabled (local time string, for display).
    pub moved_at: String,
}
crate::json_struct!(DisabledRecord, default, {
    id, browser, family, profile, profile_dir, name, version,
    source_path, held_path, moved_at
});

/// A browser SetupSweep knows how to locate profiles for.
struct BrowserDef {
    name: &'static str,
    family: Family,
    /// Environment variable holding the base folder ("%LOCALAPPDATA%" etc.).
    base_var: &'static str,
    /// Path of the browser folder under that base.
    rel: &'static str,
    /// Chromium layout only: `true` when `rel` itself *is* the profile
    /// folder (Opera) instead of a "User Data" folder holding profiles.
    root_is_profile: bool,
}

const BROWSERS: &[BrowserDef] = &[
    BrowserDef {
        name: "Chrome",
        family: Family::Chromium,
        base_var: "LOCALAPPDATA",
        rel: "Google\\Chrome\\User Data",
        root_is_profile: false,
    },
    BrowserDef {
        name: "Edge",
        family: Family::Chromium,
        base_var: "LOCALAPPDATA",
        rel: "Microsoft\\Edge\\User Data",
        root_is_profile: false,
    },
    BrowserDef {
        name: "Brave",
        family: Family::Chromium,
        base_var: "LOCALAPPDATA",
        rel: "BraveSoftware\\Brave-Browser\\User Data",
        root_is_profile: false,
    },
    BrowserDef {
        name: "Vivaldi",
        family: Family::Chromium,
        base_var: "LOCALAPPDATA",
        rel: "Vivaldi\\User Data",
        root_is_profile: false,
    },
    BrowserDef {
        name: "Chromium",
        family: Family::Chromium,
        base_var: "LOCALAPPDATA",
        rel: "Chromium\\User Data",
        root_is_profile: false,
    },
    BrowserDef {
        name: "Opera",
        family: Family::Chromium,
        base_var: "APPDATA",
        rel: "Opera Software\\Opera Stable",
        root_is_profile: true,
    },
    BrowserDef {
        name: "Opera GX",
        family: Family::Chromium,
        base_var: "APPDATA",
        rel: "Opera Software\\Opera GX Stable",
        root_is_profile: true,
    },
    BrowserDef {
        name: "Firefox",
        family: Family::Firefox,
        base_var: "APPDATA",
        rel: "Mozilla\\Firefox",
        root_is_profile: false,
    },
    BrowserDef {
        name: "LibreWolf",
        family: Family::Firefox,
        base_var: "APPDATA",
        rel: "librewolf",
        root_is_profile: false,
    },
    BrowserDef {
        name: "Waterfox",
        family: Family::Firefox,
        base_var: "APPDATA",
        rel: "Waterfox",
        root_is_profile: false,
    },
    BrowserDef {
        name: "Floorp",
        family: Family::Firefox,
        base_var: "APPDATA",
        rel: "Floorp",
        root_is_profile: false,
    },
];

/// Folder names under a Chromium "User Data" that are not user profiles.
const NON_PROFILE_DIRS: &[&str] = &[
    "System Profile",
    "Guest Profile",
    "GrShaderCache",
    "ShaderCache",
];

/// Inventory every extension on this PC, plus the ones SetupSweep is
/// holding. Never fails as a whole: an unreadable browser/profile just
/// contributes nothing (or the on-disk fallback), per profile.
pub fn discover(store_root: &Path) -> Vec<InstalledExtension> {
    let mut out = Vec::new();
    for def in BROWSERS {
        let Some(base) = std::env::var(def.base_var).ok().filter(|v| !v.is_empty()) else {
            continue;
        };
        let root = PathBuf::from(base).join(def.rel);
        match def.family {
            Family::Chromium => {
                if def.root_is_profile {
                    if root.is_dir() {
                        out.extend(scan_chromium_profile(def, "Default", &root));
                    }
                } else {
                    for (name, dir) in subdirs(&root) {
                        if NON_PROFILE_DIRS.contains(&name.as_str()) {
                            continue;
                        }
                        out.extend(scan_chromium_profile(def, &name, &dir));
                    }
                }
            }
            Family::Firefox => {
                let profiles_dir = root.join("Profiles");
                let names = firefox_profile_names(&root);
                for (dir_name, dir) in subdirs(&profiles_dir) {
                    let display = names
                        .iter()
                        .find(|(p, _)| p == &dir)
                        .map(|(_, n)| n.clone())
                        .unwrap_or(dir_name);
                    out.extend(scan_firefox_profile(def, &display, &dir));
                }
            }
        }
    }
    // Held extensions no longer appear in profile scans; list them from the
    // records so the user can always restore them.
    for rec in load_records(store_root) {
        let dup = out.iter().any(|e| {
            e.browser == rec.browser
                && e.profile_dir == Path::new(&rec.profile_dir)
                && e.id == rec.id
        });
        if !dup {
            out.push(InstalledExtension {
                browser: rec.browser.clone(),
                family: if rec.family == "firefox" {
                    Family::Firefox
                } else {
                    Family::Chromium
                },
                profile: rec.profile.clone(),
                profile_dir: PathBuf::from(&rec.profile_dir),
                id: rec.id.clone(),
                name: rec.name.clone(),
                version: rec.version.clone(),
                enabled: None,
                source_path: PathBuf::from(&rec.source_path),
                held: Some(rec),
            });
        }
    }
    out
}

/// Subdirectories of `dir` as (file name, path), sorted by name; empty when
/// the folder is missing or unreadable.
fn subdirs(dir: &Path) -> Vec<(String, PathBuf)> {
    let mut v: Vec<(String, PathBuf)> = match std::fs::read_dir(dir) {
        Ok(rd) => rd
            .filter_map(|e| e.ok())
            .filter(|e| e.file_type().map(|t| t.is_dir()).unwrap_or(false))
            .map(|e| (e.file_name().to_string_lossy().into_owned(), e.path()))
            .collect(),
        Err(_) => Vec::new(),
    };
    v.sort();
    v
}

/// Display names for Firefox-family profiles from `profiles.ini`
/// (`Path=` may be absolute or relative to the browser folder).
fn firefox_profile_names(root: &Path) -> Vec<(PathBuf, String)> {
    let Ok(text) = std::fs::read_to_string(root.join("profiles.ini")) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    let mut name: Option<String> = None;
    let mut path: Option<String> = None;
    let mut flush = |name: &mut Option<String>, path: &mut Option<String>| {
        if let (Some(n), Some(p)) = (name.take(), path.take()) {
            let pb = PathBuf::from(&p);
            let full = if pb.is_absolute() { pb } else { root.join(pb) };
            out.push((full, n));
        }
    };
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            flush(&mut name, &mut path);
        } else if let Some(v) = line.strip_prefix("Name=") {
            name = Some(v.trim().to_string());
        } else if let Some(v) = line.strip_prefix("Path=") {
            path = Some(v.trim().to_string());
        }
    }
    flush(&mut name, &mut path);
    out
}

// ---------------------------------------------------------------------------
// Chromium

fn scan_chromium_profile(
    def: &BrowserDef,
    profile: &str,
    profile_dir: &Path,
) -> Vec<InstalledExtension> {
    let ext_root = profile_dir.join("Extensions");
    if !ext_root.is_dir() {
        return Vec::new();
    }
    let prefs = std::fs::read(profile_dir.join("Preferences")).ok();
    let app_locale = prefs.as_deref().and_then(chromium_app_locale);
    let mut out = Vec::new();
    for (id, id_dir) in subdirs(&ext_root) {
        // An extension folder holds one subfolder per installed version;
        // the browser runs one of them — report the highest-named version
        // whose manifest parses.
        let mut versions = subdirs(&id_dir);
        versions.reverse();
        let mut found: Option<(String, String)> = None;
        for (vdir_name, vdir) in &versions {
            if let Some(bytes) = std::fs::read(vdir.join("manifest.json")).ok() {
                if let Some((name, version)) = parse_chromium_manifest(&bytes) {
                    let version = if version.is_empty() {
                        vdir_name.trim_end_matches("_0").to_string()
                    } else {
                        version
                    };
                    let name = resolve_chromium_name(vdir, &name, &id, app_locale.as_deref());
                    found = Some((name, version));
                    break;
                }
            }
        }
        let (name, version) = found.unwrap_or_else(|| (id.clone(), String::new()));
        let enabled = prefs
            .as_deref()
            .and_then(|b| chromium_enabled_state(b, &id));
        out.push(InstalledExtension {
            browser: def.name.to_string(),
            family: Family::Chromium,
            profile: profile.to_string(),
            profile_dir: profile_dir.to_path_buf(),
            id,
            name,
            version,
            enabled,
            source_path: id_dir,
            held: None,
        });
    }
    out
}

/// Name and version from a Chromium `manifest.json`. `None` when the bytes
/// are not a JSON object at all; missing fields come back empty.
pub fn parse_chromium_manifest(bytes: &[u8]) -> Option<(String, String)> {
    let v = json::parse_bytes(bytes).ok()?;
    let obj = v.as_object()?;
    let get = |k: &str| {
        obj.iter()
            .find(|(key, _)| key == k)
            .and_then(|(_, val)| val.as_str())
            .unwrap_or("")
            .to_string()
    };
    Some((get("name"), get("version")))
}

/// The extension's message key when a Chromium manifest name is a
/// `__MSG_key__` localization reference; `None` for ordinary names (and for
/// a malformed, empty-key token).
pub fn msg_key(name: &str) -> Option<&str> {
    let inner = name.strip_prefix("__MSG_")?.strip_suffix("__")?;
    if inner.is_empty() {
        None
    } else {
        Some(inner)
    }
}

/// The profile's preferred locale, cheaply: `intl.app_locale` in the
/// profile's `Preferences` file (e.g. "en_US" or "en-US"). `None` when
/// missing, empty, or unparseable.
pub fn chromium_app_locale(prefs: &[u8]) -> Option<String> {
    let v = json::parse_bytes(prefs).ok()?;
    let s = v.get("intl")?.get("app_locale")?.as_str()?.trim();
    if s.is_empty() {
        None
    } else {
        Some(s.to_string())
    }
}

/// The `message` text for `key` in one `_locales/<locale>/messages.json`.
/// `None` when the file is unreadable or malformed, the key is missing, or
/// the message is empty. Key lookup ignores case, as Chromium's does.
fn locale_message(messages_json: &Path, key: &str) -> Option<String> {
    let bytes = std::fs::read(messages_json).ok()?;
    let v = json::parse_bytes(&bytes).ok()?;
    let obj = v.as_object()?;
    let entry = obj.iter().find(|(k, _)| k.eq_ignore_ascii_case(key))?;
    let msg = entry.1.get("message")?.as_str()?;
    if msg.is_empty() {
        None
    } else {
        Some(msg.to_string())
    }
}

/// Resolve a Chromium manifest name for display. Ordinary names pass
/// through unchanged. A `__MSG_key__` reference is looked up in the
/// extension's `_locales/<locale>/messages.json`, in this deterministic
/// order: the profile's preferred locale, then `en`, then every available
/// locale folder in sorted order (each skipped when it does not resolve
/// the key). When nothing resolves, the readable placeholder
/// `"<id> (name unavailable)"` is returned — the raw `__MSG_…__` token is
/// never shown to a user.
pub fn resolve_chromium_name(
    version_dir: &Path,
    raw_name: &str,
    id: &str,
    app_locale: Option<&str>,
) -> String {
    let Some(key) = msg_key(raw_name) else {
        return raw_name.to_string();
    };
    let locales_dir = version_dir.join("_locales");
    let mut candidates: Vec<String> = Vec::new();
    if let Some(loc) = app_locale {
        let loc = loc.trim().replace('-', "_");
        if !loc.is_empty() {
            candidates.push(loc);
        }
    }
    candidates.push("en".to_string());
    if let Ok(rd) = std::fs::read_dir(&locales_dir) {
        let mut extra: Vec<String> = rd
            .filter_map(|e| e.ok())
            .filter(|e| e.file_type().map(|t| t.is_dir()).unwrap_or(false))
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        extra.sort();
        for name in extra {
            if !candidates.iter().any(|c| c.eq_ignore_ascii_case(&name)) {
                candidates.push(name);
            }
        }
    }
    for cand in &candidates {
        let messages = locales_dir.join(cand).join("messages.json");
        if let Some(text) = locale_message(&messages, key) {
            return text;
        }
    }
    format!("{id} (name unavailable)")
}

/// Whether Chromium has extension `id` enabled, from the profile's
/// `Preferences` file: `extensions.settings.<id>.state` is 1 when enabled
/// and 2 when disabled by the user. Anything else (missing file, missing
/// entry, other state) is `None` — unknown, never guessed.
pub fn chromium_enabled_state(prefs: &[u8], id: &str) -> Option<bool> {
    let v = json::parse_bytes(prefs).ok()?;
    let state = v
        .get("extensions")?
        .get("settings")?
        .get(id)?
        .get("state")?
        .as_i64()?;
    match state {
        1 => Some(true),
        2 => Some(false),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Firefox family

fn scan_firefox_profile(
    def: &BrowserDef,
    profile: &str,
    profile_dir: &Path,
) -> Vec<InstalledExtension> {
    let mut out = Vec::new();
    if let Ok(bytes) = std::fs::read(profile_dir.join("extensions.json")) {
        if let Some(entries) = parse_extensions_json(&bytes) {
            for e in entries {
                let source_path = e
                    .root_uri
                    .as_deref()
                    .and_then(firefox_path_from_root_uri)
                    .filter(|p| p.exists())
                    .unwrap_or_else(|| {
                        let xpi = profile_dir.join("extensions").join(format!("{}.xpi", e.id));
                        if xpi.exists() {
                            xpi
                        } else {
                            profile_dir.join("extensions").join(&e.id)
                        }
                    });
                out.push(InstalledExtension {
                    browser: def.name.to_string(),
                    family: Family::Firefox,
                    profile: profile.to_string(),
                    profile_dir: profile_dir.to_path_buf(),
                    id: e.id,
                    name: e.name,
                    version: e.version,
                    enabled: e.enabled,
                    source_path,
                    held: None,
                });
            }
            return out;
        }
    }
    // No (readable) extensions.json: fall back to what is on disk, with the
    // state honestly unknown. The source path is known, so these entries
    // can still be disabled and restored.
    let ext_dir = profile_dir.join("extensions");
    if let Ok(rd) = std::fs::read_dir(&ext_dir) {
        for entry in rd.filter_map(|e| e.ok()) {
            let path = entry.path();
            let id = if entry.file_type().map(|t| t.is_file()).unwrap_or(false) {
                match path.extension().and_then(|x| x.to_str()) {
                    Some("xpi") => path
                        .file_stem()
                        .map(|s| s.to_string_lossy().into_owned())
                        .unwrap_or_default(),
                    _ => continue,
                }
            } else {
                entry.file_name().to_string_lossy().into_owned()
            };
            if id.is_empty() {
                continue;
            }
            out.push(InstalledExtension {
                browser: def.name.to_string(),
                family: Family::Firefox,
                profile: profile.to_string(),
                profile_dir: profile_dir.to_path_buf(),
                name: id.clone(),
                id,
                version: String::new(),
                enabled: None,
                source_path: path,
                held: None,
            });
        }
    }
    out
}

/// One add-on parsed from a Firefox-family `extensions.json`.
#[derive(Debug, PartialEq)]
pub struct FirefoxAddon {
    pub id: String,
    pub name: String,
    pub version: String,
    /// `active && !userDisabled`; `None` when neither field is present.
    pub enabled: Option<bool>,
    pub root_uri: Option<String>,
}

/// Parse `extensions.json`. `None` when the file is not the expected shape
/// at all; individual malformed add-ons are skipped, never fatal.
pub fn parse_extensions_json(bytes: &[u8]) -> Option<Vec<FirefoxAddon>> {
    let v = json::parse_bytes(bytes).ok()?;
    let addons = v.get("addons")?.as_array()?;
    let mut out = Vec::new();
    for a in addons {
        let Some(id) = a.get("id").and_then(|x| x.as_str()) else {
            continue;
        };
        if id.is_empty() {
            continue;
        }
        let name = a
            .get("defaultLocale")
            .and_then(|l| l.get("name"))
            .and_then(|n| n.as_str())
            .unwrap_or(id)
            .to_string();
        let version = a
            .get("version")
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .to_string();
        let active = a.get("active").and_then(|x| x.as_bool());
        let user_disabled = a.get("userDisabled").and_then(|x| x.as_bool());
        let enabled = match (active, user_disabled) {
            (None, None) => None,
            _ => Some(active.unwrap_or(true) && !user_disabled.unwrap_or(false)),
        };
        out.push(FirefoxAddon {
            id: id.to_string(),
            name,
            version,
            enabled,
            root_uri: a
                .get("rootURI")
                .and_then(|x| x.as_str())
                .map(str::to_string),
        });
    }
    Some(out)
}

/// Local path inside a Firefox `rootURI` such as
/// `jar:file:///C:/Users/…/extensions/id.xpi!/` or
/// `file:///C:/Users/…/extensions/id/`. `None` for non-file URIs.
pub fn firefox_path_from_root_uri(uri: &str) -> Option<PathBuf> {
    let s = uri.strip_prefix("jar:").unwrap_or(uri);
    let s = s.strip_prefix("file://")?;
    let s = s.strip_suffix("!/").unwrap_or(s);
    // "jar:file:///C:/…" becomes "/C:/…": drop the leading slash only for
    // drive-letter paths (byte[2] == ':'), leaving Unix paths untouched.
    let s = s
        .strip_prefix('/')
        .filter(|_| s.as_bytes().get(2) == Some(&b':'))
        .map(|_| &s[1..])
        .unwrap_or(s);
    Some(PathBuf::from(percent_decode(s)))
}

fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hex = |c: u8| -> Option<u8> {
                match c {
                    b'0'..=b'9' => Some(c - b'0'),
                    b'a'..=b'f' => Some(c - b'a' + 10),
                    b'A'..=b'F' => Some(c - b'A' + 10),
                    _ => None,
                }
            };
            if let (Some(h), Some(l)) = (hex(bytes[i + 1]), hex(bytes[i + 2])) {
                out.push(h * 16 + l);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

// ---------------------------------------------------------------------------
// Disable / restore bookkeeping

/// The folder (under SetupSweep's data root) holding disabled extensions
/// and the records file.
pub fn extinv_dir(store_root: &Path) -> PathBuf {
    store_root.join("extinv")
}

pub fn records_path(store_root: &Path) -> PathBuf {
    extinv_dir(store_root).join("records.json")
}

/// All recorded disables. A missing or unreadable records file means "none"
/// (a corrupt file must not break the inventory; each record is also
/// recoverable by hand from the holding folder layout).
pub fn load_records(store_root: &Path) -> Vec<DisabledRecord> {
    let Ok(text) = std::fs::read_to_string(records_path(store_root)) else {
        return Vec::new();
    };
    json::from_json_str::<Vec<DisabledRecord>>(&text).unwrap_or_default()
}

fn save_records(store_root: &Path, records: &[DisabledRecord]) -> Result<()> {
    let dir = extinv_dir(store_root);
    std::fs::create_dir_all(&dir).with_context(|| format!("create {}", dir.display()))?;
    let text = json::to_string_pretty(&json::Value::Array(
        records.iter().map(|r| r.to_json()).collect(),
    ));
    let tmp = dir.join("records.json.tmp");
    std::fs::write(&tmp, text).with_context(|| format!("write {}", tmp.display()))?;
    std::fs::rename(&tmp, records_path(store_root))
        .with_context(|| format!("replace {}", records_path(store_root).display()))?;
    Ok(())
}

/// Disable `ext` reversibly: move its files into SetupSweep's holding
/// area and record exactly what moved. Nothing is deleted.
pub fn disable(store_root: &Path, ext: &InstalledExtension) -> Result<DisabledRecord> {
    if ext.held.is_some() {
        return Err(anyhow!("{} is already disabled", ext.name));
    }
    let src = &ext.source_path;
    if !src.exists() {
        return Err(anyhow!(
            "the files for {} were not found at {}",
            ext.name,
            src.display()
        ));
    }
    if !path_within(src, &ext.profile_dir) {
        return Err(anyhow!(
            "refusing to move {}: it is not inside the profile folder {}",
            src.display(),
            ext.profile_dir.display()
        ));
    }
    let mut records = load_records(store_root);
    if records.iter().any(|r| {
        r.browser == ext.browser
            && r.profile_dir == ext.profile_dir.to_string_lossy()
            && r.id == ext.id
    }) {
        return Err(anyhow!("{} is already disabled", ext.name));
    }
    let file_name = src
        .file_name()
        .ok_or_else(|| anyhow!("{} has no file name", src.display()))?
        .to_string_lossy()
        .into_owned();
    let held_dir = extinv_dir(store_root)
        .join("disabled")
        .join(crate::util::safe_file_name(&ext.browser))
        .join(crate::util::safe_file_name(&ext.profile));
    std::fs::create_dir_all(&held_dir).with_context(|| format!("create {}", held_dir.display()))?;
    let held_path = held_dir.join(&file_name);
    if held_path.exists() {
        return Err(anyhow!(
            "a held copy of {} already exists at {}",
            ext.name,
            held_path.display()
        ));
    }
    move_any(src, &held_path)
        .with_context(|| format!("move {} to {}", src.display(), held_path.display()))?;
    let rec = DisabledRecord {
        id: ext.id.clone(),
        browser: ext.browser.clone(),
        family: ext.family.label().to_string(),
        profile: ext.profile.clone(),
        profile_dir: ext.profile_dir.to_string_lossy().into_owned(),
        name: ext.name.clone(),
        version: ext.version.clone(),
        source_path: src.to_string_lossy().into_owned(),
        held_path: held_path.to_string_lossy().into_owned(),
        moved_at: crate::util::now_string(),
    };
    records.push(rec.clone());
    if let Err(e) = save_records(store_root, &records) {
        // Roll the move back so the extension is never stranded unrecorded.
        let _ = move_any(&held_path, src);
        return Err(e);
    }
    Ok(rec)
}

/// Restore a previously disabled extension: move its files back to the
/// exact recorded location and drop the record.
pub fn restore(store_root: &Path, record: &DisabledRecord) -> Result<()> {
    let held = Path::new(&record.held_path);
    let dest = Path::new(&record.source_path);
    if !held.exists() {
        return Err(anyhow!(
            "the held copy of {} was not found at {}",
            record.name,
            held.display()
        ));
    }
    if dest.exists() {
        return Err(anyhow!(
            "{} is already present at {}; refusing to overwrite it",
            record.name,
            dest.display()
        ));
    }
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
    }
    move_any(held, dest)
        .with_context(|| format!("move {} back to {}", held.display(), dest.display()))?;
    let mut records = load_records(store_root);
    records.retain(|r| r != record);
    save_records(store_root, &records)
}

/// `true` when `child` is `parent` itself or lives underneath it
/// (component-wise, so `C:\Prof` does not "contain" `C:\Profile`).
pub fn path_within(child: &Path, parent: &Path) -> bool {
    child == parent || child.starts_with(parent)
}

/// Move a file or folder, falling back to copy-then-remove across volumes.
/// Links are never followed by the copy fallback: a link moves by rename
/// only, so its target is never copied or deleted by mistake.
fn move_any(src: &Path, dst: &Path) -> Result<()> {
    match std::fs::rename(src, dst) {
        Ok(()) => Ok(()),
        Err(first) => {
            if crate::util::is_link(src) {
                return Err(first.into());
            }
            if src.is_dir() {
                copy_dir(src, dst)?;
                std::fs::remove_dir_all(src)?;
            } else {
                std::fs::copy(src, dst)?;
                std::fs::remove_file(src)?;
            }
            Ok(())
        }
    }
}

fn copy_dir(src: &Path, dst: &Path) -> Result<()> {
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let from = entry.path();
        let to = dst.join(entry.file_name());
        if crate::util::is_link(&from) {
            return Err(anyhow!(
                "refusing to copy the link {} across volumes",
                from.display()
            ));
        }
        if from.is_dir() {
            copy_dir(&from, &to)?;
        } else {
            std::fs::copy(&from, &to)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::util::TestDir;

    fn manifest(name: &str, version: &str) -> Vec<u8> {
        format!(r#"{{"manifest_version":3,"name":"{name}","version":"{version}"}}"#).into_bytes()
    }

    #[test]
    fn chromium_manifest_parses() {
        let (n, v) = parse_chromium_manifest(&manifest("uBlock Origin", "1.60.0")).unwrap();
        assert_eq!(n, "uBlock Origin");
        assert_eq!(v, "1.60.0");
    }

    #[test]
    fn chromium_manifest_parser_returns_raw_localized_token() {
        // The pure parser returns the manifest name as-is, including a
        // `__MSG_…__` localization token; resolving it for display happens
        // at scan time (`chromium localized name` tests below), where the
        // `_locales` folder is known.
        let (n, v) =
            parse_chromium_manifest(br#"{"name":"__MSG_appName__","version":"2.0"}"#).unwrap();
        assert_eq!(n, "__MSG_appName__");
        assert_eq!(v, "2.0");
    }

    #[test]
    fn msg_key_detection() {
        assert_eq!(msg_key("__MSG_appName__"), Some("appName"));
        assert_eq!(msg_key("Plain Name"), None);
        assert_eq!(msg_key("__MSG___"), None);
        assert_eq!(msg_key("__MSG_no_suffix"), None);
        assert_eq!(msg_key(""), None);
    }

    #[test]
    fn chromium_app_locale_from_preferences() {
        assert_eq!(
            chromium_app_locale(br#"{"intl":{"app_locale":"fr"}}"#).as_deref(),
            Some("fr")
        );
        assert_eq!(chromium_app_locale(br#"{"intl":{}}"#), None);
        assert_eq!(
            chromium_app_locale(br#"{"intl":{"app_locale":"  "}}"#),
            None
        );
        assert_eq!(chromium_app_locale(b"junk"), None);
    }

    fn def_chrome() -> BrowserDef {
        BrowserDef {
            name: "Chrome",
            family: Family::Chromium,
            base_var: "LOCALAPPDATA",
            rel: "Google\\Chrome\\User Data",
            root_is_profile: false,
        }
    }

    /// Fixture: one extension whose manifest name is `__MSG_appName__`,
    /// with `_locales/en/messages.json` present only when `en_messages` is
    /// `Some`. Returns (profile dir, extension version dir).
    fn fixture_chrome_msg(td: &TestDir, en_messages: Option<&str>) -> (PathBuf, PathBuf) {
        let profile = td.path().join("User Data").join("Default");
        let vdir = profile.join("Extensions").join("mnopqrs").join("3.0_0");
        std::fs::create_dir_all(&vdir).unwrap();
        std::fs::write(
            vdir.join("manifest.json"),
            br#"{"manifest_version":3,"name":"__MSG_appName__","version":"3.0"}"#,
        )
        .unwrap();
        if let Some(m) = en_messages {
            let en = vdir.join("_locales").join("en");
            std::fs::create_dir_all(&en).unwrap();
            std::fs::write(en.join("messages.json"), m).unwrap();
        }
        (profile, vdir)
    }

    fn write_locale(vdir: &Path, locale: &str, messages: &str) {
        let dir = vdir.join("_locales").join(locale);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("messages.json"), messages).unwrap();
    }

    fn scanned_name(profile: &Path) -> String {
        let exts = scan_chromium_profile(&def_chrome(), "Default", profile);
        assert_eq!(exts.len(), 1);
        exts[0].name.clone()
    }

    #[test]
    fn chromium_localized_name_resolves_from_messages_json() {
        let td = crate::util::tempdir().unwrap();
        let (profile, _) = fixture_chrome_msg(
            &td,
            Some(
                r#"{"appName":{"message":"My Cool Extension","description":"Shown in the toolbar"}}"#,
            ),
        );
        assert_eq!(scanned_name(&profile), "My Cool Extension");
    }

    #[test]
    fn chromium_localized_name_key_lookup_ignores_case() {
        let td = crate::util::tempdir().unwrap();
        let (profile, _) =
            fixture_chrome_msg(&td, Some(r#"{"APPNAME":{"message":"Case Insensitive"}}"#));
        assert_eq!(scanned_name(&profile), "Case Insensitive");
    }

    #[test]
    fn chromium_locale_fallback_order() {
        // Preferred locale (from Preferences) wins when it resolves.
        let td = crate::util::tempdir().unwrap();
        let (profile, vdir) =
            fixture_chrome_msg(&td, Some(r#"{"appName":{"message":"English Name"}}"#));
        write_locale(&vdir, "fr", r#"{"appName":{"message":"Nom Français"}}"#);
        std::fs::write(
            profile.join("Preferences"),
            br#"{"intl":{"app_locale":"fr"}}"#,
        )
        .unwrap();
        assert_eq!(scanned_name(&profile), "Nom Français");

        // Preferred locale absent: `en` is next (ahead of sorted folders).
        let td2 = crate::util::tempdir().unwrap();
        let (profile2, vdir2) =
            fixture_chrome_msg(&td2, Some(r#"{"appName":{"message":"English Name"}}"#));
        write_locale(&vdir2, "de", r#"{"appName":{"message":"Deutscher Name"}}"#);
        write_locale(&vdir2, "fr", r#"{"appName":{"message":"Nom Français"}}"#);
        std::fs::write(
            profile2.join("Preferences"),
            br#"{"intl":{"app_locale":"es"}}"#,
        )
        .unwrap();
        assert_eq!(scanned_name(&profile2), "English Name");

        // No `en`, no prefs: the first locale folder in sorted order is
        // used, deterministically.
        let td3 = crate::util::tempdir().unwrap();
        let (profile3, vdir3) = fixture_chrome_msg(&td3, None);
        write_locale(&vdir3, "fr", r#"{"appName":{"message":"Nom Français"}}"#);
        write_locale(&vdir3, "de", r#"{"appName":{"message":"Deutscher Name"}}"#);
        assert_eq!(scanned_name(&profile3), "Deutscher Name");
    }

    #[test]
    fn chromium_localized_name_missing_key_falls_back_to_id() {
        let td = crate::util::tempdir().unwrap();
        let (profile, _) =
            fixture_chrome_msg(&td, Some(r#"{"otherKey":{"message":"Something Else"}}"#));
        assert_eq!(scanned_name(&profile), "mnopqrs (name unavailable)");
    }

    #[test]
    fn chromium_localized_name_malformed_messages_falls_back_to_id() {
        let td = crate::util::tempdir().unwrap();
        let (profile, _) = fixture_chrome_msg(&td, Some("not json {"));
        assert_eq!(scanned_name(&profile), "mnopqrs (name unavailable)");

        // An empty message counts as unresolved too.
        let td2 = crate::util::tempdir().unwrap();
        let (profile2, _) = fixture_chrome_msg(&td2, Some(r#"{"appName":{"message":""}}"#));
        assert_eq!(scanned_name(&profile2), "mnopqrs (name unavailable)");
    }

    #[test]
    fn chromium_localized_name_without_locales_falls_back_to_id() {
        let td = crate::util::tempdir().unwrap();
        let (profile, _) = fixture_chrome_msg(&td, None);
        assert_eq!(scanned_name(&profile), "mnopqrs (name unavailable)");
    }

    #[test]
    fn chromium_manifest_garbage_is_none() {
        assert!(parse_chromium_manifest(b"not json").is_none());
        assert!(parse_chromium_manifest(b"[1,2]").is_none());
        let (n, v) = parse_chromium_manifest(b"{}").unwrap();
        assert_eq!((n.as_str(), v.as_str()), ("", ""));
    }

    #[test]
    fn chromium_state_from_preferences() {
        let prefs = br#"{"extensions":{"settings":{
            "aaa":{"state":1},
            "bbb":{"state":2},
            "ccc":{"state":7}
        }}}"#;
        assert_eq!(chromium_enabled_state(prefs, "aaa"), Some(true));
        assert_eq!(chromium_enabled_state(prefs, "bbb"), Some(false));
        assert_eq!(chromium_enabled_state(prefs, "ccc"), None);
        assert_eq!(chromium_enabled_state(prefs, "missing"), None);
        assert_eq!(chromium_enabled_state(b"junk", "aaa"), None);
    }

    #[test]
    fn extensions_json_parses() {
        let text = br#"{"addons":[
          {"id":"uBlock0@raymondhill.net","version":"1.60.0",
           "defaultLocale":{"name":"uBlock Origin"},
           "active":true,"userDisabled":false,
           "rootURI":"jar:file:///C:/P/extensions/uBlock0@raymondhill.net.xpi!/"},
          {"id":"off@example.com","version":"0.1","active":true,"userDisabled":true},
          {"id":"bare@example.com"},
          {"version":"9.9"}
        ]}"#;
        let addons = parse_extensions_json(text).unwrap();
        assert_eq!(addons.len(), 3);
        assert_eq!(addons[0].name, "uBlock Origin");
        assert_eq!(addons[0].enabled, Some(true));
        assert_eq!(addons[1].enabled, Some(false));
        assert_eq!(addons[1].name, "off@example.com");
        assert_eq!(addons[2].enabled, None);
        assert!(parse_extensions_json(b"{}").is_none());
        assert!(parse_extensions_json(b"nope").is_none());
    }

    #[test]
    fn root_uri_to_path() {
        let p = firefox_path_from_root_uri("jar:file:///C:/Users/A/AppData/extensions/x%20y.xpi!/")
            .unwrap();
        assert_eq!(p, PathBuf::from("C:/Users/A/AppData/extensions/x y.xpi"));
        assert!(firefox_path_from_root_uri("https://example.com/x").is_none());
    }

    #[test]
    fn path_within_is_componentwise() {
        // Forward slashes on purpose: `Path` splits on `/` on every
        // platform, so the component logic is exercised identically here
        // and on Windows (where real profile paths use `\`, which Windows
        // `Path` also splits on).
        assert!(path_within(
            Path::new("C:/Profile/Ext"),
            Path::new("C:/Profile")
        ));
        assert!(!path_within(
            Path::new("C:/Profile2/Ext"),
            Path::new("C:/Profile")
        ));
    }

    fn fixture_chrome(td: &TestDir) -> (PathBuf, PathBuf) {
        // <td>/User Data/Default/Extensions/<id>/1.0_0/manifest.json
        let user_data = td.path().join("User Data");
        let profile = user_data.join("Default");
        let vdir = profile.join("Extensions").join("abcdefgh").join("1.0_0");
        std::fs::create_dir_all(&vdir).unwrap();
        std::fs::write(vdir.join("manifest.json"), manifest("Test Ext", "1.0")).unwrap();
        std::fs::write(
            profile.join("Preferences"),
            br#"{"extensions":{"settings":{"abcdefgh":{"state":1}}}}"#,
        )
        .unwrap();
        (user_data, profile)
    }

    #[test]
    fn chromium_profile_scan() {
        let td = crate::util::tempdir().unwrap();
        let (_, profile) = fixture_chrome(&td);
        let def = BrowserDef {
            name: "Chrome",
            family: Family::Chromium,
            base_var: "LOCALAPPDATA",
            rel: "",
            root_is_profile: false,
        };
        let found = scan_chromium_profile(&def, "Default", &profile);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].name, "Test Ext");
        assert_eq!(found[0].version, "1.0");
        assert_eq!(found[0].enabled, Some(true));
        assert_eq!(found[0].id, "abcdefgh");
    }

    #[test]
    fn disable_restore_round_trip() {
        let td = crate::util::tempdir().unwrap();
        let (_, profile) = fixture_chrome(&td);
        let store = td.path().join("store");
        std::fs::create_dir_all(&store).unwrap();
        let ext = InstalledExtension {
            browser: "Chrome".into(),
            family: Family::Chromium,
            profile: "Default".into(),
            profile_dir: profile.clone(),
            id: "abcdefgh".into(),
            name: "Test Ext".into(),
            version: "1.0".into(),
            enabled: Some(true),
            source_path: profile.join("Extensions").join("abcdefgh"),
            held: None,
        };
        let rec = disable(&store, &ext).unwrap();
        // Files moved, nothing deleted; record is exact.
        assert!(!ext.source_path.exists());
        assert!(Path::new(&rec.held_path)
            .join("1.0_0")
            .join("manifest.json")
            .exists());
        assert_eq!(load_records(&store).len(), 1);
        // Double disable is refused.
        let mut held_view = ext.clone();
        held_view.held = Some(rec.clone());
        assert!(disable(&store, &held_view).is_err());
        // Restore puts everything back and clears the record.
        restore(&store, &rec).unwrap();
        assert!(ext.source_path.join("1.0_0").join("manifest.json").exists());
        assert!(load_records(&store).is_empty());
        // Restoring again fails cleanly (held copy gone).
        assert!(restore(&store, &rec).is_err());
    }

    #[test]
    fn disable_refuses_paths_outside_profile() {
        let td = crate::util::tempdir().unwrap();
        let store = td.path().join("store");
        std::fs::create_dir_all(&store).unwrap();
        let outside = td.path().join("elsewhere");
        std::fs::create_dir_all(&outside).unwrap();
        let ext = InstalledExtension {
            browser: "Chrome".into(),
            family: Family::Chromium,
            profile: "Default".into(),
            profile_dir: td.path().join("profile"),
            id: "x".into(),
            name: "X".into(),
            version: String::new(),
            enabled: None,
            source_path: outside,
            held: None,
        };
        assert!(disable(&store, &ext).is_err());
        assert!(ext.source_path.exists());
    }

    #[test]
    fn firefox_profile_scan_uses_extensions_json() {
        let td = crate::util::tempdir().unwrap();
        let profile = td.path().join("abc.default");
        let ext_dir = profile.join("extensions");
        std::fs::create_dir_all(&ext_dir).unwrap();
        std::fs::write(ext_dir.join("a@b.xpi"), b"fake").unwrap();
        std::fs::write(
            profile.join("extensions.json"),
            br#"{"addons":[{"id":"a@b","version":"3.2",
              "defaultLocale":{"name":"AB"},"active":true,"userDisabled":false}]}"#,
        )
        .unwrap();
        let def = BrowserDef {
            name: "Firefox",
            family: Family::Firefox,
            base_var: "APPDATA",
            rel: "",
            root_is_profile: false,
        };
        let found = scan_firefox_profile(&def, "abc.default", &profile);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].name, "AB");
        assert_eq!(found[0].version, "3.2");
        assert_eq!(found[0].enabled, Some(true));
        assert_eq!(found[0].source_path, ext_dir.join("a@b.xpi"));
    }

    #[test]
    fn firefox_profile_scan_falls_back_to_disk() {
        let td = crate::util::tempdir().unwrap();
        let profile = td.path().join("abc.default");
        let ext_dir = profile.join("extensions");
        std::fs::create_dir_all(&ext_dir).unwrap();
        std::fs::write(ext_dir.join("a@b.xpi"), b"fake").unwrap();
        std::fs::write(profile.join("extensions.json"), b"corrupt").unwrap();
        let def = BrowserDef {
            name: "Firefox",
            family: Family::Firefox,
            base_var: "APPDATA",
            rel: "",
            root_is_profile: false,
        };
        let found = scan_firefox_profile(&def, "abc.default", &profile);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].id, "a@b");
        assert_eq!(found[0].enabled, None);
    }
}
