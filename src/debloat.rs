//! OEM Bloatware Shield: finds the trial software, promotions and
//! pre-installed apps that PC makers and Windows add to new PCs, including
//! the Store apps standard uninstallers skip and the copies Windows keeps to
//! install for new user accounts ("provisioned" packages).
//!
//! Every match is classified: *Recommended* (trials, ads, games and
//! promotions nobody asked for) or *Optional* (maker tools that some people
//! use, such as support assistants). Drivers, audio and hotkey utilities,
//! security that protects the PC, and Windows' own parts are never listed.

use crate::programs::{InstalledProgram, Source};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Class {
    /// Safe to remove for almost everyone.
    Recommended,
    /// Useful to some people; unticked by default.
    Optional,
}
crate::json_enum!(Class {
    Recommended,
    Optional
});

impl Class {
    pub fn label(self) -> &'static str {
        match self {
            Class::Recommended => "Recommended",
            Class::Optional => "Optional",
        }
    }
}

struct Rule {
    /// Lower-case text matched against the program or package name.
    pattern: &'static str,
    /// Match package names from the start (Store apps) instead of anywhere.
    store: bool,
    class: Class,
    why: &'static str,
}

const fn r(pattern: &'static str, class: Class, why: &'static str) -> Rule {
    Rule {
        pattern,
        store: false,
        class,
        why,
    }
}
const fn s(pattern: &'static str, class: Class, why: &'static str) -> Rule {
    Rule {
        pattern,
        store: true,
        class,
        why,
    }
}

use Class::{Optional as O, Recommended as R};

