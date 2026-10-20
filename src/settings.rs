//! User settings, persisted as JSON in the data folder.

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileRoot {
    pub path: String,
    pub recursive: bool,
}
crate::json_struct!(FileRoot, { path, recursive });

impl FileRoot {
    fn new(path: &str, recursive: bool) -> Self {
        FileRoot {
            path: path.into(),
            recursive,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Settings {
    /// Interface language code ("" = follow Windows).
    pub language: String,
    /// UI zoom factor.
    pub zoom: f32,
    /// Registry keys captured by snapshots (whole subtrees).
    pub registry_roots: Vec<String>,
    /// Folders captured by snapshots. `%VAR%` references are expanded.
    pub file_roots: Vec<FileRoot>,
    /// Glob patterns for registry noise to ignore.
    pub registry_exclusions: Vec<String>,
    /// Glob patterns for file noise to ignore.
    pub file_exclusions: Vec<String>,
    /// Also apply InstallScope's built-in list of volatile locations (temporary
    /// files, logs, browser and app caches).
    pub auto_exclusions: bool,
    /// Copy everything to a backup before deleting it.
    pub backup_before_removal: bool,
    /// Create a System Restore point before uninstalling.
    pub create_restore_point: bool,
    /// When removing a logged program folder, also remove files created in it
    /// after installation (settings, caches). Off = keep them.
    pub remove_unlogged_files: bool,
    /// Ask before destructive actions.
    pub confirm_actions: bool,
    /// Also send removed files to the Recycle Bin (in addition to the backup).
    pub use_recycle_bin: bool,
    /// Close Windows Explorer during a removal when it has the program's
    /// shell extensions loaded, so they are deleted at once rather than at
    /// the next restart. It starts again when the removal is done.
    pub restart_explorer: bool,
    /// Close running copies of a program before removing it.
    pub close_programs: bool,
    /// Installer Decrapifier: an OpenAI-compatible chat endpoint on this
    /// computer for components the rules can't judge ("" = rules only).
    pub local_model_url: String,
    /// The model to ask at that endpoint.
    pub local_model_name: String,
    /// Show entries flagged as SystemComponent in the program list.
    pub show_system_components: bool,
    /// Show Microsoft Store apps in the program list.
    pub show_store_apps: bool,
    /// Advanced mode shows technical details and developer tools.
    pub advanced_mode: bool,
    /// Brighter text and stronger outlines.
    pub high_contrast: bool,
    /// The welcome screen has been completed.
    pub first_run_done: bool,
    /// PortableApps Forge: the PortableApps folder holding the
    /// PortableApps.com Launcher and Installer ("" = search every drive).
    pub portableapps_dir: String,
    /// PortableApps Forge: your name, added to the package's publisher.
    pub packager_name: String,
    /// PortableApps Forge: where packages are saved.
    pub portable_out_dir: String,
    /// Saved monitoring scopes, e.g. "Quick (Program Files only)" or "Deep".
    pub profiles: Vec<ScanProfile>,
    /// The profile the current scope came from ("" = none).
    pub active_profile: String,
}
crate::json_struct!(Settings, default, { language, zoom, registry_roots, file_roots, registry_exclusions, file_exclusions, auto_exclusions, backup_before_removal, create_restore_point, remove_unlogged_files, confirm_actions, use_recycle_bin, restart_explorer, close_programs, local_model_url, local_model_name, show_system_components, show_store_apps, advanced_mode, high_contrast, first_run_done, portableapps_dir, packager_name, portable_out_dir, profiles, active_profile });

/// A named monitoring scope: what snapshots capture and ignore.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct ScanProfile {
    pub name: String,
    pub registry_roots: Vec<String>,
    pub file_roots: Vec<FileRoot>,
    pub registry_exclusions: Vec<String>,
    pub file_exclusions: Vec<String>,
}
crate::json_struct!(ScanProfile, default, { name, registry_roots, file_roots, registry_exclusions, file_exclusions });

/// In Simple mode the safety switches are locked to their safe values,
/// whatever was chosen in Advanced mode.
impl Settings {
    /// Save the current scope as a profile (replacing one with that name).
    pub fn save_profile(&mut self, name: &str) {
        let name = name.trim();
        if name.is_empty() {
            return;
        }
        let p = ScanProfile {
            name: name.to_string(),
            registry_roots: self.registry_roots.clone(),
            file_roots: self.file_roots.clone(),
            registry_exclusions: self.registry_exclusions.clone(),
            file_exclusions: self.file_exclusions.clone(),
        };
        match self
            .profiles
            .iter_mut()
            .find(|x| x.name.eq_ignore_ascii_case(name))
        {
            Some(x) => *x = p,
            None => self.profiles.push(p),
        }
        self.active_profile = name.to_string();
    }

    /// Make a saved profile the current scope. False if there is none by that name.
    pub fn use_profile(&mut self, name: &str) -> bool {
        let Some(p) = self
            .profiles
            .iter()
            .find(|x| x.name.eq_ignore_ascii_case(name.trim()))
            .cloned()
        else {
            return false;
        };
        self.registry_roots = p.registry_roots;
        self.file_roots = p.file_roots;
        self.registry_exclusions = p.registry_exclusions;
        self.file_exclusions = p.file_exclusions;
        self.active_profile = p.name;
        true
    }

    pub fn delete_profile(&mut self, name: &str) {
        self.profiles.retain(|x| !x.name.eq_ignore_ascii_case(name));
        if self.active_profile.eq_ignore_ascii_case(name) {
            self.active_profile.clear();
        }
    }

    /// Registry exclusions in effect: the user's list plus the built-in one.
    pub fn effective_registry_exclusions(&self) -> Vec<String> {
        let mut v = self.registry_exclusions.clone();
        if self.auto_exclusions {
            v.extend(crate::noise::builtin_registry_exclusions());
        }
        v
    }

    /// File exclusions in effect: the user's list plus the built-in one.
    pub fn effective_file_exclusions(&self) -> Vec<String> {
        let mut v = self.file_exclusions.clone();
        if self.auto_exclusions {
            v.extend(crate::noise::builtin_file_exclusions());
        }
        v
    }

    pub fn backup_enabled(&self) -> bool {
        self.backup_before_removal || !self.advanced_mode
    }
    pub fn confirm_enabled(&self) -> bool {
        self.confirm_actions || !self.advanced_mode
    }
    /// Whether an action must show its confirmation dialog. Actions flagged
    /// `always_confirm` ask even when the user turned confirmations off in
    /// Advanced mode (they are hard or impossible to undo, so the
    /// confirmation setting must not silence them); every other action
    /// follows `confirm_enabled()`.
    pub fn must_confirm(&self, always_confirm: bool) -> bool {
        always_confirm || self.confirm_enabled()
    }
    pub fn remove_unlogged_enabled(&self) -> bool {
        self.remove_unlogged_files && self.advanced_mode
    }
    pub fn system_components_shown(&self) -> bool {
        self.show_system_components && self.advanced_mode
    }
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            language: String::new(),
            zoom: 1.0,
            registry_roots: default_registry_roots(),
            file_roots: default_file_roots(),
            registry_exclusions: default_registry_exclusions(),
            file_exclusions: default_file_exclusions(),
            auto_exclusions: true,
            backup_before_removal: true,
            create_restore_point: true,
            remove_unlogged_files: false,
            confirm_actions: true,
            use_recycle_bin: false,
            restart_explorer: true,
            close_programs: true,
            local_model_url: String::new(),
            local_model_name: String::new(),
            show_system_components: false,
            show_store_apps: false,
            advanced_mode: false,
            high_contrast: false,
            first_run_done: false,
            portableapps_dir: String::new(),
            packager_name: String::new(),
            portable_out_dir: String::new(),
            profiles: Vec::new(),
            active_profile: String::new(),
        }
    }
}

