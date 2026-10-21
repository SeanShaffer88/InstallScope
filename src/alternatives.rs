//! The winget / Scoop bridge: lightweight or open-source alternatives to the
//! program being uninstalled, installable through Windows' package manager
//! (winget) or Scoop with one click. The list is curated and kept in
//! SetupSweep; nothing is looked up online until you choose to install.

#[derive(Clone, Debug, PartialEq)]
pub struct Alternative {
    pub name: &'static str,
    /// What it is, in a few words.
    pub what: &'static str,
    pub open_source: bool,
    /// winget package id (exact).
    pub winget: Option<&'static str>,
    /// Scoop app, with its bucket ("extras/qbittorrent").
    pub scoop: Option<&'static str>,
}
crate::json_struct!(Alternative, write_only, { name, what, open_source, winget, scoop });

const fn a(
    name: &'static str,
    what: &'static str,
    open_source: bool,
    winget: &'static str,
    scoop: &'static str,
) -> Alternative {
    Alternative {
        name,
        what,
        open_source,
        winget: if winget.is_empty() {
            None
        } else {
            Some(winget)
        },
        scoop: if scoop.is_empty() { None } else { Some(scoop) },
    }
}

const SEVENZIP: Alternative = a(
    "7-Zip",
    "Archive manager, small and fast",
    true,
    "7zip.7zip",
    "main/7zip",
);
const NANAZIP: Alternative = a(
    "NanaZip",
    "7-Zip with Windows 11 menus",
    true,
    "M2Team.NanaZip",
    "",
);
const SUMATRA: Alternative = a(
    "SumatraPDF",
    "Tiny PDF, e-book and comic reader",
    true,
    "SumatraPDF.SumatraPDF",
    "extras/sumatrapdf",
);
const OKULAR: Alternative = a(
    "Okular",
    "Document viewer with annotations",
    true,
    "KDE.Okular",
    "extras/okular",
);
const LIBREOFFICE: Alternative = a(
    "LibreOffice",
    "Full office suite",
    true,
    "TheDocumentFoundation.LibreOffice",
    "extras/libreoffice",
);
const ONLYOFFICE: Alternative = a(
    "ONLYOFFICE Desktop",
    "Office suite with Microsoft-style layout",
    true,
    "ONLYOFFICE.DesktopEditors",
    "extras/onlyoffice-desktopeditors",
);
const GIMP: Alternative = a("GIMP", "Photo editor", true, "GIMP.GIMP.3", "extras/gimp");
const KRITA: Alternative = a(
    "Krita",
    "Painting and illustration",
    true,
    "KDE.Krita",
    "extras/krita",
);
const PAINTNET: Alternative = a(
    "Paint.NET",
    "Simple photo editor",
    false,
    "dotPDN.PaintDotNet",
    "extras/paint.net",
);
const INKSCAPE: Alternative = a(
    "Inkscape",
    "Vector graphics",
    true,
    "Inkscape.Inkscape",
    "extras/inkscape",
);
const DARKTABLE: Alternative = a(
    "darktable",
    "RAW photo editor",
    true,
    "darktable.darktable",
    "extras/darktable",
);
const VLC: Alternative = a(
    "VLC media player",
    "Plays almost any video or audio",
    true,
    "VideoLAN.VLC",
    "extras/vlc",
);
const MPCHC: Alternative = a(
    "MPC-HC",
    "Light video player",
    true,
    "clsid2.mpc-hc",
    "extras/mpc-hc-fork",
);
const QBITTORRENT: Alternative = a(
    "qBittorrent",
    "Torrent client without ads",
    true,
    "qBittorrent.qBittorrent",
    "extras/qbittorrent",
);
const HANDBRAKE: Alternative = a(
    "HandBrake",
    "Video converter",
    true,
    "HandBrake.HandBrake",
    "extras/handbrake",
);
const SHUTTER: Alternative = a("Shutter Encoder", "Video and audio converter", true, "", "");
const FFMPEG: Alternative = a(
    "FFmpeg",
    "Command-line media converter",
    true,
    "Gyan.FFmpeg",
    "main/ffmpeg",
);
const AUDACITY: Alternative = a(
    "Audacity",
    "Audio editor and recorder",
    true,
    "Audacity.Audacity",
    "extras/audacity",
);
const OBS: Alternative = a(
    "OBS Studio",
    "Screen recording and streaming",
    true,
    "OBSProject.OBSStudio",
    "extras/obs-studio",
);
const SHAREX: Alternative = a(
    "ShareX",
    "Screenshots and screen recording",
    true,
    "ShareX.ShareX",
    "extras/sharex",
);
const GREENSHOT: Alternative = a(
    "Greenshot",
    "Light screenshot tool",
    true,
    "Greenshot.Greenshot",
    "extras/greenshot",
);
const BLEACHBIT: Alternative = a(
    "BleachBit",
    "Cleaner without upsells",
    true,
    "BleachBit.BleachBit",
    "extras/bleachbit",
);
const KEEPASSXC: Alternative = a(
    "KeePassXC",
    "Offline password manager",
    true,
    "KeePassXCTeam.KeePassXC",
    "extras/keepassxc",
);
const BITWARDEN: Alternative = a(
    "Bitwarden",
    "Password manager with sync",
    true,
    "Bitwarden.Bitwarden",
    "extras/bitwarden",
);
const FIREFOX: Alternative = a(
    "Firefox",
    "Private, independent browser",
    true,
    "Mozilla.Firefox",
    "extras/firefox",
);
const LIBREWOLF: Alternative = a(
    "LibreWolf",
    "Privacy-hardened Firefox",
    true,
    "LibreWolf.LibreWolf",
    "extras/librewolf",
);
const BRAVE: Alternative = a(
    "Brave",
    "Chromium browser that blocks ads",
    true,
    "Brave.Brave",
    "extras/brave",
);
const THUNDERBIRD: Alternative = a(
    "Thunderbird",
    "Email and calendar",
    true,
    "Mozilla.Thunderbird",
    "extras/thunderbird",
);
const NOTEPADPP: Alternative = a(
    "Notepad++",
    "Fast text and code editor",
    true,
    "Notepad++.Notepad++",
    "extras/notepadplusplus",
);
const VSCODIUM: Alternative = a(
    "VSCodium",
    "VS Code without Microsoft telemetry",
    true,
    "VSCodium.VSCodium",
    "extras/vscodium",
);
const EVERYTHING: Alternative = a(
    "Everything",
    "Instant file search",
    false,
    "voidtools.Everything",
    "extras/everything",
);
const WIZTREE: Alternative = a(
    "WizTree",
    "Fast disk space analyzer",
    false,
    "AntibodySoftware.WizTree",
    "extras/wiztree",
);
const WINDIRSTAT: Alternative = a(
    "WinDirStat",
    "Disk space analyzer",
    true,
    "WinDirStat.WinDirStat",
    "extras/windirstat",
);
const SIGNAL: Alternative = a(
    "Signal",
    "Private messaging",
    true,
    "OpenWhisperSystems.Signal",
    "extras/signal",
);
const JITSI: Alternative = a(
    "Jitsi Meet",
    "Video calls in the browser, no account",
    true,
    "",
    "",
);
const ELEMENT: Alternative = a(
    "Element",
    "Chat and calls (Matrix)",
    true,
    "Element.Element",
    "extras/element",
);
const NEXTCLOUD: Alternative = a(
    "Nextcloud",
    "Self-hosted file sync",
    true,
    "Nextcloud.NextcloudDesktop",
    "extras/nextcloud",
);
const SYNCTHING: Alternative = a(
    "Syncthing",
    "Sync folders between your devices",
    true,
    "Syncthing.Syncthing",
    "main/syncthing",
);
const RUSTDESK: Alternative = a(
    "RustDesk",
    "Remote desktop",
    true,
    "RustDesk.RustDesk",
    "extras/rustdesk",
);
const MUSICBEE: Alternative = a(
    "MusicBee",
    "Music library and player",
    false,
    "",
    "extras/musicbee",
);
const FOOBAR: Alternative = a(
    "foobar2000",
    "Light music player",
    false,
    "PeterPawlowski.foobar2000",
    "extras/foobar2000",
);
const STRAWBERRY: Alternative = a(
    "Strawberry",
    "Music library and player",
    true,
    "StrawberryMusicPlayer.Strawberry",
    "extras/strawberry",
);
const CALIBRE: Alternative = a(
    "calibre",
    "E-book library and converter",
    true,
    "calibre.calibre",
    "extras/calibre",
);
const SIMPLEWALL: Alternative = a(
    "simplewall",
    "Simple firewall control",
    true,
    "Henry++.simplewall",
    "extras/simplewall",
);
const HWINFO: Alternative = a(
    "HWiNFO",
    "Hardware information",
    false,
    "REALiX.HWiNFO",
    "extras/hwinfo",
);
const LHM: Alternative = a(
    "LibreHardwareMonitor",
    "Temperatures and fan speeds",
    true,
    "LibreHardwareMonitor.LibreHardwareMonitor",
    "extras/librehardwaremonitor",
);
const PEAZIP: Alternative = a(
    "PeaZip",
    "Archive manager",
    true,
    "Giorgiotani.Peazip",
    "extras/peazip",
);
const IRFANVIEW: Alternative = a(
    "IrfanView",
    "Fast image viewer",
    false,
    "IrfanSkiljan.IrfanView",
    "extras/irfanview",
);
const NOMACS: Alternative = a(
    "nomacs",
    "Image viewer",
    true,
    "nomacs.nomacs",
    "extras/nomacs",
);
const KDENLIVE: Alternative = a(
    "Kdenlive",
    "Video editor",
    true,
    "KDE.Kdenlive",
    "extras/kdenlive",
);
const SHOTCUT: Alternative = a(
    "Shotcut",
    "Video editor",
    true,
    "Meltytech.Shotcut",
    "extras/shotcut",
);
const DAVINCI: Alternative = a(
    "DaVinci Resolve",
    "Professional video editor (free version)",
    false,
    "",
    "",
);
const FREECAD: Alternative = a(
    "FreeCAD",
    "3D CAD modeler",
    true,
    "FreeCAD.FreeCAD",
    "extras/freecad",
);
const BLENDER: Alternative = a(
    "Blender",
    "3D creation suite",
    true,
    "BlenderFoundation.Blender",
    "extras/blender",
);
const ANKI: Alternative = a("Anki", "Flashcards", true, "Anki.Anki", "extras/anki");
const JOPLIN: Alternative = a(
    "Joplin",
    "Notes with sync",
    true,
    "Joplin.Joplin",
    "extras/joplin",
);
const OBSIDIAN: Alternative = a(
    "Obsidian",
    "Notes in plain Markdown files",
    false,
    "Obsidian.Obsidian",
    "extras/obsidian",
);
const FLAMESHOT: Alternative = a(
    "Flameshot",
    "Screenshot and annotate",
    true,
    "Flameshot.Flameshot",
    "extras/flameshot",
);
const POWERTOYS: Alternative = a(
    "PowerToys",
    "Microsoft's own power-user tools",
    true,
    "Microsoft.PowerToys",
    "extras/powertoys",
);
const DEFENDER_NOTE: Alternative = a(
    "Microsoft Defender",
    "Already built into Windows 11; no install needed",
    false,
    "",
    "",
);
const UNIGETUI: Alternative = a(
    "UniGetUI",
    "Update every app through winget and Scoop",
    true,
    "MartiCliment.UniGetUI",
    "extras/unigetui",
);
const SNAPPY: Alternative = a(
    "Snappy Driver Installer Origin",
    "Offline driver updates without ads",
    true,
    "GlennDelahoy.SnappyDriverInstallerOrigin",
    "extras/snappy-driver-installer-origin",
);

