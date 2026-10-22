//! Every persisted type must write exactly what earlier versions wrote and
//! read their files back. The golden files in `testdata/json` hold serde's
//! output for these samples, captured before the switch.

use crate::json::{from_json_str, to_json_string, to_json_string_pretty, FromJson, ToJson};

macro_rules! golden {
    ($name:literal) => {
        (
            include_str!(concat!("testdata/json/", $name, ".json")),
            include_str!(concat!("testdata/json/", $name, ".pretty.json")),
        )
    };
}

fn check<T: ToJson + FromJson>(v: &T, (compact, pretty): (&str, &str)) {
    let name = std::any::type_name::<T>();
    assert_eq!(to_json_string(v), compact, "{name}");
    assert_eq!(to_json_string_pretty(v), pretty, "{name}");
    // The old files read back to the same thing.
    for text in [compact, pretty] {
        let back: T = from_json_str(text).unwrap_or_else(|e| panic!("{name}: {e}"));
        assert_eq!(to_json_string(&back), compact, "{name}");
    }
}

/// Written and read back to the same text.
fn round_trip<T: ToJson + FromJson>(v: &T) {
    let text = to_json_string_pretty(v);
    let back: T = from_json_str(&text).unwrap();
    assert_eq!(
        to_json_string_pretty(&back),
        text,
        "{}",
        std::any::type_name::<T>()
    );
}

#[test]
fn defaults_of_every_persisted_type() {
    check(
        &crate::settings::Settings {
            advanced_mode: false,
            ..Default::default()
        },
        golden!("settings_default"),
    );
    check(
        &crate::model::InstallLog::default(),
        golden!("install_log_default"),
    );
    check(&crate::revert::Plan::default(), golden!("plan_default"));
    check(
        &crate::transfer::Manifest::default(),
        golden!("manifest_default"),
    );
    check(&crate::cache::Cache::default(), golden!("cache_default"));
    check(&crate::usage::UsageDb::default(), golden!("usage_default"));
    check(
        &crate::store::PendingSession::default(),
        golden!("pending_default"),
    );
    check(
        &crate::pafbuild::Job {
            settings: crate::settings::Settings {
                advanced_mode: false,
                ..Default::default()
            },
            ..Default::default()
        },
        golden!("job_default"),
    );
    round_trip(&crate::settings::ScanProfile::default());
    round_trip(&crate::model::ChangeSet::default());
    round_trip(&crate::pafbuild::AppDetails::default());
    round_trip(&crate::pafbuild::Status::default());
    round_trip(&crate::programs::InstalledProgram::default());
    round_trip(&crate::prescreen::Screening::default());
    round_trip(&crate::portability::Report::default());
    round_trip(&crate::slimmer::Analysis::default());
    round_trip(&crate::usage::ExeStats::default());
}

fn sample_log() -> crate::model::InstallLog {
    use crate::model::*;
    let mut cs = ChangeSet::default();
    cs.keys_added.push(
        KeyData::new("HKLM\\SOFTWARE\\Vendor\\App")
            .with(RegValue::string("", "default"))
            .with(RegValue::dword("Version", 3))
            .with(RegValue {
                name: "Bin".into(),
                kind: reg_type::BINARY,
                data: vec![0, 1, 254, 255],
            }),
    );
    cs.keys_removed.push(KeyData::new("HKCU\\Software\\Old"));
    cs.values_added.push(ValueRef {
        key: "HKCU\\Environment".into(),
        value: RegValue::string("PATH", "C:\\x;C:\\y"),
    });
    cs.values_modified.push(ValueChange {
        key: "HKCU\\Environment".into(),
        before: RegValue::string("TEMP", "a"),
        after: RegValue::string("TEMP", "b"),
    });
    cs.files_added.push(FileEntry {
        path: "C:\\Program Files\\App\\app.exe".into(),
        is_dir: false,
        size: 1 << 40,
        mtime: -1,
    });
    cs.files_added.push(FileEntry {
        path: "C:\\Program Files\\App".into(),
        is_dir: true,
        size: 0,
        mtime: 1_700_000_000,
    });
    cs.files_modified.push(FileChange {
        before: FileEntry {
            path: "C:\\a.ini".into(),
            is_dir: false,
            size: 1,
            mtime: 2,
        },
        after: FileEntry {
            path: "C:\\a.ini".into(),
            is_dir: false,
            size: 3,
            mtime: 4,
        },
    });
    InstallLog {
        id: "20261016-0507-abc".into(),
        name: "Ünïcode \"App\"".into(),
        publisher: "Vendor".into(),
        version: "1.2.3".into(),
        created: "2026-10-16 05:07:09".into(),
        installer: "C:\\setup.exe".into(),
        uninstall_key: "HKLM\\SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\App".into(),
        notes: "line1\nline2\t\u{1}".into(),
        group: "Tools".into(),
        tags: vec!["auto".into()],
        changes: cs,
    }
}

