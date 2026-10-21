//! System Restore: list, create, delete and roll back to restore points, and
//! manage System Protection (on/off and how much disk space it may use).

use anyhow::{anyhow, Result};

#[derive(Clone, Debug, PartialEq)]
pub struct RestorePoint {
    pub sequence: u32,
    pub description: String,
    /// Local time, "YYYY-MM-DD HH:MM".
    pub created: String,
    pub kind: Kind,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Install,
    Uninstall,
    Manual,
    Update,
    Other,
}

impl Kind {
    fn from_type(t: u32) -> Self {
        match t {
            0 => Kind::Install,
            1 => Kind::Uninstall,
            12 => Kind::Manual,
            17 | 18 => Kind::Update,
            _ => Kind::Other,
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Kind::Install => "Program installed",
            Kind::Uninstall => "Program removed",
            Kind::Manual => "Created manually",
            Kind::Update => "Windows update",
            Kind::Other => "System",
        }
    }
}

fn powershell(script: &str) -> Result<String> {
    let (code, out) = crate::platform::run_hidden(
        "powershell.exe",
        &[
            "-NoProfile",
            "-NonInteractive",
            "-ExecutionPolicy",
            "Bypass",
            "-Command",
            script,
        ],
    )?;
    if code != 0 {
        return Err(anyhow!("{}", out.trim()));
    }
    Ok(out)
}

/// One restore point as PowerShell's `ConvertTo-Json` writes it
/// (PascalCase names).
struct RawPoint {
    sequence_number: u32,
    description: String,
    creation_time: String,
    restore_point_type: u32,
}

impl crate::json::FromJson for RawPoint {
    fn from_json(v: &crate::json::Value) -> Result<Self, crate::json::Error> {
        let Some(obj) = v.as_object() else {
            return crate::json::type_error("a restore point");
        };
        let opt = |name: &str| crate::json::field(obj, name);
        Ok(RawPoint {
            sequence_number: match opt("SequenceNumber") {
                Some(x) => u32::from_json(x)?,
                None => return crate::json::type_error("SequenceNumber"),
            },
            description: opt("Description")
                .map(String::from_json)
                .transpose()?
                .unwrap_or_default(),
            creation_time: opt("CreationTime")
                .map(String::from_json)
                .transpose()?
                .unwrap_or_default(),
            restore_point_type: opt("RestorePointType")
                .map(u32::from_json)
                .transpose()?
                .unwrap_or_default(),
        })
    }
}

/// `20261015083012.000000-000` → `2026-10-15 08:30`.
fn wmi_time(s: &str) -> String {
    if s.len() >= 12 && s[..12].chars().all(|c| c.is_ascii_digit()) {
        format!(
            "{}-{}-{} {}:{}",
            &s[0..4],
            &s[4..6],
            &s[6..8],
            &s[8..10],
            &s[10..12]
        )
    } else {
        s.to_string()
    }
}

/// Parse `Get-ComputerRestorePoint | ConvertTo-Json` (one object or an array).
pub fn parse_points(json: &str) -> Vec<RestorePoint> {
    let json = json.trim();
    if json.is_empty() {
        return Vec::new();
    }
    let raw: Vec<RawPoint> = if json.starts_with('[') {
        crate::json::compat::from_str(json).unwrap_or_default()
    } else {
        crate::json::compat::from_str::<RawPoint>(json)
            .map(|p| vec![p])
            .unwrap_or_default()
    };
    let mut v: Vec<RestorePoint> = raw
        .into_iter()
        .map(|r| RestorePoint {
            sequence: r.sequence_number,
            description: r.description,
            created: wmi_time(&r.creation_time),
            kind: Kind::from_type(r.restore_point_type),
        })
        .collect();
    v.sort_by_key(|p| std::cmp::Reverse(p.sequence));
    v
}

/// All restore points, newest first (needs administrator rights).
pub fn list() -> Result<Vec<RestorePoint>> {
    let out = powershell(
        "Get-ComputerRestorePoint | Select-Object SequenceNumber,Description,CreationTime,RestorePointType | ConvertTo-Json -Compress",
    )?;
    Ok(parse_points(&out))
}