const RULES: &[Rule] = &[
    // Trials and promotions bundled by PC makers.
    r(
        "mcafee livesafe",
        R,
        "Antivirus trial; Microsoft Defender already protects the PC",
    ),
    r(
        "mcafee webadvisor",
        R,
        "Browser add-on bundled with new PCs",
    ),
    r(
        "mcafee security",
        R,
        "Antivirus trial; Microsoft Defender already protects the PC",
    ),
    r(
        "mcafee personal security",
        R,
        "Antivirus trial; Microsoft Defender already protects the PC",
    ),
    r(
        "norton security",
        R,
        "Antivirus trial; Microsoft Defender already protects the PC",
    ),
    r(
        "norton 360",
        R,
        "Antivirus trial; Microsoft Defender already protects the PC",
    ),
    r("norton safe web", R, "Browser add-on bundled with new PCs"),
    r("expressvpn", R, "Paid VPN offer pre-installed by the maker"),
    r("wildtangent", R, "Game store with trial games"),
    r("booking.com", R, "Advertising shortcut"),
    r("dropbox promotion", R, "Storage promotion"),
    r("dropbox 25 gb", R, "Storage promotion"),
    r(
        "amazon alexa",
        O,
        "Voice assistant pre-installed by the maker",
    ),
    // HP.
    r("hp jumpstart", R, "Welcome and offers app"),
    r("hp documentation", R, "Manuals you can read online"),
    r("hp connection optimizer", R, "Rarely needed network helper"),
    r("hp audio switch", O, "Audio device switcher"),
    r(
        "hp support assistant",
        O,
        "Driver updates and warranty tool",
    ),
    r(
        "hp wolf security",
        O,
        "Extra security layer; Defender already protects the PC",
    ),
    r("hp sure click", O, "Extra browser isolation"),
    r("myhp", O, "HP's welcome and settings app"),
    r("hp smart", O, "Printer app, only needed with an HP printer"),
    // Dell.
    r(
        "dell digital delivery",
        R,
        "Downloads software bought with the PC",
    ),
    r("dell customer connect", R, "Surveys and offers"),
    r("dell mobile connect", R, "Discontinued phone link app"),
    r("dell supportassist", O, "Driver updates and warranty tool"),
    r("dell optimizer", O, "Performance tuning tool"),
    r("dell update", O, "Driver and BIOS updates"),
    r(
        "dell display manager",
        O,
        "Monitor settings, only for Dell monitors",
    ),
    // Lenovo.
    r("lenovo welcome", R, "Welcome and offers app"),
    r("lenovo now", R, "Offers and app store"),
    r("lenovo utility", O, "Hotkey helper on older models"),
    r("lenovo vantage", O, "Battery, driver and warranty settings"),
    r("lenovo smart appearance", O, "Camera effects"),
    s(
        "e046963f.lenovocompanion",
        O,
        "Lenovo Vantage (battery, driver and warranty settings)",
    ),
    s("e0469640.lenovoutility", O, "Lenovo hotkey helper"),
    // Acer.
    r("acer jumpstart", R, "Welcome and offers app"),
    r("acer collection", R, "App offers"),
    r("acer product registration", R, "Registration prompts"),
    r("acercloud", R, "Discontinued cloud service"),
    r("acer care center", O, "Driver updates and diagnostics"),
    r(
        "acer quick access",
        O,
        "Quick settings for some Acer models",
    ),
    // ASUS.
    r("asus giftbox", R, "App offers"),
    r("asus welcome", R, "Welcome app"),
    r("myasus", O, "ASUS settings, driver updates and warranty"),
    s("b9eceab9.asusgiftbox", R, "App offers"),
    // MSI, Samsung, Toshiba/Dynabook.
    r(
        "msi app player",
        R,
        "Android emulator bundled for promotions",
    ),
    r("samsung update", O, "Driver updates for Samsung PCs"),
    r(
        "dynabook service station",
        O,
        "Driver updates for Dynabook PCs",
    ),
    // Store apps Windows adds for consumers.
    s("microsoft.bingnews", R, "News app"),
    s("microsoft.bingweather", O, "Weather app"),
    s("microsoft.bingsearch", R, "Bing search add-on"),
    s("microsoft.getstarted", R, "Tips app"),
    s("microsoft.gethelp", O, "Get Help (Microsoft support chat)"),
    s(
        "microsoft.microsoftsolitairecollection",
        R,
        "Card games with ads",
    ),
    s("microsoft.microsoftofficehub", R, "Microsoft 365 promotion"),
    s("microsoft.people", R, "Old contacts app"),
    s(
        "microsoft.windowsfeedbackhub",
        O,
        "Sends feedback to Microsoft",
    ),
    s("microsoft.zunevideo", O, "Movies & TV"),
    s("microsoft.windowsmaps", O, "Maps"),
    s("microsoft.todos", O, "Microsoft To Do"),
    s(
        "microsoft.powerautomatedesktop",
        O,
        "Automation tool most people never use",
    ),
    s("microsoft.549981c3f5f10", R, "Cortana, discontinued"),
    s("microsoft.windows.devhome", O, "Developer dashboard"),
    s("microsoft.outlookforwindows", O, "The new Outlook"),
    s("microsoftcorporationii.microsoftfamily", O, "Family Safety"),
    s("microsoftcorporationii.quickassist", O, "Remote help tool"),
    s("clipchamp.clipchamp", O, "Video editor"),
    s("msteams", O, "Microsoft Teams (personal)"),
    s("microsoftteams", O, "Microsoft Teams (personal)"),
    s("microsoft.copilot", O, "Copilot app"),
    s("microsoft.windows.ai.copilot", O, "Copilot app"),
    // Sponsored Store apps and games.
    s("king.com.", R, "Sponsored game (Candy Crush and others)"),
    s(
        "spotifyab.spotifymusic",
        O,
        "Spotify, pre-installed as a promotion",
    ),
    s("disney.", R, "Sponsored app"),
    s("facebook.", R, "Sponsored app"),
    s("bytedancepte.ltd.tiktok", R, "Sponsored app"),
    s("amazonvideo.primevideo", R, "Sponsored app"),
    s(
        "4df9e0f8.netflix",
        O,
        "Netflix, pre-installed as a promotion",
    ),
    s("7ee7776c.linkedinforwindows", R, "Sponsored app"),
    s("9e2f88e3.twitter", R, "Sponsored app"),
    s(
        "5a894077.mcafeesecurity",
        R,
        "Antivirus trial; Microsoft Defender already protects the PC",
    ),
    s("a278ab0d.marchofempires", R, "Sponsored game"),
    s("a278ab0d.dragonmaniarlegends", R, "Sponsored game"),
    s("2fe3cb00.picsart", R, "Sponsored app"),
    s(
        "adobesystemsincorporated.adobephotoshopexpress",
        R,
        "Sponsored app",
    ),
    s("duolingo-learnlanguagesforfree", R, "Sponsored app"),
    s("eclipsemanager", R, "Sponsored app"),
    s("actipro", R, "Sponsored app"),
    s("phototastic", R, "Sponsored app"),
    s("shazam", R, "Sponsored app"),
    s("flipboard", R, "Sponsored app"),
    s("wildtangentgames", R, "Game store with trial games"),
    s("royalrevolt", R, "Sponsored game"),
    s("hiddencity", R, "Sponsored game"),
    s("bubblewitch", R, "Sponsored game"),
    s("farmville", R, "Sponsored game"),
    s("asphalt", R, "Sponsored game"),
];

