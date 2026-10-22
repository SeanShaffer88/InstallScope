//! Bounded desktop declarations. Conditions/imports/includes/macros are never evaluated.
use super::*;
use anyhow::ensure;
use quick_xml::{events::Event, Reader as XmlReader};
fn row(path: &Path, tool: &str, value: &str, detail: &str) -> Result<Finding> {
    Ok(Finding::new(path, tool, field(value)?, detail))
}
fn xml(path: &Path, text: &str) -> Result<Vec<Finding>> {
    let mut reader = XmlReader::from_str(text);
    let mut depth = 0usize;
    let mut active: Option<(usize, String, String)> = None;
    let mut rows = Vec::new();
    loop {
        match reader.read_event()? {
            Event::DocType(_) => {
                bail!("DTD declarations are unsupported; no entity expansion or external loading")
            }
            Event::Empty(e) if e.local_name().as_ref() == b"Project" => {
                for attribute in e.attributes() {
                    let attribute = attribute?;
                    if attribute.key.as_ref() == b"Sdk" {
                        let sdk = attribute.decoded_and_normalized_value(
                            quick_xml::XmlVersion::Implicit1_0,
                            reader.decoder(),
                        )?;
                        rows.push(row(path,".NET project SDK",&sdk,"Literal SDK attribute. Imports, restore, framework availability and resolver selection are not evaluated.")?);
                    }
                }
            }
            Event::Start(e) => {
                depth += 1;
                if depth > 64 {
                    bail!("XML depth exceeds 64");
                }
                let name = String::from_utf8(e.local_name().as_ref().to_vec())?;
                if name == "Project" {
                    for a in e.attributes() {
                        let a = a?;
                        if a.key.as_ref() == b"Sdk" {
                            let sdk = a.decoded_and_normalized_value(
                                quick_xml::XmlVersion::Implicit1_0,
                                reader.decoder(),
                            )?;
                            rows.push(row(path,".NET project SDK",&sdk,"SDK attribute declaration. SDK imports, resolver selection, restore and framework availability are not evaluated. Compare with global.json and installed SDK evidence.")?);
                        }
                    }
                }
                if matches!(
                    name.as_str(),
                    "PlatformToolset"
                        | "WindowsTargetPlatformVersion"
                        | "TargetFramework"
                        | "TargetFrameworks"
                        | "TargetFrameworkVersion"
                        | "RuntimeIdentifier"
                        | "RuntimeIdentifiers"
                        | "NETSdkVersion"
                ) {
                    active = Some((depth, name, String::new()));
                }
            }
            Event::Text(e) => {
                if let Some((_, _, value)) = &mut active {
                    let decoded = e.decode()?;
                    ensure!(
                        value.len() + decoded.len() <= 256,
                        "XML declaration exceeds 256 bytes"
                    );
                    value.push_str(&decoded);
                }
            }
            Event::CData(e) => {
                if let Some((_, _, value)) = &mut active {
                    let decoded = e.decode()?;
                    ensure!(
                        value.len() + decoded.len() <= 256,
                        "XML declaration exceeds 256 bytes"
                    );
                    value.push_str(&decoded);
                }
            }
            Event::GeneralRef(_) => {
                if active.is_some() {
                    bail!("Entity-based property value remains unsupported");
                }
            }
            Event::End(_) => {
                if active.as_ref().is_some_and(|(d, _, _)| *d == depth) {
                    let (_, name, value) = active.take().unwrap();
                    let tool = match name.as_str() {
                        "PlatformToolset" => "MSVC toolset",
                        "WindowsTargetPlatformVersion" => "Windows SDK declaration",
                        "NETSdkVersion" => ".NET SDK declaration",
                        _ => ".NET target declaration",
                    };
                    rows.push(row(path,tool,value.trim(),&format!("Literal {name} property; conditions, imports, inheritance and MSBuild property expansion are NOT evaluated. Compare against the installed toolchain and selected configuration. This is not the effective build value."))?);
                    ensure!(rows.len() <= 64, "More than 64 desktop declarations");
                }
                depth = depth.saturating_sub(1);
            }
            Event::Eof => {
                ensure!(depth == 0, "Unclosed XML elements");
                break;
            }
            _ => {}
        }
        ensure!(rows.len() <= 64, "More than 64 desktop declarations");
    }
    Ok(rows)
}
fn json(path: &Path, text: &str) -> Result<Vec<Finding>> {
    let value = crate::json::parse(text)?;
    ensure!(
        value.as_object().is_some(),
        "Manifest must be a JSON object"
    );
    let mut rows = Vec::new();
    let name = path.file_name().unwrap_or_default().to_string_lossy();
    if name == ".vsconfig" {
        if let Some(list) = value.get("components") {
            let list = list.as_array().context("components must be an array")?;
            ensure!(
                list.len() <= 64,
                "More than 64 requested Visual Studio components"
            );
            for item in list {
                rows.push(row(path,"Visual Studio component",item.as_str().context("Component ID must be a string")?,"Requested component ID. Installation state, workload dependencies and licensing are not inferred. Review in Microsoft Visual Studio Installer.")?);
            }
        }
    }
    if name == "vcpkg.json" {
        for key in ["builtin-baseline", "default-triplet"] {
            if let Some(v) = value.get(key) {
                rows.push(row(path,"vcpkg declaration",v.as_str().context("vcpkg declaration must be a string")?,"Declared dependency baseline/triplet. No registry, dependency restore, ABI or installed package check is performed.")?);
            }
        }
        if value.get("dependencies").is_some() {
            rows.push(Finding::new(path,"vcpkg dependencies","Declared","Native packages are required. Review vcpkg dependencies, manifest features, triplet and registry configuration before an explicitly reviewed build."));
        }
    }
    if name.starts_with("CMake") {
        if let Some(v) = value.get("cmakeMinimumRequired") {
            ensure!(v.as_object().is_some(), "CMake minimum must be an object");
            let mut parts = Vec::new();
            for key in ["major", "minor", "patch"] {
                let n = match v.get(key) {
                    Some(n) => n
                        .as_u64()
                        .context("CMake version parts must be nonnegative integers")?,
                    None if key == "patch" => 0,
                    _ => bail!("CMake minimum requires major and minor"),
                };
                ensure!(n <= u32::MAX as u64, "Invalid CMake minimum version");
                parts.push(n.to_string());
            }
            rows.push(row(path,"CMake minimum",&parts.join("."),"Preset minimum version; included files, conditions and macros are not followed. Compare with Toolchain / PATH metadata.")?);
        }
        for key in ["configurePresets", "buildPresets", "testPresets"] {
            if let Some(v) = value.get(key) {
                let list = v.as_array().context("Preset collection must be an array")?;
                ensure!(list.len() <= 32, "More than 32 presets per collection");
                for item in list {
                    let name = item
                        .get("name")
                        .and_then(crate::json::Value::as_str)
                        .context("Preset must have a string name")?;
                    let mut finding=row(path,"CMake preset",name,"Declared preset only. Inheritance, includes, conditions, environment and toolchain files are not evaluated or executed. Select a preset explicitly in the reviewed build arguments.")?;
                    for key in [
                        "generator",
                        "architecture",
                        "toolset",
                        "configurePreset",
                        "binaryDir",
                    ] {
                        if let Some(s) = item.get(key).and_then(crate::json::Value::as_str) {
                            finding.evidence.push(format!("{key}: {}", field(s)?));
                        }
                    }
                    rows.push(finding);
                }
            }
        }
    }
    Ok(rows)
}
pub(super) fn collect(root: &Path, reader: &mut Reader<'_>) -> Vec<Finding> {
    let mut paths = vec![
        root.join("CMakeLists.txt"),
        root.join("CMakePresets.json"),
        root.join("CMakeUserPresets.json"),
        root.join(".vsconfig"),
        root.join("vcpkg.json"),
        root.join("Directory.Build.props"),
    ];
    match fs::read_dir(root) {
        Ok(entries) => {
            for (i, entry) in entries.enumerate() {
                if i == 128 {
                    reader.partial = true;
                    reader.notes.push(
                        "Desktop project discovery stopped at 128 directory entries; no recursion."
                            .into(),
                    );
                    break;
                }
                if reader.progress.cancelled() {
                    break;
                }
                match entry {
                    Ok(e) => {
                        if e.path().extension().is_some_and(|ext| {
                            ["vcxproj", "csproj", "fsproj", "vbproj"]
                                .iter()
                                .any(|s| ext.eq_ignore_ascii_case(s))
                        }) {
                            paths.push(e.path());
                        }
                    }
                    Err(e) => {
                        reader.partial = true;
                        reader.notes.push(format!("Desktop entry unavailable: {e}"));
                    }
                }
            }
        }
        Err(e) => {
            reader.partial = true;
            reader
                .notes
                .push(format!("Desktop project discovery unavailable: {e}"));
        }
    }
    paths.sort();
    paths.dedup();
    let mut rows = Vec::new();
    for path in paths {
        if reader.progress.cancelled() {
            break;
        }
        let result = (|| -> Result<Vec<Finding>> {
            let Some(text) = reader.read(&path)? else {
                return Ok(vec![]);
            };
            if path.file_name().is_some_and(|s| s == "CMakeLists.txt") {
                Ok(vec![Finding::new(&path,"CMake project","CMakeLists.txt present","CMake script presence only. Minimum version, includes, FetchContent, toolchains and conditionals are not evaluated. Review presets and run configuration separately with explicit approval.")])
            } else if path
                .extension()
                .is_some_and(|s| s.eq_ignore_ascii_case("json"))
                || path.file_name().is_some_and(|s| s == ".vsconfig")
            {
                json(&path, &text)
            } else {
                xml(&path, &text)
            }
        })();
        match result {
            Ok(found) => rows.extend(found),
            Err(e) => {
                reader.partial = true;
                reader.notes.push(format!(
                    "{}: desktop declarations unavailable: {e:#}",
                    path.display()
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
    fn xml_comments_are_not_requirements_and_conditions_stay_unverified() {
        let rows=xml(Path::new("test.vcxproj"),"<Project><!-- <PlatformToolset>bad</PlatformToolset> --><PropertyGroup Condition=\"x\"><PlatformToolset>v143</PlatformToolset><WindowsTargetPlatformVersion>10.0.26100.0</WindowsTargetPlatformVersion></PropertyGroup></Project>").unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].declared, "v143");
        assert_eq!(rows[0].state, State::Review);
    }
    #[test]
    fn dtd_and_malformed_xml_are_rejected() {
        assert!(xml(Path::new("x.csproj"), "<!DOCTYPE foo><Project/>").is_err());
        assert!(xml(Path::new("x.csproj"), "<Project>").is_err());
    }
    #[test]
    fn cmake_presets_remain_declarations_not_executed_includes() {
        let rows=json(Path::new("CMakePresets.json"),r#"{"include":["https://example.invalid"],"configurePresets":[{"name":"debug","generator":"Ninja"}],"cmakeMinimumRequired":{"major":3,"minor":25}}"#).unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].declared, "3.25.0");
        assert_eq!(rows[1].evidence, vec!["generator: Ninja"]);
    }
    #[test]
    fn malformed_cmake_versions_do_not_become_zero() {
        assert!(json(
            Path::new("CMakePresets.json"),
            r#"{"cmakeMinimumRequired":{"major":"bad","minor":25}}"#
        )
        .is_err());
    }
    #[test]
    fn sdk_only_project_and_property_budget_are_handled() {
        assert_eq!(
            xml(
                Path::new("x.csproj"),
                r#"<Project Sdk="Microsoft.NET.Sdk"/>"#
            )
            .unwrap()[0]
                .declared,
            "Microsoft.NET.Sdk"
        );
        let text = format!(
            "<Root>{}</Root>",
            r#"<Project Sdk="Microsoft.NET.Sdk"/>"#.repeat(65)
        );
        assert!(xml(Path::new("x.csproj"), &text).is_err());
    }
}