const SR_KEY: &str = "HKLM\\SOFTWARE\\Microsoft\\Windows NT\\CurrentVersion\\SystemRestore";

/// Create a restore point now. Windows normally allows only one every 24
/// hours; SetupSweep lifts that limit for its own request and puts it back.
pub fn create_restore_point(description: &str) -> Result<()> {
    let safe: String = description
        .chars()
        .filter(|c| c.is_alphanumeric() || " -_.()".contains(*c))
        .take(60)
        .collect();
    let script = format!(
        "$k='{key}'; $old=(Get-ItemProperty -Path \"Registry::$k\" -Name SystemRestorePointCreationFrequency -ErrorAction SilentlyContinue).SystemRestorePointCreationFrequency; \
         New-ItemProperty -Path \"Registry::$k\" -Name SystemRestorePointCreationFrequency -Value 0 -PropertyType DWord -Force | Out-Null; \
         try {{ Checkpoint-Computer -Description '{safe}' -RestorePointType 'MODIFY_SETTINGS' -ErrorAction Stop }} \
         finally {{ if ($null -eq $old) {{ Remove-ItemProperty -Path \"Registry::$k\" -Name SystemRestorePointCreationFrequency -ErrorAction SilentlyContinue }} else {{ Set-ItemProperty -Path \"Registry::$k\" -Name SystemRestorePointCreationFrequency -Value $old }} }}",
        key = SR_KEY.replace("HKLM", "HKEY_LOCAL_MACHINE"),
    );
    powershell(&script)
        .map(|_| ())
        .map_err(|e| anyhow!("restore point failed: {e}"))
}

/// Delete one restore point.
pub fn delete(sequence: u32) -> Result<()> {
    #[cfg(windows)]
    {
        crate::platform::windows::remove_restore_point(sequence)
    }
    #[cfg(not(windows))]
    {
        let _ = sequence;
        Err(anyhow!("System Restore is only available on Windows"))
    }
}

/// Roll Windows back to a restore point. Windows restarts immediately.
pub fn restore_to(sequence: u32) -> Result<()> {
    powershell(&format!(
        "Restore-Computer -RestorePoint {sequence} -Confirm:$false"
    ))
    .map(|_| ())
}

/// Open Windows' own System Restore wizard.
pub fn open_wizard() {
    let _ = crate::platform::spawn_command_line("rstrui.exe");
}

/// System Protection on the system drive, and its disk space.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Protection {
    pub enabled: bool,
    pub used: Option<u64>,
    pub max: Option<u64>,
    /// Size of the system drive, to show the limit as a percentage.
    pub drive_size: Option<u64>,
}

/// Parse a size like "2.31 GB (2%)" from vssadmin.
fn vss_size(s: &str) -> Option<u64> {
    let s = s.trim();
    let mut it = s.split_whitespace();
    let n: f64 = it.next()?.replace(',', ".").parse().ok()?;
    let unit = it.next()?.to_ascii_uppercase();
    let mult = match unit.as_str() {
        "B" | "BYTES" => 1.0,
        "KB" => 1024.0,
        "MB" => 1024.0 * 1024.0,
        "GB" => 1024.0 * 1024.0 * 1024.0,
        "TB" => 1024.0 * 1024.0 * 1024.0 * 1024.0,
        _ => return None,
    };
    Some((n * mult) as u64)
}

/// Parse `vssadmin list shadowstorage` for the given drive ("C:").
pub fn parse_shadowstorage(text: &str, drive: &str) -> (Option<u64>, Option<u64>) {
    let drive = drive.to_ascii_uppercase();
    for block in text.split("Shadow Copy Storage association") {
        let for_drive = block.lines().any(|l| {
            l.trim_start().starts_with("For volume:")
                && l.to_ascii_uppercase().contains(&format!("({drive})"))
        });
        if !for_drive {
            continue;
        }
        let field = |name: &str| {
            block
                .lines()
                .find(|l| l.trim_start().starts_with(name))
                .and_then(|l| l.split_once(':'))
                .and_then(|(_, v)| vss_size(v))
        };
        return (
            field("Used Shadow Copy Storage space"),
            field("Maximum Shadow Copy Storage space"),
        );
    }
    (None, None)
}

