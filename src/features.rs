//! Optional Windows features (turned on or off through DISM) and App
//! Execution Aliases (the `python.exe`, `winget.exe`… commands that Store
//! apps add to `%LOCALAPPDATA%\Microsoft\WindowsApps`).

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Feature {
    pub name: String,
    pub enabled: bool,
    /// "Enable Pending", "Disable Pending" (waiting for a restart), etc.
    pub state: String,
}
crate::json_struct!(Feature, { name, enabled, state });

impl Feature {
    pub fn pending(&self) -> bool {
        self.state.to_lowercase().contains("pending")
    }

    /// A friendlier name for well-known features.
    pub fn display_name(&self) -> &str {
        friendly_name(&self.name).unwrap_or(&self.name)
    }
}

pub fn friendly_name(name: &str) -> Option<&'static str> {
    Some(match name {
        "Microsoft-Windows-Subsystem-Linux" => "Windows Subsystem for Linux",
        "VirtualMachinePlatform" => "Virtual Machine Platform",
        "HypervisorPlatform" => "Windows Hypervisor Platform",
        "Microsoft-Hyper-V-All" => "Hyper-V",
        "Containers-DisposableClientVM" => "Windows Sandbox",
        "Containers" => "Containers",
        "NetFx3" => ".NET Framework 3.5 (includes .NET 2.0 and 3.0)",
        "NetFx4-AdvSrvs" => ".NET Framework 4 Advanced Services",
        "TelnetClient" => "Telnet Client",
        "TFTP" => "TFTP Client",
        "SMB1Protocol" => "SMB 1.0/CIFS File Sharing Support",
        "IIS-WebServerRole" => "Internet Information Services",
        "MediaPlayback" => "Media Features",
        "WindowsMediaPlayer" => "Windows Media Player Legacy",
        "LegacyComponents" => "Legacy Components",
        "DirectPlay" => "DirectPlay",
        "Printing-PrintToPDFServices-Features" => "Microsoft Print to PDF",
        "Printing-XPSServices-Features" => "Microsoft XPS Document Writer",
        "Printing-Foundation-Features" => "Print and Document Services",
        "WorkFolders-Client" => "Work Folders Client",
        "SearchEngine-Client-Package" => "Windows Search",
        "MSRDC-Infrastructure" => "Remote Differential Compression API Support",
        "Windows-Defender-ApplicationGuard" => "Microsoft Defender Application Guard",
        "Microsoft-Windows-Client-EmbeddedExp-Package" => "Device Lockdown",
        "Client-ProjFS" => "Windows Projected File System",
        "Recall" => "Recall",
        _ => return None,
    })
}

/// Parse `dism /English /Online /Get-Features /Format:Table`.
pub fn parse_features(output: &str) -> Vec<Feature> {
    let mut out = Vec::new();
    let mut in_table = false;
    // Rows since the last rule line; a second rule means they were the header.
    let mut since_rule = 0usize;
    for line in output.lines() {
        let l = line.trim();
        if l.starts_with("---") && l.contains('|') {
            out.truncate(out.len() - since_rule);
            since_rule = 0;
            in_table = true;
            continue;
        }
        if !in_table {
            continue;
        }
        let Some((name, state)) = l.split_once('|') else {
            if l.is_empty() {
                continue;
            }
            break;
        };
        let (name, state) = (name.trim(), state.trim());
        if name.is_empty() {
            continue;
        }
        since_rule += 1;
        out.push(Feature {
            name: name.to_string(),
            enabled: state.eq_ignore_ascii_case("Enabled")
                || state.eq_ignore_ascii_case("Enable Pending"),
            state: state.to_string(),
        });
    }
    out.sort_by_key(|f| f.display_name().to_lowercase());
    out
}

pub fn list_features() -> anyhow::Result<Vec<Feature>> {
    if !cfg!(windows) && std::env::var_os("SETUPSCOPE_DEMO").is_some() {
        return Ok(parse_features(
            "---- | ----\nFeature Name | State\n---- | ----\n\
Microsoft-Windows-Subsystem-Linux | Enabled\nVirtualMachinePlatform | Enabled\n\
Containers-DisposableClientVM | Disabled\nNetFx3 | Enable Pending\nTelnetClient | Disabled\n\
Printing-PrintToPDFServices-Features | Enabled\nSMB1Protocol | Disabled\n",
        ));
    }
    let (code, out) = crate::platform::run_hidden(
        "dism.exe",
        &["/English", "/Online", "/Get-Features", "/Format:Table"],
    )?;
    let list = parse_features(&out);
    if list.is_empty() {
        anyhow::bail!(
            "DISM exit code {code}: {}",
            out.lines()
                .rev()
                .find(|l| !l.trim().is_empty())
                .unwrap_or("")
        );
    }
    Ok(list)
}

