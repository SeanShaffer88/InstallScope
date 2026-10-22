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

/// A requested safety net must never silently disappear because rights are missing.
pub fn preflight_restore_point(requested: bool, elevated: bool) -> Result<bool> {
    if requested && !elevated {
        return Err(anyhow!("A requested System Restore point requires administrator rights. Restart SetupScope as administrator, or explicitly clear 'Create a System Restore point first' to continue without one. Uninstall canceled."));
    }
    Ok(requested)
}

fn creation_script(description: &str) -> String {
    let safe: String = description
        .chars()
        .filter(|c| c.is_alphanumeric() || " -_.()".contains(*c))
        .take(20)
        .collect();
    format!(
        "$ErrorActionPreference='Stop'; \
         $before=@(Get-ComputerRestorePoint -ErrorAction Stop | Select-Object -ExpandProperty SequenceNumber); \
         $k='Registry::{key}'; $name='SystemRestorePointCreationFrequency'; \
         $props=Get-ItemProperty -LiteralPath $k -ErrorAction Stop; \
         $property=$props.PSObject.Properties[$name]; $hadOld=$null -ne $property; $old=$null; \
         if ($hadOld) {{ $old=$property.Value; if ($old -isnot [int] -and $old -isnot [uint32]) {{ throw 'Restore-point frequency is not a DWORD; it was left unchanged.' }} }}; \
         $description='{safe} ['+[guid]::NewGuid().ToString('N')+']'; \
         New-ItemProperty -LiteralPath $k -Name $name -Value 0 -PropertyType DWord -Force -ErrorAction Stop | Out-Null; \
         try {{ \
             Checkpoint-Computer -Description $description -RestorePointType 'MODIFY_SETTINGS' -ErrorAction Stop; \
             $created=@(Get-ComputerRestorePoint -ErrorAction Stop | Where-Object {{ $_.SequenceNumber -notin $before -and $_.Description -ceq $description }}); \
             if ($created.Count -ne 1) {{ throw 'Windows did not report the requested new restore point. Operation canceled.' }} \
         }} finally {{ \
             if ($hadOld) {{ Set-ItemProperty -LiteralPath $k -Name $name -Value $old -ErrorAction Stop }} \
             else {{ Remove-ItemProperty -LiteralPath $k -Name $name -ErrorAction Stop }} \
         }}",
        key = SR_KEY.replace("HKLM", "HKEY_LOCAL_MACHINE"),
    )
}

/// Create and verify this request's uniquely named restore point. Temporarily
/// lift the frequency limit, restoring its previous DWORD or absence on exit.
pub fn create_restore_point(description: &str) -> Result<()> {
    preflight_restore_point(true, crate::platform::is_elevated())?;
    powershell(&creation_script(description))
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
    fn requested_restore_points_require_rights_but_explicit_opt_out_does_not() {
        assert!(preflight_restore_point(true, false).is_err());
        assert!(preflight_restore_point(true, true).unwrap());
        assert!(!preflight_restore_point(false, false).unwrap());
        assert!(!preflight_restore_point(false, true).unwrap());
    }

    #[cfg(windows)]
    #[test]
    fn creation_script_verifies_new_points_and_restores_frequency_on_failure() {
        // Run the production script in real Windows PowerShell with every
        // system-changing/provider command replaced by an in-process fake.
        // No real registry entry or restore point is read or changed.
        let mocks = r#"
function Get-ComputerRestorePoint {
    [CmdletBinding()] param()
    if ($script:mode -eq 'before-list-fails' -or ($script:mode -eq 'after-list-fails' -and $script:called)) { throw 'list failed' }
    $script:points
}
function Get-ItemProperty {
    [CmdletBinding()] param($LiteralPath)
    if ($script:hadOld) { [pscustomobject]@{SystemRestorePointCreationFrequency=$script:initial} }
    else { [pscustomobject]@{} }
}
function New-ItemProperty {
    [CmdletBinding()] param($LiteralPath,$Name,$Value,$PropertyType,[switch]$Force)
    if ($script:mode -eq 'write-fails') { throw 'write failed' }
    $script:frequency=$Value
}
function Set-ItemProperty {
    [CmdletBinding()] param($LiteralPath,$Name,$Value)
    if ($script:mode -eq 'cleanup-fails') { throw 'cleanup failed' }
    $script:frequency=$Value
}
function Remove-ItemProperty {
    [CmdletBinding()] param($LiteralPath,$Name)
    if ($script:mode -eq 'cleanup-fails') { throw 'cleanup failed' }
    $script:frequency=$null
}
function Checkpoint-Computer {
    [CmdletBinding()] param($Description,$RestorePointType)
    $script:called=$true
    if ($script:mode -eq 'checkpoint-fails') { throw 'creation failed' }
    if ($script:mode -eq 'warning-only') { Write-Warning 'skipped'; return }
    $id=8; $label=$Description
    if ($script:mode -eq 'old-sequence') { $id=7 }
    if ($script:mode -eq 'wrong-description') { $label='another request' }
    $script:points += [pscustomobject]@{SequenceNumber=$id;Description=$label}
}
"#;
        let production = creation_script("Before test ' $(throw 'injected')");
        for had_old in [false, true] {
            for mode in [
                "success",
                "before-list-fails",
                "after-list-fails",
                "write-fails",
                "checkpoint-fails",
                "warning-only",
                "old-sequence",
                "wrong-description",
                "cleanup-fails",
            ] {
                let script = format!(
                    "$script:mode='{mode}'; $script:hadOld=${had_old}; $script:initial=$null; if ($script:hadOld) {{ $script:initial=[int]1440 }}; $script:frequency=$script:initial; $script:called=$false; $script:points=@([pscustomobject]@{{SequenceNumber=7;Description='previous'}}); {mocks}\n $failed=$false; try {{ & {{ {production} }} }} catch {{ $failed=$true }}; if ($failed -ne ${should_fail}) {{ throw 'Incorrect success result' }}; if ($script:mode -ne 'cleanup-fails' -and $script:frequency -ne $script:initial) {{ throw 'Frequency was not restored' }}; if ($script:mode -in @('before-list-fails','write-fails') -and $script:called) {{ throw 'Creation ran after preflight failure' }}; 'verified'",
                    should_fail = mode != "success",
                );
                let (code, out) = crate::platform::run_hidden(
                    "powershell.exe",
                    &["-NoProfile", "-NonInteractive", "-Command", &script],
                )
                .unwrap();
                assert_eq!(code, 0, "mode={mode}, old={had_old}: {out}");
                assert!(out.contains("verified"), "{out}");
            }
        }
    }

    #[test]
    fn parses_restore_points() {
        let one = r#"{"SequenceNumber":7,"Description":"Before SetupScope uninstall","CreationTime":"20261015083012.000000-000","RestorePointType":12}"#;
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
