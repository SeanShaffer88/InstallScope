//! Privacy by construction: never serialize diagnostic source reports.
use crate::{
    devcheck::{Report, Status, TOOLS},
    json::{self, ToJson, Value},
};
use anyhow::{bail, Result};
use std::path::Path;
#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub struct Sections {
    pub runtime: bool,
    pub readiness: bool,
    pub tools: bool,
}
fn object(fields: Vec<(&str, Value)>) -> Value {
    Value::Object(fields.into_iter().map(|(k, v)| (k.into(), v)).collect())
}
/// The exact UTF-8 preview/export. No environment reads or arbitrary text copied.
pub fn preview(sections: Sections, report: Option<&Report>) -> Result<String> {
    if !sections.runtime && !sections.readiness && !sections.tools {
        bail!("Select at least one section.");
    }
    if (sections.readiness || sections.tools) && report.is_none() {
        bail!("Run build readiness inspection first, or select runtime only.");
    }
    let mut bundle = object(vec![
        ("schema", 1u32.to_json()),
        ("assembled_at", crate::util::now_string().to_json()),
        ("privacy", "Allowlist redaction: paths, identifiers, addresses, custom tool names, free-form text, environment values, project contents and recovery receipts excluded. No upload.".to_json()),
        ("limitations", "Assembly date is not scan date. Existing scan may be stale, partial or cancelled. Counts do not prove build compatibility; per-check evidence and executable versions omitted. Runtime is this process, not OS certification. Review before sharing.".to_json()),
    ]);
    if sections.runtime {
        bundle.set(
            "runtime",
            object(vec![
                ("version", crate::APP_VERSION.to_json()),
                ("os", std::env::consts::OS.to_json()),
                ("architecture", std::env::consts::ARCH.to_json()),
            ]),
        );
    }
    if sections.readiness {
        let r = report.unwrap();
        let count = |status| {
            r.checks
                .iter()
                .filter(|c| c.status == status)
                .count()
                .to_json()
        };
        bundle.set(
            "readiness",
            object(vec![
                ("profile", r.profile.label().to_json()),
                ("target", r.target.label().to_json()),
                ("checks_found", count(Status::Found)),
                ("checks_review", count(Status::Review)),
                ("checks_missing", count(Status::Missing)),
                ("checks_unknown", count(Status::Unknown)),
                ("path_entry_count", r.entries.len().to_json()),
                ("path_partial", r.path_partial.to_json()),
                ("cancelled", r.cancelled.to_json()),
            ]),
        );
    }
    if sections.tools {
        let r = report.unwrap();
        bundle.set(
            "tools",
            Value::Array(
                TOOLS
                    .iter()
                    .map(|&name| {
                        object(vec![
                            ("name", name.to_json()),
                            (
                                "candidate_count",
                                r.tools
                                    .iter()
                                    .filter(|t| t.name == name)
                                    .fold(0usize, |n, t| n.saturating_add(t.candidates.len()))
                                    .to_json(),
                            ),
                        ])
                    })
                    .collect(),
            ),
        );
    }
    let text = json::to_json_string_pretty(&bundle);
    if text.len() > 32768 {
        bail!("Bundle exceeds 32 KiB limit.");
    }
    Ok(text)
}
pub fn export(path: &Path, reviewed: &str) -> Result<()> {
    if !path.is_absolute() {
        bail!("Choose an absolute output path.");
    }
    if reviewed.len() > 32768 {
        bail!("Bundle exceeds 32 KiB limit.");
    }
    crate::store::durable::write(path, reviewed.as_bytes(), false)?;
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn selection_and_private_source_exclusion() {
        assert!(preview(Sections::default(), None).is_err());
        assert!(preview(
            Sections {
                readiness: true,
                ..Default::default()
            },
            None
        )
        .is_err());
        assert!(preview(
            Sections {
                tools: true,
                ..Default::default()
            },
            None
        )
        .is_err());
        let mut report = crate::devcheck::inspect(
            &crate::devcheck::Request::default(),
            &crate::snapshot::Progress::default(),
        )
        .unwrap();
        let secret = "PRIVATE_SENTINEL_secret_令牌_C:\\Users\\Sean";
        report.notes.push(secret.into());
        report.version_command = secret.into();
        report.tools.push(crate::devcheck::Tool {
            name: secret.into(),
            candidates: vec![secret.into()],
            notes: vec![secret.into()],
        });
        for t in &mut report.tools {
            t.notes.push(secret.into());
            t.candidates.push(secret.into());
        }
        for c in &mut report.checks {
            c.name = secret.into();
            c.evidence.push(secret.into());
        }
        for p in &mut report.entries {
            p.raw = secret.into();
            p.detail = secret.into();
            p.path = Some(secret.into());
        }
        for bits in 1..8 {
            let s = preview(
                Sections {
                    runtime: bits & 1 != 0,
                    readiness: bits & 2 != 0,
                    tools: bits & 4 != 0,
                },
                Some(&report),
            )
            .unwrap();
            assert!(!s.contains("PRIVATE_SENTINEL"));
            assert!(!s.contains("Sean"));
            let v: Value = json::from_json_str(&s).unwrap();
            assert_eq!(v.get("runtime").is_some(), bits & 1 != 0);
            assert_eq!(v.get("readiness").is_some(), bits & 2 != 0);
            assert_eq!(v.get("tools").is_some(), bits & 4 != 0);
        }
    }
    #[test]
    fn export_never_replaces() {
        let dir = std::env::temp_dir().join(format!(
            "setupscope-bundle-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir(&dir).unwrap();
        let path = dir.join("report.json");
        export(&path, "first").unwrap();
        assert!(export(&path, "second").is_err());
        assert_eq!(std::fs::read(&path).unwrap(), b"first");
        assert!(export(Path::new("relative.json"), "x").is_err());
        assert!(export(&dir.join("large.json"), &"x".repeat(32769)).is_err());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