/// Turn a feature on or off. Returns true when Windows must restart to finish.
pub fn set_feature(name: &str, enable: bool) -> anyhow::Result<bool> {
    let arg = format!("/FeatureName:{name}");
    let mut args = vec!["/English", "/Online"];
    if enable {
        args.extend(["/Enable-Feature", &arg, "/All", "/NoRestart"]);
    } else {
        args.extend(["/Disable-Feature", &arg, "/NoRestart"]);
    }
    let (code, out) = crate::platform::run_hidden("dism.exe", &args)?;
    match code {
        0 => Ok(false),
        3010 => Ok(true),
        _ => anyhow::bail!(
            "DISM exit code {code}: {}",
            out.lines()
                .rev()
                .find(|l| !l.trim().is_empty())
                .unwrap_or("")
        ),
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Alias {
    /// The command, e.g. `python.exe`.
    pub command: String,
    /// The app package that provides it, when known.
    pub package: String,
}
crate::json_struct!(Alias, { command, package });

/// List App Execution Aliases that are switched on. Each one is a small
/// link file in the WindowsApps folder; the app it belongs to keeps a copy
/// in a folder named after its package.
pub fn list_aliases_in(dir: &std::path::Path) -> Vec<Alias> {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut files = Vec::new();
    let mut packages = Vec::new();
    for e in rd.flatten() {
        let name = e.file_name().to_string_lossy().to_string();
        let is_dir = e.file_type().is_ok_and(|t| t.is_dir());
        if is_dir {
            packages.push(name);
        } else if name.to_lowercase().ends_with(".exe") {
            files.push(name);
        }
    }
    let mut out: Vec<Alias> = files
        .into_iter()
        .map(|command| {
            let package = packages
                .iter()
                .find(|p| dir.join(p).join(&command).exists())
                .map(|p| p.split('_').next().unwrap_or(p).to_string())
                .unwrap_or_default();
            Alias { command, package }
        })
        .collect();
    out.sort_by_key(|a| a.command.to_lowercase());
    out
}

pub fn list_aliases() -> Vec<Alias> {
    let dir = crate::platform::expand_env("%LOCALAPPDATA%\\Microsoft\\WindowsApps");
    if dir.contains('%') {
        return Vec::new();
    }
    list_aliases_in(std::path::Path::new(&dir))
}

/// Windows Settings page where aliases are switched on and off.
pub const ALIAS_SETTINGS_URI: &str = "ms-settings:advanced-apps";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_dism_table() {
        let out = "\nDeployment Image Servicing and Management tool\nVersion: 10.0.26100.1\n\n\
Image Version: 10.0.26100.2033\n\nFeatures listing for package : Microsoft-Windows-Foundation-Package~31bf3856ad364e35~amd64~~10.0.26100.1\n\n\
------------------------------------------- | --------\n\
Feature Name                                | State\n\
------------------------------------------- | --------\n\
Printing-PrintToPDFServices-Features        | Enabled\n\
Microsoft-Windows-Subsystem-Linux           | Disabled\n\
NetFx3                                      | Enable Pending\n\
\n\
The operation completed successfully.\n";
        let f = parse_features(out);
        // "Feature Name" header row is skipped because it follows the first rule.
        let names: Vec<&str> = f.iter().map(|x| x.name.as_str()).collect();
        assert!(
            names.contains(&"Microsoft-Windows-Subsystem-Linux"),
            "{names:?}"
        );
        assert!(
            !names.iter().any(|n| n.starts_with("Feature Name")),
            "{names:?}"
        );
        let wsl = f
            .iter()
            .find(|x| x.name == "Microsoft-Windows-Subsystem-Linux")
            .unwrap();
        assert!(!wsl.enabled);
        assert_eq!(wsl.display_name(), "Windows Subsystem for Linux");
        let net = f.iter().find(|x| x.name == "NetFx3").unwrap();
        assert!(net.enabled && net.pending());
    }

    #[test]
    fn lists_aliases() {
        let dir = crate::util::tempdir().unwrap();
        let pkg = dir
            .path()
            .join("PythonSoftwareFoundation.Python.3.13_qbz5n2kfra8p0");
        std::fs::create_dir(&pkg).unwrap();
        std::fs::write(pkg.join("python.exe"), b"x").unwrap();
        std::fs::write(dir.path().join("python.exe"), b"x").unwrap();
        std::fs::write(dir.path().join("winget.exe"), b"x").unwrap();
        let a = list_aliases_in(dir.path());
        assert_eq!(a.len(), 2);
        assert_eq!(a[0].command, "python.exe");
        assert_eq!(a[0].package, "PythonSoftwareFoundation.Python.3.13");
        assert_eq!(a[1].package, "");
    }
}