/// Never listed, whatever the name says: drivers and the tools that run
/// hardware (audio, touchpad, hotkeys, graphics, firmware).
const NEVER: &[&str] = &[
    "driver",
    "realtek",
    "synaptics",
    "elan",
    "intel",
    "nvidia",
    "amd ",
    "radeon",
    "dolby",
    "dts ",
    "waves maxxaudio",
    "thunderbolt",
    "bios",
    "firmware",
    "hotkey",
    "system event utility",
    "smart sound",
    "precision touchpad",
    "chipset",
    "bluetooth",
    "wi-fi",
    "wireless",
    "killer",
    "xbox identity",
    "xboxgameoverlay",
    "xbox.tcui",
    "microsoft.windowsstore",
    "microsoft.storepurchaseapp",
    "desktopappinstaller",
    "microsoft.windows.photos",
    "microsoft.windowscalculator",
    "microsoft.windowsnotepad",
    "microsoft.windowsterminal",
    "microsoft.screensketch",
    "microsoft.paint",
    "microsoft.hevcvideoextension",
    "microsoft.webpimageextension",
    "microsoft.vp9videoextensions",
    "microsoft.heifimageextension",
    "microsoft.webmediaextensions",
    "microsoft.sechealthui",
    "microsoft.windowscamera",
    "microsoft.windows.shellexperiencehost",
];

#[derive(Clone, Debug, PartialEq)]
pub struct BloatItem {
    pub program: InstalledProgram,
    pub class: Class,
    pub why: String,
    /// Also kept by Windows for new user accounts (Store apps only).
    pub provisioned: bool,
}
crate::json_struct!(BloatItem, { program, class, why, provisioned });

/// The outcome of the most recent deprovisioning run, kept in the data
/// folder so the user can see later what was deprovisioned and what failed.
/// Deprovisioning cannot be undone, so its record must outlive the status
/// line it was reported on.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct DeprovisionReport {
    pub version: u32,
    /// Local time the run finished (`DateTime::ymd_hms`).
    pub ran_at: String,
    pub rows: Vec<crate::backup::ActionResult>,
}
crate::json_struct!(DeprovisionReport, default, { version, ran_at, rows });

fn matches(rule: &Rule, p: &InstalledProgram) -> bool {
    let name = p.name.to_lowercase();
    if rule.store {
        p.source == Source::Store
            && (name.starts_with(rule.pattern) || p.key.to_lowercase().starts_with(rule.pattern))
    } else {
        p.source != Source::Store && name.contains(rule.pattern)
    }
}

fn never(p: &InstalledProgram) -> bool {
    let name = p.name.to_lowercase();
    NEVER.iter().any(|n| name.contains(n))
}

impl BloatItem {
    /// The name to show: Store package names ("king.com.CandyCrushSaga")
    /// become readable titles ("Candy Crush Saga").
    pub fn title(&self) -> String {
        if self.program.source == Source::Store {
            store_title(&self.program.name)
        } else {
            self.program.name.clone()
        }
    }
}

/// A readable title for a Store package name: the part after the publisher
/// prefix, with words split at capitals.
pub fn store_title(package: &str) -> String {
    let last = package.rsplit('.').next().unwrap_or(package);
    let looks_like_id =
        last.chars().any(|c| c.is_ascii_digit()) && last.chars().all(|c| c.is_ascii_hexdigit());
    if last.is_empty() || looks_like_id {
        return package.to_string();
    }
    let mut out = String::new();
    let chars: Vec<char> = last.chars().collect();
    for (i, &c) in chars.iter().enumerate() {
        if i > 0 && c.is_uppercase() {
            let prev = chars[i - 1];
            let next_lower = chars.get(i + 1).is_some_and(|n| n.is_lowercase());
            if prev.is_lowercase() || prev.is_ascii_digit() || (prev.is_uppercase() && next_lower) {
                out.push(' ');
            }
        }
        out.push(c);
    }
    out
}