pub fn default_registry_roots() -> Vec<String> {
    [
        "HKLM\\SOFTWARE",
        "HKLM\\SYSTEM\\CurrentControlSet\\Services",
        "HKLM\\SYSTEM\\CurrentControlSet\\Control\\Session Manager\\Environment",
        "HKCU\\Software",
        "HKCU\\Environment",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect()
}

pub fn default_file_roots() -> Vec<FileRoot> {
    vec![
        FileRoot::new("%ProgramFiles%", true),
        FileRoot::new("%ProgramFiles(x86)%", true),
        FileRoot::new("%ProgramData%", true),
        FileRoot::new("%APPDATA%", true),
        FileRoot::new("%LOCALAPPDATA%", true),
        FileRoot::new("%USERPROFILE%\\AppData\\LocalLow", true),
        FileRoot::new("%PUBLIC%\\Desktop", true),
        FileRoot::new("%USERPROFILE%\\Desktop", false),
        FileRoot::new("%SystemRoot%\\System32", false),
        FileRoot::new("%SystemRoot%\\System32\\drivers", false),
        FileRoot::new("%SystemRoot%\\SysWOW64", false),
        FileRoot::new("%SystemRoot%\\System32\\Tasks", true),
        FileRoot::new("%SystemRoot%\\Fonts", false),
    ]
}

/// Registry locations Windows itself changes constantly; capturing them
/// would bury the real installer changes in noise.
pub fn default_registry_exclusions() -> Vec<String> {
    [
        "HKLM\\SOFTWARE\\Microsoft\\Windows Defender",
        "HKLM\\SOFTWARE\\Microsoft\\Windows Defender Security Center",
        "HKLM\\SOFTWARE\\Microsoft\\Windows Search",
        "HKLM\\SOFTWARE\\Microsoft\\Cryptography\\RNG",
        "HKLM\\SOFTWARE\\Microsoft\\Windows NT\\CurrentVersion\\Perflib",
        "HKLM\\SOFTWARE\\Microsoft\\Windows NT\\CurrentVersion\\Notifications",
        "HKLM\\SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\Diagnostics",
        "HKLM\\SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\WindowsUpdate",
        "HKLM\\SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\AppModel",
        "HKLM\\SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\CloudStore",
        "HKLM\\SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\Reliability",
        "HKLM\\SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\Authentication",
        "HKLM\\SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\Component Based Servicing",
        "HKLM\\SOFTWARE\\Microsoft\\SecurityManager",
        "HKLM\\SOFTWARE\\Microsoft\\SQMClient",
        "HKLM\\SOFTWARE\\Microsoft\\Tracing",
        "HKLM\\SOFTWARE\\Microsoft\\WlanSvc",
        "HKLM\\SOFTWARE\\Microsoft\\Provisioning",
        "HKLM\\SOFTWARE\\Microsoft\\EdgeUpdate",
        "HKLM\\SOFTWARE\\WOW6432Node\\Microsoft\\EdgeUpdate",
        "HKLM\\SYSTEM\\CurrentControlSet\\Services\\bam",
        "HKLM\\SYSTEM\\CurrentControlSet\\Services\\Tcpip\\Parameters\\Interfaces",
        "HKLM\\SYSTEM\\CurrentControlSet\\Services\\NlaSvc\\Parameters\\Internet",
        "HKLM\\SYSTEM\\CurrentControlSet\\Services\\SharedAccess\\Epoch",
        "HKLM\\SYSTEM\\CurrentControlSet\\Services\\*\\Parameters\\Tcpip",
        "HKCU\\Software\\Microsoft\\Windows\\CurrentVersion\\Explorer\\UserAssist",
        "HKCU\\Software\\Microsoft\\Windows\\CurrentVersion\\Explorer\\RecentDocs",
        "HKCU\\Software\\Microsoft\\Windows\\CurrentVersion\\Explorer\\ComDlg32",
        "HKCU\\Software\\Microsoft\\Windows\\CurrentVersion\\Explorer\\FeatureUsage",
        "HKCU\\Software\\Microsoft\\Windows\\CurrentVersion\\Explorer\\SessionInfo",
        "HKCU\\Software\\Microsoft\\Windows\\CurrentVersion\\Explorer\\TypedPaths",
        "HKCU\\Software\\Microsoft\\Windows\\CurrentVersion\\Explorer\\RunMRU",
        "HKCU\\Software\\Microsoft\\Windows\\CurrentVersion\\Explorer\\MountPoints2",
        "HKCU\\Software\\Microsoft\\Windows\\CurrentVersion\\Explorer\\StartPage2",
        "HKCU\\Software\\Microsoft\\Windows\\CurrentVersion\\Explorer\\BamThrottling",
        "HKCU\\Software\\Microsoft\\Windows\\CurrentVersion\\Explorer\\Streams",
        "HKCU\\Software\\Microsoft\\Windows\\CurrentVersion\\CloudStore",
        "HKCU\\Software\\Microsoft\\Windows\\CurrentVersion\\Notifications",
        "HKCU\\Software\\Microsoft\\Windows\\CurrentVersion\\PushNotifications",
        "HKCU\\Software\\Microsoft\\Windows\\CurrentVersion\\Search",
        "HKCU\\Software\\Microsoft\\Windows\\CurrentVersion\\Store",
        "HKCU\\Software\\Microsoft\\Windows\\Shell\\BagMRU",
        "HKCU\\Software\\Microsoft\\Windows\\Shell\\Bags",
        "HKCU\\Software\\Classes\\Local Settings\\Software\\Microsoft\\Windows\\Shell\\BagMRU",
        "HKCU\\Software\\Classes\\Local Settings\\Software\\Microsoft\\Windows\\Shell\\Bags",
        "HKCU\\Software\\Classes\\Local Settings\\Software\\Microsoft\\Windows\\Shell\\MuiCache",
        "HKCU\\Software\\Classes\\Local Settings\\Software\\Microsoft\\Windows\\CurrentVersion\\AppModel",
        "HKCU\\Software\\Microsoft\\Windows NT\\CurrentVersion\\AppCompatFlags\\Compatibility Assistant",
        "HKCU\\Software\\Microsoft\\Internet Explorer",
        "HKCU\\Software\\Microsoft\\Edge",
        "HKCU\\Software\\Microsoft\\OneDrive",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect()
}

pub fn default_file_exclusions() -> Vec<String> {
    [
        "%TEMP%",
        "%LOCALAPPDATA%\\Temp",
        "%SystemRoot%\\Temp",
        "%LOCALAPPDATA%\\Packages",
        "%LOCALAPPDATA%\\Microsoft\\Windows\\INetCache",
        "%LOCALAPPDATA%\\Microsoft\\Windows\\Explorer",
        "%LOCALAPPDATA%\\Microsoft\\Windows\\WebCache",
        "%LOCALAPPDATA%\\Microsoft\\Windows\\Notifications",
        "%LOCALAPPDATA%\\Microsoft\\Windows\\History",
        "%LOCALAPPDATA%\\Microsoft\\Edge\\User Data",
        "%LOCALAPPDATA%\\Microsoft\\OneDrive\\logs",
        "%LOCALAPPDATA%\\Google\\Chrome\\User Data",
        "%LOCALAPPDATA%\\BraveSoftware\\Brave-Browser\\User Data",
        "%LOCALAPPDATA%\\Mozilla\\Firefox\\Profiles",
        "%APPDATA%\\Mozilla\\Firefox\\Profiles",
        "%LOCALAPPDATA%\\D3DSCache",
        "%LOCALAPPDATA%\\CrashDumps",
        "%LOCALAPPDATA%\\ConnectedDevicesPlatform",
        "%LOCALAPPDATA%\\NVIDIA\\DXCache",
        "%LOCALAPPDATA%\\NVIDIA\\GLCache",
        "%APPDATA%\\Microsoft\\Windows\\Recent",
        "%ProgramData%\\Microsoft\\Windows Defender",
        "%ProgramData%\\Microsoft\\Search",
        "%ProgramData%\\Microsoft\\Diagnosis",
        "%ProgramData%\\Microsoft\\Windows\\WER",
        "%ProgramData%\\Microsoft\\Windows\\AppRepository",
        "%ProgramData%\\Microsoft\\Network",
        "%ProgramData%\\Packages",
        "%ProgramData%\\USOPrivate",
        "%ProgramData%\\USOShared",
        "%ProgramFiles%\\WindowsApps",
        "%ProgramFiles%\\Windows Defender Advanced Threat Protection",
        "*.etl",
        "*\\desktop.ini",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect()
}

#[cfg(test)]
mod confirm_tests {
    use super::*;

    #[test]
    fn always_confirm_actions_ask_in_every_mode() {
        // Simple mode (the default): everything asks.
        let s = Settings::default();
        assert!(s.must_confirm(false));
        assert!(s.must_confirm(true));
        // Advanced mode with confirmations on: everything asks.
        let mut s = Settings {
            advanced_mode: true,
            ..Settings::default()
        };
        assert!(s.must_confirm(false));
        assert!(s.must_confirm(true));
        // Advanced mode with confirmations off: ordinary actions skip the
        // dialog, always-confirm actions still ask.
        s.confirm_actions = false;
        assert!(!s.confirm_enabled());
        assert!(!s.must_confirm(false));
        assert!(s.must_confirm(true));
    }
}

#[cfg(test)]
mod profile_tests {
    use super::*;

    #[test]
    fn profiles_round_trip() {
        let mut s = Settings::default();
        s.save_profile("Deep");
        s.registry_roots = vec!["HKCU\\Software".into()];
        s.save_profile("Quick");
        assert_eq!(s.profiles.len(), 2);
        assert!(s.use_profile("deep"));
        assert_eq!(s.registry_roots, default_registry_roots());
        assert_eq!(s.active_profile, "Deep");
        assert!(s.use_profile("Quick"));
        assert_eq!(s.registry_roots, vec!["HKCU\\Software".to_string()]);
        assert!(!s.use_profile("missing"));
        s.delete_profile("QUICK");
        assert_eq!(s.profiles.len(), 1);
        assert!(s.active_profile.is_empty());
    }
}

#[cfg(test)]
mod restore_point_default_tests {
    use super::*;

    fn settings_from(json: &str) -> Settings {
        crate::json::compat::from_str(json).expect("test JSON must parse")
    }

    #[test]
    fn restore_point_is_on_by_default() {
        assert!(Settings::default().create_restore_point);
    }

    #[test]
    fn missing_field_gets_the_new_default() {
        assert!(settings_from("{}").create_restore_point);
    }

    #[test]
    fn explicit_false_in_an_existing_file_is_kept() {
        let s = settings_from(r#"{"create_restore_point":false}"#);
        assert!(!s.create_restore_point);
        // And an explicit true still parses as true.
        assert!(settings_from(r#"{"create_restore_point":true}"#).create_restore_point);
    }

    #[test]
    fn round_trip_preserves_the_value() {
        let mut s = Settings::default();
        s.create_restore_point = false;
        let text = crate::json::compat::to_string_pretty(&s).expect("must serialize");
        assert!(!settings_from(&text).create_restore_point);
    }
}
