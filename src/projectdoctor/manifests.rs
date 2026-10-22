use super::*;
fn unknown(source: &Path, tool: &str, detail: impl Into<String>) -> Finding {
    let mut row = Finding::new(source, tool, "Unverified", detail);
    row.state = State::Unknown;
    row
}
fn text_value(value: Option<&toml::Value>) -> Result<&str> {
    field(
        value
            .and_then(toml::Value::as_str)
            .context("Literal text required; inherited/dynamic constraints are not resolved")?,
    )
}
fn json_value(value: Option<&crate::json::Value>) -> Result<&str> {
    field(
        value
            .and_then(crate::json::Value::as_str)
            .context("Literal text requirement expected")?,
    )
}
fn toml(text: &str) -> Result<toml::Value> {
    toml::from_str::<toml::Value>(text).map_err(|_|anyhow::anyhow!("Unsupported or malformed TOML; review this file. Raw contents are not included in this report"))
}
fn parse_file(path: &Path, text: &str) -> Result<Vec<Finding>> {
    let name = path.file_name().unwrap().to_string_lossy();
    let mut rows = vec![];
    match name.as_ref() {
        "Cargo.toml" => {
            let value = toml(text)?;
            if let Some(v) = value.get("package").and_then(|v| v.get("rust-version")) {
                let declared = text_value(Some(v))?;
                rows.push(Finding::new(path,"Rust minimum",declared,"Cargo rust-version is a minimum compiler declaration. Dependencies, workspace/member inheritance, features and actual rustup selection are separate requirements."));
            }
            if value.get("workspace").is_some() {
                rows.push(unknown(path,"Cargo workspace","Workspace members/dependencies and inherited package requirements are not resolved by this bounded inspection. Choose a member folder as well when needed."));
            }
        }
        "pyproject.toml" => {
            let value = toml(text)?;
            if let Some(v) = value.get("project").and_then(|v| v.get("requires-python")) {
                rows.push(Finding::new(path,"Python",text_value(Some(v))?,"Declared requires-python. Only stable numeric PEP 440 comparisons are checked against embedded file metadata; package dependencies, virtual environments and interpreter execution are unverified."));
            }
            if value.get("build-system").is_some() {
                rows.push(unknown(path,"Python build backend","A build-system section is present. Backend/dependency availability is not executed or resolved; review its requirements before building."));
            }
        }
        "package.json" => {
            let value = crate::json::parse(text)?;
            if value.as_object().is_none() {
                bail!("package.json must contain an object");
            }
            if let Some(engines) = value.get("engines") {
                if engines.as_object().is_none() {
                    bail!("engines must be an object");
                }
                for (key, tool) in [("node", "Node.js"), ("npm", "npm")] {
                    if let Some(v) = engines.get(key) {
                        rows.push(Finding::new(path,tool,json_value(Some(v))?,"engines declaration; enforcement depends on the package manager. Stable numeric ranges are inspected; scripts are never run."));
                    }
                }
            }
            if let Some(dev) = value.get("devEngines") {
                if let Some(runtime) = dev.get("runtime") {
                    let list: Vec<_> = match runtime.as_array() {
                        Some(a) if a.len() <= 8 => a.iter().collect(),
                        Some(_) => bail!("More than 8 devEngines runtime entries"),
                        None => vec![runtime],
                    };
                    for entry in list {
                        if json_value(entry.get("name"))? == "node" {
                            if let Some(v) = entry.get("version") {
                                rows.push(Finding::new(path,"Node.js",json_value(Some(v))?,"devEngines.runtime declaration. onFail/enforcement and package-manager behavior are not executed or verified."));
                            }
                        } else {
                            rows.push(unknown(path,"devEngines","An unsupported runtime declaration is present; it was not treated as a Node.js compatibility match."));
                        }
                    }
                }
                for key in ["os", "cpu", "libc", "packageManager"] {
                    if dev.get(key).is_some() {
                        rows.push(unknown(path,&format!("devEngines.{key}"),"Additional environment/package-manager restrictions require review; this inspection does not establish that they are satisfied."));
                    }
                }
            }
            if let Some(v) = value.get("packageManager") {
                rows.push(Finding::new(path,"Package manager",json_value(Some(v))?,"Declared package-manager selection only; Corepack/shims, downloads and actual version selection are not run."));
            }
            for key in ["os", "cpu", "workspaces"] {
                if value.get(key).is_some() {
                    rows.push(unknown(path,&format!("package.json {key}"),"Additional platform/workspace constraints are present; no compatibility or dependency completeness is established by this report."));
                }
            }
        }
        ".python-version" | ".node-version" | ".nvmrc" => {
            let declared = field(text.trim())?;
            let tool = if name == ".python-version" {
                "Python pin"
            } else {
                "Node.js pin"
            };
            rows.push(Finding::new(path,tool,declared,"Version-manager declaration only. Exact stable pins can be compared with candidate file metadata; aliases, partial pins, automatic activation and environment overrides remain unverified."));
        }
        "rust-toolchain" | "rust-toolchain.toml" => {
            if name == "rust-toolchain" && !text.trim_start().starts_with('[') {
                let channel = field(text.trim())?;
                if !channel.is_ascii() {
                    bail!("Legacy rust-toolchain must be ASCII");
                }
                rows.push(Finding::new(path,"Rust toolchain",channel,"Nearest legacy rust-toolchain declaration (takes precedence over rust-toolchain.toml in the same folder)."));
            } else {
                let value = toml(text)?;
                let toolchain = value
                    .get("toolchain")
                    .and_then(toml::Value::as_table)
                    .context("A [toolchain] table is required")?;
                if toolchain.contains_key("channel") && toolchain.contains_key("path") {
                    bail!("toolchain.channel and toolchain.path are mutually exclusive");
                }
                if let Some(v) = toolchain.get("channel") {
                    rows.push(Finding::new(path,"Rust toolchain",text_value(Some(v))?,"Nearest Rust toolchain-file channel. Only an installed store-name candidate is checked; overrides may take precedence."));
                } else {
                    rows.push(unknown(path,"Rust toolchain","Custom path/default channel selection is not resolved or followed by this inspection."));
                }
                for key in ["components", "targets"] {
                    if let Some(v) = toolchain.get(key) {
                        let list = v.as_array().context("Components/targets must be arrays")?;
                        if list.len() > 16 {
                            bail!("At most 16 component/target declarations are reported");
                        }
                        let mut values = vec![];
                        for item in list {
                            values.push(text_value(Some(item))?);
                        }
                        let mut row=Finding::new(path,&format!("Rust {key}"),&values.join(", "),"Declared additional components/targets; installed component completeness remains unverified. Existing Rust/MSVC prerequisite diagnostics are available separately.");
                        row.state = State::Unknown;
                        rows.push(row);
                    }
                }
            }
        }
        _ => {}
    }
    Ok(rows)
}
pub(super) fn collect(root: &Path, reader: &mut Reader<'_>) -> Vec<Finding> {
    let mut rows = vec![];
    if let Some((path, text)) = reader.nearest(root, &["global.json"]) {
        match dotnet::parse(&text) {
            Ok(policy) => {
                let declared = format!(
                    "{}; {}",
                    policy
                        .version
                        .as_ref()
                        .map(ToString::to_string)
                        .unwrap_or_else(|| "unbounded".into()),
                    policy.roll
                );
                let mut row=Finding::new(&path,".NET SDK",&declared,"Nearest global.json from the selected folder as working directory. MSBuild can start its search at a solution/project folder instead.");
                row.sdk = Some(policy);
                rows.push(row);
            }
            Err(e) => {
                reader.partial = true;
                rows.push(unknown(
                    &path,
                    ".NET SDK",
                    format!("Cannot assess SDK declaration: {e:#}"),
                ));
            }
        }
    }
    if let Some((path, text)) = reader.nearest(root, &["rust-toolchain", "rust-toolchain.toml"]) {
        match parse_file(&path, &text) {
            Ok(found) => rows.extend(found),
            Err(e) => {
                reader.partial = true;
                rows.push(unknown(
                    &path,
                    "Rust toolchain",
                    format!("Cannot assess toolchain declaration: {e:#}"),
                ));
            }
        }
    }
    for name in [
        "Cargo.toml",
        "package.json",
        "pyproject.toml",
        ".nvmrc",
        ".node-version",
        ".python-version",
    ] {
        if reader.progress.cancelled() {
            break;
        }
        let path = root.join(name);
        match reader.read(&path) {
            Ok(Some(text)) => match parse_file(&path, &text) {
                Ok(found) => rows.extend(found),
                Err(e) => {
                    reader.partial = true;
                    rows.push(unknown(
                        &path,
                        name,
                        format!("Cannot assess manifest: {e:#}"),
                    ));
                }
            },
            Ok(None) => {}
            Err(e) => {
                reader.partial = true;
                rows.push(unknown(
                    &path,
                    name,
                    format!("Manifest inspection unavailable: {e:#}"),
                ));
            }
        }
    }
    rows
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn full_toml_supports_quoted_tables_and_comments() {
        let rows = parse_file(
            Path::new("Cargo.toml"),
            "['package']\n'rust-version' = '1.91' # comment",
        )
        .unwrap();
        assert_eq!(rows[0].declared, "1.91");
        let rows = parse_file(
            Path::new("rust-toolchain.toml"),
            "[toolchain]\nchannel='1.91.0'\ntargets=[\n'x86_64-pc-windows-msvc',\n]",
        )
        .unwrap();
        assert_eq!(rows.len(), 2);
    }
    #[test]
    fn package_scripts_are_not_interpreted() {
        let rows=parse_file(Path::new("package.json"),r#"{"engines":{"node":">=18 <22"},"scripts":{"postinstall":"arbitrary shell code"},"devEngines":{"runtime":{"name":"node","version":"^20.0.0"}}}"#).unwrap();
        assert_eq!(rows.len(), 2);
        assert!(rows.iter().all(|r| !r.detail.contains("arbitrary shell")));
    }
    #[test]
    fn dynamic_and_inherited_values_fail_instead_of_matching() {
        assert!(parse_file(
            Path::new("Cargo.toml"),
            "[package]\nrust-version={workspace=true}"
        )
        .is_err());
        assert!(parse_file(Path::new("package.json"), r#"{"engines":{"node":123}}"#).is_err());
    }
}