/// Find bloatware among installed desktop programs and Store apps.
/// `provisioned` holds the package names Windows keeps for new accounts.
pub fn scan(
    programs: &[InstalledProgram],
    store: &[InstalledProgram],
    provisioned: &[String],
) -> Vec<BloatItem> {
    let mut out: Vec<BloatItem> = Vec::new();
    for p in programs.iter().chain(store) {
        if p.system_component || never(p) {
            continue;
        }
        if let Some(rule) = RULES.iter().find(|r| matches(r, p)) {
            let provisioned = p.source == Source::Store
                && provisioned.iter().any(|x| x.eq_ignore_ascii_case(&p.name));
            out.push(BloatItem {
                program: p.clone(),
                class: rule.class,
                why: rule.why.into(),
                provisioned,
            });
        }
    }
    out.sort_by_cached_key(|i| (i.class, i.title().to_lowercase()));
    out
}

/// Names of the Store packages Windows installs for new user accounts.
pub fn provisioned_packages() -> Vec<String> {
    #[cfg(windows)]
    {
        let script =
            "Get-AppxProvisionedPackage -Online | Select-Object -ExpandProperty DisplayName";
        if let Ok((0, out)) = crate::platform::run_hidden(
            "powershell.exe",
            &[
                "-NoProfile",
                "-NonInteractive",
                "-ExecutionPolicy",
                "Bypass",
                "-Command",
                script,
            ],
        ) {
            return out
                .lines()
                .map(|l| l.trim().to_string())
                .filter(|l| !l.is_empty())
                .collect();
        }
        Vec::new()
    }
    #[cfg(not(windows))]
    {
        Vec::new()
    }
}

/// Split the confirmed deprovision list into what may proceed and what must
/// be skipped. Deprovisioning is irreversible and machine-wide, so it may
/// only run for programs that were actually removed in the same flow: the
/// caller passes the names it confirmed for deprovisioning (`pending`) and
/// the names whose uninstall completed (`removed`). Pending names that were
/// skipped, failed or cancelled keep their provisioned copies and are
/// returned as `skipped`, in order, without duplicates.
pub fn partition_deprovision(pending: &[String], removed: &[String]) -> (Vec<String>, Vec<String>) {
    let mut proceed = Vec::new();
    let mut skipped = Vec::new();
    for name in pending {
        if proceed.contains(name) || skipped.contains(name) {
            continue;
        }
        if removed.iter().any(|r| r == name) {
            proceed.push(name.clone());
        } else {
            skipped.push(name.clone());
        }
    }
    (proceed, skipped)
}