struct Rule {
    patterns: &'static [&'static str],
    alternatives: &'static [Alternative],
}

const RULES: &[Rule] = &[
    Rule {
        patterns: &["winrar", "winzip", "bandizip", "izarc", "zipware"],
        alternatives: &[SEVENZIP, NANAZIP, PEAZIP],
    },
    Rule {
        patterns: &[
            "adobe acrobat",
            "acrobat reader",
            "foxit",
            "nitro pdf",
            "pdf reader",
            "pdfelement",
        ],
        alternatives: &[SUMATRA, OKULAR],
    },
    Rule {
        patterns: &[
            "microsoft office",
            "microsoft 365",
            "wps office",
            "kingsoft office",
            "softmaker",
        ],
        alternatives: &[LIBREOFFICE, ONLYOFFICE],
    },
    Rule {
        patterns: &[
            "photoshop",
            "corel paintshop",
            "affinity photo",
            "photo editor",
            "pixlr",
        ],
        alternatives: &[GIMP, KRITA, PAINTNET],
    },
    Rule {
        patterns: &["illustrator", "coreldraw", "affinity designer"],
        alternatives: &[INKSCAPE],
    },
    Rule {
        patterns: &["lightroom", "luminar", "capture one"],
        alternatives: &[DARKTABLE],
    },
    Rule {
        patterns: &[
            "premiere",
            "vegas pro",
            "filmora",
            "camtasia",
            "cyberlink powerdirector",
            "pinnacle studio",
            "movavi video editor",
        ],
        alternatives: &[KDENLIVE, SHOTCUT, DAVINCI],
    },
    Rule {
        patterns: &[
            "video converter",
            "videoconvert",
            "vidconvert",
            "freemake",
            "any video",
            "format factory",
            "wondershare uniconverter",
            "movavi video converter",
            "xilisoft",
        ],
        alternatives: &[HANDBRAKE, SHUTTER, FFMPEG],
    },
    Rule {
        patterns: &[
            "kmplayer",
            "gom player",
            "potplayer",
            "realplayer",
            "powerdvd",
            "5kplayer",
            "divx",
        ],
        alternatives: &[VLC, MPCHC],
    },
    Rule {
        patterns: &["utorrent", "bittorrent", "vuze", "frostwire", "bitcomet"],
        alternatives: &[QBITTORRENT],
    },
    Rule {
        patterns: &["audition", "goldwave", "wavepad", "sound forge"],
        alternatives: &[AUDACITY],
    },
    Rule {
        patterns: &[
            "bandicam",
            "fraps",
            "camstudio",
            "action!",
            "icecream screen recorder",
            "screen recorder",
        ],
        alternatives: &[OBS, SHAREX],
    },
    Rule {
        patterns: &["snagit", "lightshot", "gyazo", "picpick"],
        alternatives: &[SHAREX, GREENSHOT, FLAMESHOT],
    },
    Rule {
        patterns: &[
            "ccleaner",
            "advanced systemcare",
            "wise care",
            "glary utilities",
            "pc cleaner",
            "auslogics",
            "avg tuneup",
            "pc optimizer",
            "speedup",
            "reimage",
            "restoro",
            "fortect",
            "mackeeper",
            "driver booster",
            "driver easy",
            "drivermax",
        ],
        alternatives: &[BLEACHBIT, SNAPPY, POWERTOYS],
    },
    Rule {
        patterns: &[
            "lastpass",
            "dashlane",
            "1password",
            "roboform",
            "keeper password",
        ],
        alternatives: &[BITWARDEN, KEEPASSXC],
    },
    Rule {
        patterns: &[
            "google chrome",
            "opera",
            "microsoft edge",
            "yandex browser",
            "avast secure browser",
            "avg secure browser",
            "ccleaner browser",
        ],
        alternatives: &[FIREFOX, LIBREWOLF, BRAVE],
    },
    Rule {
        patterns: &["outlook", "mailbird", "em client", "windows live mail"],
        alternatives: &[THUNDERBIRD],
    },
    Rule {
        patterns: &["ultraedit", "editplus", "sublime text", "textpad"],
        alternatives: &[NOTEPADPP, VSCODIUM],
    },
    Rule {
        patterns: &["visual studio code"],
        alternatives: &[VSCODIUM],
    },
    Rule {
        patterns: &["copernic", "x1 search", "listary"],
        alternatives: &[EVERYTHING],
    },
    Rule {
        patterns: &["treesize", "spacesniffer", "disk analyzer"],
        alternatives: &[WIZTREE, WINDIRSTAT],
    },
    Rule {
        patterns: &["whatsapp", "telegram", "viber", "facebook messenger"],
        alternatives: &[SIGNAL, ELEMENT],
    },
    Rule {
        patterns: &["zoom", "skype", "webex", "gotomeeting"],
        alternatives: &[JITSI, ELEMENT],
    },
    Rule {
        patterns: &["dropbox", "google drive", "box drive", "pcloud", "mega"],
        alternatives: &[SYNCTHING, NEXTCLOUD],
    },
    Rule {
        patterns: &["teamviewer", "anydesk", "logmein", "splashtop"],
        alternatives: &[RUSTDESK],
    },
    Rule {
        patterns: &["itunes", "winamp", "mediamonkey", "aimp"],
        alternatives: &[STRAWBERRY, MUSICBEE, FOOBAR],
    },
    Rule {
        patterns: &["adobe digital editions", "kindle", "kobo"],
        alternatives: &[CALIBRE],
    },
    Rule {
        patterns: &["glasswire", "zonealarm", "comodo firewall"],
        alternatives: &[SIMPLEWALL],
    },
    Rule {
        patterns: &["aida64", "speccy", "sisoftware"],
        alternatives: &[HWINFO, LHM],
    },
    Rule {
        patterns: &["acdsee", "faststone", "xnview"],
        alternatives: &[IRFANVIEW, NOMACS],
    },
    Rule {
        patterns: &["autocad", "sketchup", "solidworks", "fusion 360"],
        alternatives: &[FREECAD, BLENDER],
    },
    Rule {
        patterns: &["3ds max", "maya", "cinema 4d"],
        alternatives: &[BLENDER],
    },
    Rule {
        patterns: &["evernote", "onenote", "notion"],
        alternatives: &[JOPLIN, OBSIDIAN],
    },
    Rule {
        patterns: &["quizlet", "brainscape"],
        alternatives: &[ANKI],
    },
    Rule {
        patterns: &[
            "mcafee",
            "norton",
            "avast",
            "avg antivirus",
            "avira",
            "kaspersky",
            "bitdefender",
            "total av",
            "totalav",
            "pc matic",
            "webroot",
            "panda dome",
        ],
        alternatives: &[DEFENDER_NOTE],
    },
    Rule {
        patterns: &["ninite", "patch my pc", "sumo", "software updater"],
        alternatives: &[UNIGETUI],
    },
];

