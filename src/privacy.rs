//! Small catalog of documented Windows privacy settings.
//!
//! These controls change one registry value at a time. They do not disable
//! Windows services, block Microsoft endpoints, or claim to stop all data
//! collection.

use crate::model::{reg_type, RegValue};
use crate::platform::Registry;
use anyhow::Context;
use std::path::Path;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Control {
    AdvertisingId,
    TailoredExperiences,
    TypingPersonalization,
    RecallSnapshots,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    Limited,
    Allowed,
    Unconfigured,
    Mixed,
    Unrecognized,
}

#[derive(Clone, Copy, Debug)]
pub struct Definition {
    pub control: Control,
    pub title: &'static str,
    pub effect: &'static str,
    pub tradeoff: &'static str,
    pub key: &'static str,
    pub value_names: &'static [&'static str],
    pub limited_value: u32,
    pub requires_admin: bool,
    /// A policy with irreversible effects beyond the registry value itself.
    pub warning: Option<&'static str>,
}

pub const CONTROLS: [Definition; 4] = [
    Definition {
        control: Control::AdvertisingId,
        title: "Advertising ID",
        effect: "Prevents Windows apps from using the Windows advertising ID for personalized ads.",
        tradeoff: "Apps may show less personalized ads. This does not affect other advertising identifiers or app-specific tracking.",
        key: "HKLM\\SOFTWARE\\Policies\\Microsoft\\Windows\\AdvertisingInfo",
        value_names: &["DisabledByGroupPolicy"],
        limited_value: 1,
        requires_admin: true,
        warning: None,
    },
    Definition {
        control: Control::TailoredExperiences,
        title: "Recommendations based on diagnostic data",
        effect: "Stops Windows using diagnostic data to tailor tips, recommendations and offers.",
        tradeoff: "Some Windows tips and recommendations may be less relevant. Required diagnostic data is not disabled.",
        key: "HKCU\\Software\\Policies\\Microsoft\\Windows\\CloudContent",
        value_names: &["DisableTailoredExperiencesWithDiagnosticData"],
        limited_value: 1,
        requires_admin: false,
        warning: None,
    },
    Definition {
        control: Control::TypingPersonalization,
        title: "Typing and inking personalization",
        effect: "Restricts Windows from collecting typing and inking data to personalize recognition and suggestions.",
        tradeoff: "Personalized handwriting recognition and typing suggestions may be less accurate.",
        key: "HKCU\\Software\\Microsoft\\InputPersonalization",
        value_names: &["RestrictImplicitTextCollection", "RestrictImplicitInkCollection"],
        limited_value: 1,
        requires_admin: false,
        warning: None,
    },
    Definition {
        control: Control::RecallSnapshots,
        title: "Recall screen snapshots",
        effect: "Prevents Windows Recall from saving screen snapshots for this account.",
        tradeoff: "Recall cannot search past screen activity while snapshot saving is disabled.",
        key: "HKCU\\Software\\Policies\\Microsoft\\Windows\\WindowsAI",
        value_names: &["DisableAIDataAnalysis"],
        limited_value: 1,
        requires_admin: false,
        warning: Some("Enabling this policy deletes previously saved Recall snapshots. SetupSweep backs up the policy value, not snapshot contents; clearing the policy cannot restore deleted snapshots."),
    },
];

const WINDOWS_VERSION_KEY: &str = "HKLM\\SOFTWARE\\Microsoft\\Windows NT\\CurrentVersion";
const RECALL_MIN_BUILD: u32 = 26_100;
const RECALL_MIN_UBR: u32 = 3_915;