/// Stop Windows from installing a Store app for new user accounts
/// (needs administrator rights).
pub fn deprovision(name: &str) -> anyhow::Result<()> {
    let safe: String = name
        .chars()
        .filter(|c| c.is_alphanumeric() || "._-".contains(*c))
        .collect();
    let script = format!(
        "Get-AppxProvisionedPackage -Online | Where-Object {{ $_.DisplayName -eq '{safe}' }} | Remove-AppxProvisionedPackage -Online | Out-Null"
    );
    let (code, out) = crate::platform::run_hidden(
        "powershell.exe",
        &[
            "-NoProfile",
            "-NonInteractive",
            "-ExecutionPolicy",
            "Bypass",
            "-Command",
            &script,
        ],
    )?;
    if code != 0 {
        anyhow::bail!("{}", out.trim());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn win32(name: &str, publisher: &str) -> InstalledProgram {
        InstalledProgram {
            key: name.into(),
            name: name.into(),
            publisher: publisher.into(),
            ..Default::default()
        }
    }
    fn store(name: &str) -> InstalledProgram {
        InstalledProgram {
            key: format!("{name}_1.0_x64__abc"),
            name: name.into(),
            source: Source::Store,
            ..Default::default()
        }
    }

    fn names(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn deprovision_only_for_programs_actually_removed() {
        // Everything removed: everything proceeds, nothing skipped.
        let (go, skip) = partition_deprovision(&names(&["A", "B"]), &names(&["A", "B"]));
        assert_eq!(go, names(&["A", "B"]));
        assert!(skip.is_empty());
        // Nothing removed (the removal was cancelled or failed): nothing
        // may proceed.
        let (go, skip) = partition_deprovision(&names(&["A", "B"]), &[]);
        assert!(go.is_empty());
        assert_eq!(skip, names(&["A", "B"]));
        // Mixed: only the removed ones proceed; order follows the
        // confirmed list, not the removal list.
        let (go, skip) = partition_deprovision(&names(&["A", "B", "C"]), &names(&["C", "A"]));
        assert_eq!(go, names(&["A", "C"]));
        assert_eq!(skip, names(&["B"]));
        // Duplicates in the confirmed list never produce two runs.
        let (go, skip) = partition_deprovision(&names(&["A", "A"]), &names(&["A"]));
        assert_eq!(go, names(&["A"]));
        assert!(skip.is_empty());
        // Removed names that were never confirmed are ignored.
        let (go, skip) = partition_deprovision(&names(&["A"]), &names(&["A", "Z"]));
        assert_eq!(go, names(&["A"]));
        assert!(skip.is_empty());
    }

    #[test]
    fn deprovision_report_round_trips() {
        let report = DeprovisionReport {
            version: 1,
            ran_at: "2026-10-20 01:40:00".into(),
            rows: vec![
                crate::backup::ActionResult::ok("App", "Deprovisioned"),
                crate::backup::ActionResult::err("Other", "access denied"),
            ],
        };
        let text = crate::json::compat::to_string(&report).unwrap();
        let back: DeprovisionReport = crate::json::compat::from_str(&text).unwrap();
        assert_eq!(back, report);
        // A report written by an older/newer build with missing or extra
        // fields still loads; absent rows read as empty.
        let partial: DeprovisionReport = crate::json::compat::from_str("{}").unwrap();
        assert!(partial.rows.is_empty());
    }

    #[test]
    fn readable_store_titles() {
        assert_eq!(store_title("king.com.CandyCrushSaga"), "Candy Crush Saga");
        assert_eq!(
            store_title("Microsoft.MicrosoftSolitaireCollection"),
            "Microsoft Solitaire Collection"
        );
        assert_eq!(store_title("4DF9E0F8.Netflix"), "Netflix");
        assert_eq!(store_title("MSTeams"), "MS Teams");
        assert_eq!(
            store_title("Microsoft.549981C3F5F10"),
            "Microsoft.549981C3F5F10"
        );
    }

    #[test]
    fn finds_and_classifies() {
        let progs = vec![
            win32("McAfee LiveSafe", "McAfee, LLC"),
            win32("Dell SupportAssist", "Dell Inc."),
            win32("Realtek Audio Driver", "Realtek"),
            win32("HP System Event Utility", "HP Inc."),
            win32("7-Zip 24.08", "Igor Pavlov"),
        ];
        let apps = vec![
            store("king.com.CandyCrushSaga"),
            store("Microsoft.BingWeather"),
            store("Microsoft.WindowsCalculator"),
        ];
        let found = scan(&progs, &apps, &["king.com.CandyCrushSaga".to_string()]);
        let names: Vec<&str> = found.iter().map(|b| b.program.name.as_str()).collect();
        assert_eq!(
            names,
            vec![
                "king.com.CandyCrushSaga",
                "McAfee LiveSafe",
                "Microsoft.BingWeather",
                "Dell SupportAssist"
            ]
        );
        assert_eq!(found[0].class, Class::Recommended);
        assert!(found[0].provisioned);
        assert_eq!(found[2].class, Class::Optional);
    }

    #[test]
    fn never_lists_drivers_or_windows_tools() {
        let progs = vec![
            win32("Intel Driver & Support Assistant", "Intel"),
            win32("Dolby Access", "Dolby"),
        ];
        let apps = vec![
            store("Microsoft.WindowsStore"),
            store("Microsoft.XboxIdentityProvider"),
        ];
        assert!(scan(&progs, &apps, &[]).is_empty());
    }
}