pub fn protection() -> Protection {
    let drive = std::env::var("SystemDrive").unwrap_or_else(|_| "C:".into());
    let mut p = Protection::default();
    // Volumes with protection on are listed under SPP\Clients.
    if let Ok(out) = powershell(
        "(Get-ItemProperty 'HKLM:\\SOFTWARE\\Microsoft\\Windows NT\\CurrentVersion\\SPP\\Clients' -ErrorAction SilentlyContinue).'{09F7EDC5-294E-4180-AF6A-FB0E6A0E9513}' -join \"`n\"",
    ) {
        let enc = format!("({}", drive.replace(':', "%3A"));
        p.enabled = out.to_ascii_uppercase().contains(&enc.to_ascii_uppercase());
    }
    if let Ok((0, out)) = crate::platform::run_hidden("vssadmin", &["list", "shadowstorage"]) {
        let (used, max) = parse_shadowstorage(&out, &drive);
        p.used = used;
        p.max = max;
    }
    #[cfg(windows)]
    {
        p.drive_size = crate::platform::windows::drive_size(&format!("{drive}\\"));
    }
    p
}

pub fn set_protection(on: bool) -> Result<()> {
    let drive = std::env::var("SystemDrive").unwrap_or_else(|_| "C:".into());
    let cmd = if on {
        "Enable-ComputerRestore"
    } else {
        "Disable-ComputerRestore"
    };
    powershell(&format!("{cmd} -Drive '{drive}\\'")).map(|_| ())
}

/// Let System Restore use up to `percent` of the system drive.
pub fn set_max_space(percent: u32) -> Result<()> {
    let drive = std::env::var("SystemDrive").unwrap_or_else(|_| "C:".into());
    let pct = percent.clamp(1, 100);
    let (code, out) = crate::platform::run_hidden(
        "vssadmin",
        &[
            "resize",
            "shadowstorage",
            &format!("/For={drive}"),
            &format!("/On={drive}"),
            &format!("/MaxSize={pct}%"),
        ],
    )?;
    if code != 0 {
        return Err(anyhow!("{}", out.trim()));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_restore_points() {
        let one = r#"{"SequenceNumber":7,"Description":"Before SetupSweep uninstall","CreationTime":"20261015083012.000000-000","RestorePointType":12}"#;
        let p = parse_points(one);
        assert_eq!(p.len(), 1);
        assert_eq!(p[0].created, "2026-10-15 08:30");
        assert_eq!(p[0].kind, Kind::Manual);
        let many = r#"[{"SequenceNumber":3,"Description":"Windows Update","CreationTime":"20261001120000.000000-000","RestorePointType":18},{"SequenceNumber":9,"Description":"Installed Acme","CreationTime":"20261012090000.000000-000","RestorePointType":0}]"#;
        let p = parse_points(many);
        assert_eq!(p.iter().map(|x| x.sequence).collect::<Vec<_>>(), vec![9, 3]);
        assert_eq!(p[1].kind, Kind::Update);
        assert!(parse_points("").is_empty());
    }

    #[test]
    fn parses_shadow_storage() {
        let text = "vssadmin 1.1\n\nShadow Copy Storage association\n   For volume: (C:)\\\\?\\Volume{abc}\\\n   Shadow Copy Storage volume: (C:)\\\\?\\Volume{abc}\\\n   Used Shadow Copy Storage space: 2.31 GB (1%)\n   Allocated Shadow Copy Storage space: 2.75 GB (1%)\n   Maximum Shadow Copy Storage space: 23.8 GB (10%)\n";
        let (used, max) = parse_shadowstorage(text, "C:");
        assert_eq!(used, Some((2.31 * 1073741824.0) as u64));
        assert_eq!(max, Some((23.8 * 1073741824.0) as u64));
        assert_eq!(parse_shadowstorage(text, "D:"), (None, None));
    }
}