/// The Recall snapshot policy is documented for supported editions on
/// Windows 11 24H2 build 26100.3915 and later. Unknown profiles fail closed.
pub fn recall_policy_supported(reg: &dyn Registry) -> bool {
    let Some(version) = reg.read_key(WINDOWS_VERSION_KEY) else {
        return false;
    };
    let Some(build) = version
        .get("CurrentBuildNumber")
        .and_then(RegValue::as_string)
        .and_then(|value| value.parse::<u32>().ok())
    else {
        return false;
    };
    let Some(ubr) = version.get("UBR").and_then(RegValue::as_u32) else {
        return false;
    };
    let Some(edition) = version.get("EditionID").and_then(RegValue::as_string) else {
        return false;
    };
    let edition = edition.trim().to_ascii_lowercase();
    let supported_edition = matches!(
        edition.as_str(),
        "professional"
            | "professionaln"
            | "professionalworkstation"
            | "professionalworkstationn"
            | "professionaleducation"
            | "professionaleducationn"
            | "enterprise"
            | "enterprises"
            | "education"
            | "educationn"
            | "iotenterprise"
            | "iotenterprises"
    );
    let supported_build =
        build > RECALL_MIN_BUILD || (build == RECALL_MIN_BUILD && ubr >= RECALL_MIN_UBR);
    supported_build && supported_edition
}

impl Definition {
    pub fn state(&self, reg: &dyn Registry) -> State {
        let key = reg.read_key(self.key);
        let values: Vec<_> = self
            .value_names
            .iter()
            .map(|name| key.as_ref().and_then(|key| key.get(name).cloned()))
            .collect();
        if values.iter().all(Option::is_none) {
            return State::Unconfigured;
        };
        if values
            .iter()
            .flatten()
            .any(|value| value.kind != reg_type::DWORD || value.as_u32().is_none_or(|n| n > 1))
        {
            return State::Unrecognized;
        }
        if values
            .iter()
            .all(|value| value.as_ref().and_then(RegValue::as_u32) == Some(self.limited_value))
        {
            State::Limited
        } else if values
            .iter()
            .all(|value| value.as_ref().and_then(RegValue::as_u32) == Some(0))
        {
            State::Allowed
        } else {
            State::Mixed
        }
    }

    pub fn limited_value(&self, value_name: &str) -> RegValue {
        RegValue::dword(value_name, self.limited_value)
    }
}

