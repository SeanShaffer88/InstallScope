use super::*;
use crate::json;
#[derive(Clone, Debug)]
pub(super) struct Policy {
    pub version: Option<semver::Version>,
    pub roll: String,
    pub prerelease: Option<bool>,
    pub custom_paths: bool,
}
/// JSON with comments/trailing commas accepted by global.json, preserving strings.
fn jsonc(text: &str) -> Result<json::Value> {
    let mut bytes = text.as_bytes().to_vec();
    let mut i = 0;
    let mut quoted = false;
    while i < bytes.len() {
        if quoted {
            if bytes[i] == b'\\' {
                i += 2;
                continue;
            }
            if bytes[i] == b'"' {
                quoted = false;
            }
            i += 1;
            continue;
        }
        if bytes[i] == b'"' {
            quoted = true;
            i += 1;
            continue;
        }
        if bytes.get(i..i + 2) == Some(b"//") {
            while i < bytes.len() && bytes[i] != b'\n' {
                bytes[i] = b' ';
                i += 1;
            }
            continue;
        }
        if bytes.get(i..i + 2) == Some(b"/*") {
            bytes[i] = b' ';
            bytes[i + 1] = b' ';
            i += 2;
            let mut closed = false;
            while i + 1 < bytes.len() {
                if bytes.get(i..i + 2) == Some(b"*/") {
                    bytes[i] = b' ';
                    bytes[i + 1] = b' ';
                    i += 2;
                    closed = true;
                    break;
                }
                bytes[i] = b' ';
                i += 1;
            }
            if !closed {
                bail!("Unterminated global.json comment");
            }
            continue;
        }
        i += 1;
    }
    i = 0;
    quoted = false;
    while i < bytes.len() {
        if quoted {
            if bytes[i] == b'\\' {
                i += 2;
                continue;
            }
            if bytes[i] == b'"' {
                quoted = false;
            }
            i += 1;
            continue;
        }
        if bytes[i] == b'"' {
            quoted = true;
        } else if bytes[i] == b',' {
            let mut next = i + 1;
            while next < bytes.len() && bytes[next].is_ascii_whitespace() {
                next += 1;
            }
            if matches!(bytes.get(next), Some(b'}' | b']')) {
                bytes[i] = b' ';
            }
        }
        i += 1;
    }
    Ok(json::parse_bytes(&bytes)?)
}
pub(super) fn parse(text: &str) -> Result<Policy> {
    let value = jsonc(text)?;
    let sdk = value
        .get("sdk")
        .context("No sdk object: SDK resolution remains unconstrained by this file")?;
    if sdk.as_object().is_none() {
        bail!("sdk must be an object");
    }
    let version = match sdk.get("version") {
        Some(v) => Some(semver::Version::parse(field(
            v.as_str().context("sdk.version must be text")?,
        )?)?),
        None => None,
    };
    let roll = sdk
        .get("rollForward")
        .map(|v| v.as_str().context("rollForward must be text"))
        .transpose()?
        .unwrap_or("patch")
        .to_owned();
    if ![
        "disable",
        "patch",
        "feature",
        "minor",
        "major",
        "latestPatch",
        "latestFeature",
        "latestMinor",
        "latestMajor",
    ]
    .contains(&roll.as_str())
    {
        bail!("Unsupported SDK roll-forward policy");
    }
    if version.is_none() && roll != "latestMajor" {
        bail!("No explicit SDK version; effective selection is not assessed");
    }
    let prerelease = sdk
        .get("allowPrerelease")
        .map(|v| v.as_bool().context("allowPrerelease must be boolean"))
        .transpose()?;
    Ok(Policy {
        version,
        roll,
        prerelease,
        custom_paths: sdk.get("paths").is_some(),
    })
}
fn select(policy: &Policy, versions: &[semver::Version]) -> Option<semver::Version> {
    let mut eligible: Vec<_> = versions
        .iter()
        .filter(|v| policy.prerelease != Some(false) || v.pre.is_empty())
        .filter(|v| {
            let Some(want) = &policy.version else {
                return true;
            };
            if *v < want {
                return false;
            }
            match policy.roll.as_str() {
                "disable" => *v == want,
                "patch" | "latestPatch" => {
                    v.major == want.major
                        && v.minor == want.minor
                        && v.patch / 100 == want.patch / 100
                }
                "feature" | "latestFeature" => v.major == want.major && v.minor == want.minor,
                "minor" | "latestMinor" => v.major == want.major,
                _ => true,
            }
        })
        .cloned()
        .collect();
    eligible.sort();
    if policy.roll == "patch" {
        if let Some(want) = &policy.version {
            if eligible.contains(want) {
                return Some(want.clone());
            }
        }
    }
    if matches!(policy.roll.as_str(), "feature" | "minor" | "major") {
        let first = eligible.first()?;
        let band = (first.major, first.minor, first.patch / 100);
        eligible.retain(|v| (v.major, v.minor, v.patch / 100) == band);
    }
    eligible.last().cloned()
}
pub(super) fn assess(
    row: &mut Finding,
    request: &Request,
    tooling: &devcheck::Report,
    progress: &Progress,
    reader: &mut Reader<'_>,
) {
    let Some(policy) = row.sdk.clone() else {
        return;
    };
    if policy.custom_paths {
        row.state = State::Unknown;
        row.detail.push_str(
            " sdk.paths changes resolver roots. Custom paths are not followed by this inspection.",
        );
        return;
    }
    let mut roots = request.dotnet_roots.clone();
    for tool in tooling.tools.iter().filter(|t| t.name == "dotnet") {
        for candidate in &tool.candidates {
            if let Some(parent) = candidate.parent() {
                if !roots.iter().any(|p| p == parent) {
                    roots.push(parent.into());
                }
            }
        }
    }
    let mut inspected = std::collections::HashSet::new();
    let mut complete = true;
    let mut matched = false;
    for root in roots.iter().take(16) {
        if progress.cancelled() {
            return;
        }
        let key = root.to_string_lossy().to_lowercase();
        if !inspected.insert(key) {
            continue;
        }
        let dir = root.join("sdk");
        if let Err(e) = guard(&dir) {
            row.evidence.push(format!("{}: {e:#}", dir.display()));
            complete = false;
            continue;
        }
        let entries = match fs::read_dir(&dir) {
            Ok(e) => e,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(e) => {
                complete = false;
                row.evidence.push(format!("{}: {e}", dir.display()));
                continue;
            }
        };
        let host = arch(&root.join("dotnet.exe")).ok();
        let mut versions = vec![];
        for (i, entry) in entries.enumerate() {
            if progress.cancelled() {
                return;
            }
            if i == 256 {
                complete = false;
                reader.partial = true;
                break;
            }
            let Ok(entry) = entry else {
                complete = false;
                continue;
            };
            let name = entry.file_name().to_string_lossy().into_owned();
            if let Ok(version) = semver::Version::parse(&name) {
                match regular_checked(&entry.path().join("dotnet.dll")) {
                    Ok(true) => versions.push(version),
                    Ok(false) => {}
                    Err(e) => {
                        complete = false;
                        reader.partial = true;
                        if row.evidence.len() < 32 {
                            row.evidence.push(format!(
                                "{}: SDK file unknown ({e:#})",
                                entry.path().display()
                            ));
                        }
                    }
                }
            }
        }
        if let Some(selected) = select(&policy, &versions) {
            matched = true;
            row.evidence.push(format!("{}: candidate SDK {}; host PE {} (requested target {}). Selection is computed separately per observed root, not proof of the root used by your IDE/terminal.",root.display(),selected,host.unwrap_or("unknown"),request.tools.target.label()));
        } else {
            row.evidence.push(format!(
                "{}: {} SDK directory candidate(s); none satisfy this policy in this root.",
                root.display(),
                versions.len()
            ));
        }
    }
    if roots.len() > 16 {
        complete = false;
        reader.partial = true;
        row.evidence
            .push("SDK root inventory stopped at 16 roots.".into());
    }
    reader.partial |= !complete;
    row.state = if matched {
        State::Candidate
    } else if complete && !tooling.path_partial {
        State::Missing
    } else {
        State::Unknown
    };
    if policy.prerelease.is_none() {
        row.detail.push_str(" allowPrerelease is unspecified; Visual Studio and CLI defaults can differ. Candidate calculation permits prerelease SDKs, without claiming effective selection.");
    }
    row.detail.push_str(" Only SDK directory names plus dotnet.dll presence were checked. Workloads, runtime behavior, custom resolvers and project/MSBuild resolution remain unverified.");
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn comments_and_trailing_commas_preserve_strings() {
        let p = parse(
            r#"{ // comment
        "sdk":{"version":"8.0.100", /* */ "rollForward":"latestFeature",},}"#,
        )
        .unwrap();
        assert_eq!(p.roll, "latestFeature");
        assert!(
            parse(r#"{"sdk":{"version":"8.0.100"},"x":"https://example.invalid/a/*b*/",}"#).is_ok()
        );
    }
    #[test]
    fn roll_forward_policies_choose_feature_bands_correctly() {
        let versions: Vec<_> = ["8.0.100", "8.0.105", "8.0.201", "8.1.100", "9.0.100"]
            .iter()
            .map(|v| semver::Version::parse(v).unwrap())
            .collect();
        for (roll, want) in [
            ("disable", "8.0.100"),
            ("patch", "8.0.100"),
            ("feature", "8.0.105"),
            ("latestPatch", "8.0.105"),
            ("latestFeature", "8.0.201"),
            ("latestMinor", "8.1.100"),
            ("latestMajor", "9.0.100"),
            ("minor", "8.0.105"),
            ("major", "8.0.105"),
        ] {
            let p = parse(&format!(
                r#"{{"sdk":{{"version":"8.0.100","rollForward":"{roll}"}}}}"#
            ))
            .unwrap();
            assert_eq!(select(&p, &versions).unwrap().to_string(), want, "{roll}");
        }
    }
    #[test]
    fn missing_band_moves_to_next_not_highest_for_feature() {
        let p = parse(
            r#"{"sdk":{"version":"8.0.150","rollForward":"feature","allowPrerelease":false}}"#,
        )
        .unwrap();
        let v = ["8.0.201", "8.0.300", "8.0.205"].map(|s| semver::Version::parse(s).unwrap());
        assert_eq!(select(&p, &v).unwrap().to_string(), "8.0.205");
    }
    #[test]
    fn invalid_and_duplicate_policies_fail() {
        for text in [
            r#"{"sdk":{"version":"8.0"}}"#,
            r#"{"sdk":{"version":"8.0.100","rollForward":"surprise"}}"#,
            r#"{"sdk":{"version":"8.0.100","allowPrerelease":"false"}}"#,
            r#"{"sdk":{"version":"8.0.100","version":"9.0.100"}}"#,
        ] {
            assert!(parse(text).is_err());
        }
    }
}