/// Alternatives for a program, by its name. Programs that are themselves
/// in the list get none.
pub fn for_program(name: &str) -> Vec<Alternative> {
    let n = name.to_lowercase();
    let Some(rule) = RULES
        .iter()
        .find(|r| r.patterns.iter().any(|p| n.contains(p)))
    else {
        return Vec::new();
    };
    rule.alternatives
        .iter()
        .filter(|a| !n.contains(&a.name.to_lowercase()))
        .cloned()
        .collect()
}

/// Which package managers are on this PC.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Managers {
    pub winget: bool,
    pub scoop: bool,
}

pub fn detect() -> Managers {
    #[cfg(windows)]
    {
        let winget =
            crate::platform::run_hidden("winget.exe", &["--version"]).is_ok_and(|(c, _)| c == 0);
        let scoop = std::env::var("SCOOP")
            .map(std::path::PathBuf::from)
            .or_else(|_| {
                std::env::var("USERPROFILE").map(|h| std::path::Path::new(&h).join("scoop"))
            })
            .is_ok_and(|p| {
                p.join("shims").join("scoop.cmd").is_file()
                    || p.join("shims").join("scoop.ps1").is_file()
            });
        Managers { winget, scoop }
    }
    #[cfg(not(windows))]
    {
        Managers::default()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Via {
    Winget,
    Scoop,
}

/// The command line that installs an alternative, silently.
pub fn install_command(alt: &Alternative, via: Via) -> Option<(String, Vec<String>)> {
    match via {
        Via::Winget => alt.winget.map(|id| {
            (
                "winget.exe".to_string(),
                [
                    "install",
                    "--id",
                    id,
                    "--exact",
                    "--silent",
                    "--accept-package-agreements",
                    "--accept-source-agreements",
                    "--disable-interactivity",
                ]
                .iter()
                .map(|s| s.to_string())
                .collect(),
            )
        }),
        Via::Scoop => alt.scoop.map(|app| {
            let bucket = app.split('/').next().unwrap_or("main");
            let script = if bucket == "main" {
                format!("scoop install {app}")
            } else {
                format!("scoop bucket add {bucket} 2>$null; scoop install {app}")
            };
            (
                "powershell.exe".to_string(),
                [
                    "-NoProfile",
                    "-NonInteractive",
                    "-ExecutionPolicy",
                    "Bypass",
                    "-Command",
                    &script,
                ]
                .iter()
                .map(|s| s.to_string())
                .collect(),
            )
        }),
    }
}

/// Install an alternative; returns the tool's output.
pub fn install(alt: &Alternative, via: Via) -> anyhow::Result<String> {
    let (program, args) = install_command(alt, via)
        .ok_or_else(|| anyhow::anyhow!("{} isn't available that way", alt.name))?;
    let refs: Vec<&str> = args.iter().map(String::as_str).collect();
    let (code, out) = crate::platform::run_hidden(&program, &refs)?;
    if code != 0 {
        let last: Vec<&str> = out
            .lines()
            .filter(|l| !l.trim().is_empty())
            .rev()
            .take(2)
            .collect();
        anyhow::bail!(
            "{} (code {code})",
            last.into_iter().rev().collect::<Vec<_>>().join(" ")
        );
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn suggests_alternatives() {
        let names: Vec<&str> = for_program("WinRAR 7.01 (64-bit)")
            .iter()
            .map(|a| a.name)
            .collect();
        assert_eq!(names, vec!["7-Zip", "NanaZip", "PeaZip"]);
        assert_eq!(for_program("CCleaner")[0].name, "BleachBit");
        assert!(for_program("VidConvert Pro 5")
            .iter()
            .any(|a| a.name == "HandBrake"));
        assert!(for_program("7-Zip 24.08").is_empty());
        // A program that is one of the alternatives isn't offered itself.
        assert!(for_program("Visual Studio Code")
            .iter()
            .all(|a| a.name != "Visual Studio Code"));
    }

    #[test]
    fn builds_install_commands() {
        let (p, args) = install_command(&SEVENZIP, Via::Winget).unwrap();
        assert_eq!(p, "winget.exe");
        assert!(args.windows(2).any(|w| w == ["--id", "7zip.7zip"]));
        let (p, args) = install_command(&QBITTORRENT, Via::Scoop).unwrap();
        assert_eq!(p, "powershell.exe");
        assert!(
            args.last().unwrap().contains("scoop bucket add extras")
                && args
                    .last()
                    .unwrap()
                    .contains("scoop install extras/qbittorrent")
        );
        assert!(install_command(&DEFENDER_NOTE, Via::Winget).is_none());
    }
}