/// Apply or clear a single catalogued value, recording its prior state first.
/// The caller handles confirmation and administrator checks before calling.
pub fn apply_with_backup(
    reg: &dyn Registry,
    backups_root: &Path,
    control: Control,
    limit: bool,
) -> anyhow::Result<bool> {
    let Some(definition) = CONTROLS
        .iter()
        .find(|definition| definition.control == control)
    else {
        anyhow::bail!("privacy setting is not present in the reviewed catalog");
    };
    if limit && control == Control::RecallSnapshots && !recall_policy_supported(reg) {
        anyhow::bail!("Recall snapshot policy is unsupported on this Windows version or edition");
    }
    let state = definition.state(reg);
    if limit && state == State::Limited {
        return Ok(false);
    }
    if !limit && state == State::Unconfigured {
        return Ok(false);
    }

    let mut backup = crate::backup::BackupWriter::new(
        backups_root,
        &format!("Windows privacy: {}", definition.title),
    )?;
    if !reg.exists(definition.key) {
        backup.add_key_absent(definition.key)?;
    }
    for value_name in definition.value_names {
        backup.add_value(reg, definition.key, value_name)?;
    }
    for value_name in definition.value_names {
        if limit {
            reg.set_value(definition.key, &definition.limited_value(value_name))?;
        } else if reg
            .read_key(definition.key)
            .and_then(|key| key.get(value_name).cloned())
            .is_some()
        {
            reg.delete_value(definition.key, value_name)?;
        }
    }
    backup
        .finish()
        .context("the registry value changed, but the backup could not be finalized")?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::KeyData;
    use crate::platform::MemRegistry;

    #[test]
    fn controls_report_unconfigured_allowed_limited_and_unknown_values() {
        let reg = MemRegistry::new();
        let control = &CONTROLS[1];
        assert_eq!(control.state(&reg), State::Unconfigured);

        let value_name = control.value_names[0];
        reg.put(KeyData::new(control.key).with(RegValue::dword(value_name, 0)));
        assert_eq!(control.state(&reg), State::Allowed);

        reg.set_value(control.key, &control.limited_value(value_name))
            .unwrap();
        assert_eq!(control.state(&reg), State::Limited);

        reg.set_value(control.key, &RegValue::string(value_name, "1"))
            .unwrap();
        assert_eq!(control.state(&reg), State::Unrecognized);

        reg.set_value(control.key, &RegValue::dword(value_name, 9))
            .unwrap();
        assert_eq!(control.state(&reg), State::Unrecognized);
    }

    #[test]
    fn catalog_changes_only_the_documented_user_or_machine_value() {
        assert!(CONTROLS[0].key.starts_with("HKLM\\"));
        assert!(CONTROLS[0].requires_admin);
        assert!(CONTROLS[1..]
            .iter()
            .all(|item| item.key.starts_with("HKCU\\")));
        assert!(CONTROLS[1..].iter().all(|item| !item.requires_admin));
        assert_eq!(CONTROLS[2].value_names.len(), 2);
        assert_eq!(CONTROLS[3].control, Control::RecallSnapshots);
        assert!(CONTROLS[3].warning.is_some());
    }

    fn windows_version(build: &str, ubr: u32, edition: &str) -> KeyData {
        KeyData::new(WINDOWS_VERSION_KEY)
            .with(RegValue::string("CurrentBuildNumber", build))
            .with(RegValue::dword("UBR", ubr))
            .with(RegValue::string("EditionID", edition))
    }

    #[test]
    fn recall_policy_is_gated_by_documented_build_update_and_supported_edition() {
        let reg = MemRegistry::new();
        assert!(!recall_policy_supported(&reg));

        reg.put(windows_version("26100", 3914, "Professional"));
        assert!(!recall_policy_supported(&reg));
        reg.put(windows_version("26100", 3915, "Core"));
        assert!(!recall_policy_supported(&reg));
        reg.put(windows_version("26100", 3915, "Professional"));
        assert!(recall_policy_supported(&reg));
        reg.put(windows_version("26200", 0, "Education"));
        assert!(recall_policy_supported(&reg));
        reg.put(windows_version("not-a-build", 5000, "Enterprise"));
        assert!(!recall_policy_supported(&reg));
    }

    #[test]
    fn recall_snapshot_policy_is_backed_up_and_refuses_unsupported_systems() {
        let tmp = crate::util::tempdir().unwrap();
        let backups = tmp.path().join("backups");
        let reg = MemRegistry::new();
        let definition = &CONTROLS[3];
        assert!(apply_with_backup(&reg, &backups, definition.control, true).is_err());
        assert!(!reg.exists(definition.key));
        assert!(!backups.exists());

        reg.put(windows_version("26100", 3915, "Professional"));
        reg.set_value(
            definition.key,
            &RegValue::dword(definition.value_names[0], 0),
        )
        .unwrap();
        assert!(apply_with_backup(&reg, &backups, definition.control, true).unwrap());
        assert_eq!(definition.state(&reg), State::Limited);

        let id = std::fs::read_dir(&backups)
            .unwrap()
            .next()
            .unwrap()
            .unwrap()
            .file_name()
            .to_string_lossy()
            .to_string();
        let restored = crate::backup::restore_backup(&backups, &id, &reg).unwrap();
        assert!(restored.iter().all(|row| row.ok));
        assert_eq!(definition.state(&reg), State::Allowed);
    }

    #[test]
    fn typing_and_inking_control_repairs_mixed_values_and_restores_both() {
        let tmp = crate::util::tempdir().unwrap();
        let backups = tmp.path().join("backups");
        let reg = MemRegistry::new();
        let definition = &CONTROLS[2];
        reg.put(KeyData::new(definition.key).with(RegValue::dword(definition.value_names[0], 1)));
        assert_eq!(definition.state(&reg), State::Mixed);

        assert!(apply_with_backup(&reg, &backups, definition.control, true).unwrap());
        assert_eq!(definition.state(&reg), State::Limited);
        let id = std::fs::read_dir(&backups)
            .unwrap()
            .next()
            .unwrap()
            .unwrap()
            .file_name()
            .to_string_lossy()
            .to_string();
        let restored = crate::backup::restore_backup(&backups, &id, &reg).unwrap();
        assert!(restored.iter().all(|row| row.ok));
        assert_eq!(definition.state(&reg), State::Mixed);
        assert_eq!(
            reg.read_key(definition.key)
                .unwrap()
                .get(definition.value_names[1]),
            None
        );
    }

    #[test]
    fn apply_always_creates_a_reversible_backup_of_the_exact_previous_value() {
        let tmp = crate::util::tempdir().unwrap();
        let backups = tmp.path().join("backups");
        let reg = MemRegistry::new();
        let definition = &CONTROLS[1];
        reg.put(KeyData::new(definition.key).with(RegValue::dword(definition.value_names[0], 0)));

        apply_with_backup(&reg, &backups, definition.control, true).unwrap();
        assert_eq!(definition.state(&reg), State::Limited);
        let manifest = std::fs::read_dir(&backups)
            .unwrap()
            .next()
            .unwrap()
            .unwrap()
            .file_name()
            .to_string_lossy()
            .to_string();
        let restored = crate::backup::restore_backup(&backups, &manifest, &reg).unwrap();
        assert!(restored.iter().all(|row| row.ok));
        assert_eq!(definition.state(&reg), State::Allowed);
    }

    #[test]
    fn clearing_an_override_is_backed_up_and_restores_the_value() {
        let tmp = crate::util::tempdir().unwrap();
        let backups = tmp.path().join("backups");
        let reg = MemRegistry::new();
        let definition = &CONTROLS[2];
        for value_name in definition.value_names {
            reg.set_value(definition.key, &RegValue::dword(value_name, 1))
                .unwrap();
        }

        apply_with_backup(&reg, &backups, definition.control, false).unwrap();
        assert_eq!(definition.state(&reg), State::Unconfigured);
        let manifest = std::fs::read_dir(&backups)
            .unwrap()
            .next()
            .unwrap()
            .unwrap()
            .file_name()
            .to_string_lossy()
            .to_string();
        let restored = crate::backup::restore_backup(&backups, &manifest, &reg).unwrap();
        assert!(restored.iter().all(|row| row.ok));
        assert_eq!(definition.state(&reg), State::Limited);
    }

    #[test]
    fn restoring_a_value_also_removes_a_key_created_for_the_setting() {
        let tmp = crate::util::tempdir().unwrap();
        let backups = tmp.path().join("backups");
        let reg = MemRegistry::new();
        let definition = &CONTROLS[1];
        assert!(!reg.exists(definition.key));

        assert!(apply_with_backup(&reg, &backups, definition.control, true).unwrap());
        assert!(reg.exists(definition.key));
        let id = std::fs::read_dir(&backups)
            .unwrap()
            .next()
            .unwrap()
            .unwrap()
            .file_name()
            .to_string_lossy()
            .to_string();
        let restored = crate::backup::restore_backup(&backups, &id, &reg).unwrap();
        assert!(restored.iter().all(|row| row.ok));
        assert!(!reg.exists(definition.key));
    }

    #[test]
    fn no_op_and_backup_failure_never_write_a_privacy_value() {
        let tmp = crate::util::tempdir().unwrap();
        let backups = tmp.path().join("backups");
        let reg = MemRegistry::new();
        let definition = &CONTROLS[0];

        assert!(!apply_with_backup(&reg, &backups, definition.control, false).unwrap());
        assert!(!backups.exists());

        std::fs::write(&backups, "not a directory").unwrap();
        assert!(apply_with_backup(&reg, &backups, definition.control, true).is_err());
        assert_eq!(definition.state(&reg), State::Unconfigured);
    }
}