#[test]
fn install_logs_with_every_kind_of_change() {
    let log = sample_log();
    check(&log, golden!("install_log"));
    let mut no_tags = log;
    no_tags.tags.clear();
    check(&no_tags, golden!("install_log_no_tags"));
}

fn all_actions() -> Vec<crate::revert::Action> {
    use crate::model::*;
    use crate::revert::Action;
    vec![
        Action::StopDeleteService { name: "Svc".into() },
        Action::DeleteTask {
            name: "\\Vendor\\Task".into(),
        },
        Action::DeleteFile {
            path: "C:\\f".into(),
        },
        Action::DeleteDir {
            path: "C:\\d".into(),
        },
        Action::DeleteTree {
            path: "C:\\t".into(),
        },
        Action::DeleteKey {
            path: "HKCU\\Software\\X".into(),
        },
        Action::DeleteValue {
            key: "HKCU\\Software\\X".into(),
            name: "v".into(),
        },
        Action::SetValue {
            key: "HKCU\\Software\\X".into(),
            value: RegValue::dword("d", 7),
        },
        Action::RestoreKey {
            key: KeyData::new("HKCU\\Software\\Y").with(RegValue::string("a", "b")),
        },
        Action::EditList {
            key: "HKCU\\Environment".into(),
            name: "PATH".into(),
            remove: vec!["C:\\x".into()],
            add_back: vec!["C:\\y".into()],
        },
    ]
}

fn full_plan() -> crate::revert::Plan {
    use crate::revert::{Plan, PlanItem};
    Plan {
        items: all_actions()
            .into_iter()
            .map(|action| PlanItem {
                action,
                selected: true,
                exists: false,
                note: "n".into(),
            })
            .collect(),
        info: vec!["i".into()],
    }
}

#[test]
fn every_removal_action_and_plan() {
    for a in &all_actions() {
        round_trip(a);
    }
    check(&full_plan(), golden!("plan_full"));
}

fn backup_items() -> Vec<crate::backup::BackupItem> {
    use crate::backup::BackupItem;
    use crate::model::*;
    vec![
        BackupItem::File {
            original: "C:\\a".into(),
            stored: "files/1".into(),
            size: 9,
        },
        BackupItem::Dir {
            original: "C:\\d".into(),
        },
        BackupItem::RegTree {
            keys: vec![KeyData::new("HKCU\\Software\\Z")],
        },
        BackupItem::RegValue {
            key: "HKCU\\X".into(),
            name: "v".into(),
            previous: Some(RegValue::string("v", "1")),
        },
        BackupItem::RegValue {
            key: "HKCU\\X".into(),
            name: "w".into(),
            previous: None,
        },
        BackupItem::KeyAbsent {
            path: "HKCU\\K".into(),
        },
        BackupItem::FileAbsent {
            path: "C:\\f".into(),
        },
        BackupItem::DirAbsent {
            path: "C:\\g".into(),
        },
    ]
}

#[test]
fn backups_with_every_kind_of_item() {
    let items = backup_items();
    for i in &items {
        round_trip(i);
    }
    check(&items, golden!("backup_items"));
}

#[test]
fn small_enums_and_records() {
    use crate::decrap::{Category, Component, DecidedBy, Kind, Verdict};
    round_trip(&Component {
        kind: Kind::Task,
        name: "\\V\\T".into(),
        command: "x.exe /s".into(),
        location: "HKLM\\...".into(),
        verdict: Verdict::Bloat,
        category: Category::Updater,
        reasons: vec!["r".into()],
        decided_by: DecidedBy::Model,
    });
    for t in [
        crate::trust::Trust::Signed { signer: "S".into() },
        crate::trust::Trust::Unsigned,
        crate::trust::Trust::Invalid {
            signer: "S".into(),
            reason: "R".into(),
        },
        crate::trust::Trust::Missing,
        crate::trust::Trust::Unknown,
    ] {
        round_trip(&t);
    }
    assert_eq!(
        to_json_string(&crate::trust::Trust::Signed { signer: "S".into() }),
        r#"{"Signed":{"signer":"S"}}"#
    );
    assert_eq!(
        to_json_string(&crate::trust::Trust::Unsigned),
        r#""Unsigned""#
    );
    for s in [
        crate::startup::StartupSource::Registry { key: "k".into() },
        crate::startup::StartupSource::Folder { dir: "d".into() },
    ] {
        round_trip(&s);
    }
    round_trip(&crate::transfer::Vars(vec![(
        "APPDATA".into(),
        "C:\\Users\\a\\AppData\\Roaming".into(),
    )]));
    assert_eq!(
        to_json_string(&crate::transfer::Vars(vec![("A".into(), "B".into())])),
        r#"[["A","B"]]"#
    );
}
