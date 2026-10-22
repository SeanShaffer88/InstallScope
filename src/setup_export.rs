//! Offline authoring only. Imported inventories are untrusted declarations.
use crate::json::Value;
use anyhow::{bail, Context, Result};
use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::path::Path;

pub const MAX_BYTES: usize = 2 * 1024 * 1024;
pub const MAX_PACKAGES: usize = 500;
pub const MAX_SELECTED: usize = 50;

#[derive(Clone, Debug)]
pub struct Package {
    pub id: String,
    pub version: Option<String>,
}
#[derive(Clone, Debug, Default)]
pub struct Inventory {
    pub packages: Vec<Package>,
    pub skipped_sources: usize,
    pub duplicates: usize,
}

fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 128
        && id.split('.').count() >= 2
        && id.split('.').all(|part| {
            !part.is_empty()
                && part
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'_' | b'-' | b'+'))
        })
        && id.as_bytes()[0].is_ascii_alphanumeric()
}
fn text<'a>(value: &'a Value, key: &str) -> &'a str {
    value[key].as_str().unwrap_or("")
}

pub fn load(path: &Path) -> Result<Inventory> {
    let metadata = std::fs::symlink_metadata(path).context("Inspect selected inventory file")?;
    if !metadata.is_file() { bail!("Choose a regular inventory JSON file, not a directory, link or device."); }
    if metadata.len() > MAX_BYTES as u64 { bail!("Inventory exceeds the 2 MiB limit. Export a smaller inventory."); }
    let file = std::fs::File::open(path).context("Open WinGet inventory JSON")?;
    let mut bytes = Vec::new();
    file.take(MAX_BYTES as u64 + 1).read_to_end(&mut bytes)?;
    parse(&bytes)
}
pub fn parse(bytes: &[u8]) -> Result<Inventory> {
    if bytes.len() > MAX_BYTES {
        bail!("Inventory exceeds the 2 MiB limit. Export a smaller inventory.");
    }
    let root = crate::json::parse_bytes(bytes.strip_prefix(&[0xef, 0xbb, 0xbf]).unwrap_or(bytes))
        .context("Invalid WinGet inventory JSON")?;
    let sources = root["Sources"]
        .as_array()
        .context("Expected a WinGet export with a Sources array")?;
    if sources.len() > 16 {
        bail!("At most 16 declared sources are supported.");
    }
    let mut inventory = Inventory::default();
    let mut packages = BTreeMap::new();
    let mut count = 0;
    for source in sources {
        let details = &source["SourceDetails"];
        let supported = text(details, "Name").eq_ignore_ascii_case("winget")
            && text(details, "Identifier")
                .eq_ignore_ascii_case("Microsoft.Winget.Source_8wekyb3d8bbwe")
            && text(details, "Argument")
                .trim_end_matches('/')
                .eq_ignore_ascii_case("https://cdn.winget.microsoft.com/cache")
            && text(details, "Type").eq_ignore_ascii_case("Microsoft.PreIndexed.Package");
        if !supported {
            inventory.skipped_sources += 1;
            continue;
        }
        for package in source["Packages"]
            .as_array()
            .context("Supported source must contain a Packages array")?
        {
            count += 1;
            if count > MAX_PACKAGES {
                bail!("Inventory exceeds 500 package entries. Export a smaller inventory.");
            }
            let id = text(package, "PackageIdentifier");
            if !valid_id(id) {
                bail!("Unsupported package ID. Use dotted ASCII package IDs up to 128 characters; no arguments or paths.");
            }
            let version = package
                .get("Version")
                .map(|v| v.as_str().context("Inventory Version must be text"))
                .transpose()?;
            if version.is_some_and(|v| {
                v.len() > 128 || !v.bytes().all(|c| c.is_ascii_graphic() || c == b' ')
            }) {
                bail!("Unsupported version text in inventory.");
            }
            let item = Package {
                id: id.into(),
                version: version.map(String::from),
            };
            match packages.entry(id.to_ascii_lowercase()) {
                std::collections::btree_map::Entry::Occupied(_) => inventory.duplicates += 1,
                std::collections::btree_map::Entry::Vacant(entry) => {
                    entry.insert(item);
                }
            }
        }
    }
    inventory.packages = packages.into_values().collect();
    if inventory.packages.is_empty() {
        bail!("No supported package IDs found. Use an inventory declaring the default winget community source; custom and Store sources are unsupported.");
    }
    Ok(inventory)
}

pub fn configuration(ids: &[String]) -> Result<String> {
    if ids.is_empty() || ids.len() > MAX_SELECTED {
        bail!("Select between 1 and 50 packages before exporting.");
    }
    let mut ordered = BTreeMap::new();
    for id in ids {
        if !valid_id(id) {
            bail!("Invalid package ID; configuration was not generated.");
        }
        if ordered.insert(id.to_ascii_lowercase(), id).is_some() {
            bail!("Duplicate selected package ID.");
        }
    }
    let mut yaml = String::from("# yaml-language-server: $schema=https://aka.ms/configuration-dsc-schema/0.2\n# Generated by SetupScope from explicitly selected, unverified inventory IDs.\n# Review before use. Applying can download modules/installers and change this PC.\n# Verify the local winget source and package licenses before applying elsewhere.\n# Versions, scopes, channels, settings and custom switches are NOT reproduced.\n# Packages and DSC modules are not version-pinned. This is not a reproducible lockfile.\nproperties:\n  resources:\n");
    for id in ordered.into_values() {
        yaml.push_str(&format!("    - resource: Microsoft.WinGet.DSC/WinGetPackage\n      directives:\n        description: 'Install {id}'\n        allowPrerelease: false\n      settings:\n        id: '{id}'\n        source: winget\n"));
    }
    yaml.push_str("  configurationVersion: 0.2.0\n");
    Ok(yaml)
}

/// Publish a complete new file; never replace an existing destination.
pub fn save_new(path: &Path, ids: &[String]) -> Result<()> {
    let yaml = configuration(ids)?;
    if !path.is_absolute() || path.to_string_lossy().contains('\0') {
        bail!("Choose an absolute destination path without null characters.");
    }
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .context("Choose a full destination path")?;
    let staging = parent.join(format!(".setupscope-{}.tmp", crate::util::new_id()));
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&staging)
        .context("Create staged configuration in destination folder")?;
    let result = (|| -> Result<()> {
        file.write_all(yaml.as_bytes())?;
        file.sync_all()?;
        drop(file);
        publish(&staging, path).context(
            "Cannot save configuration. Existing files are never replaced; choose a new filename.",
        )
    })();
    let _ = std::fs::remove_file(&staging);
    result
}
#[cfg(windows)]
fn publish(from: &Path, to: &Path) -> Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{MoveFileExW, MOVEFILE_WRITE_THROUGH};
    let from: Vec<_> = from.as_os_str().encode_wide().chain(Some(0)).collect();
    let to: Vec<_> = to.as_os_str().encode_wide().chain(Some(0)).collect();
    if unsafe { MoveFileExW(from.as_ptr(), to.as_ptr(), MOVEFILE_WRITE_THROUGH) } == 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    Ok(())
}
#[cfg(not(windows))]
fn publish(from: &Path, to: &Path) -> Result<()> {
    std::fs::hard_link(from, to)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture(packages: &str) -> Vec<u8> {
        format!(r#"{{"Sources":[{{"SourceDetails":{{"Name":"winget","Identifier":"Microsoft.Winget.Source_8wekyb3d8bbwe","Argument":"https://cdn.winget.microsoft.com/cache","Type":"Microsoft.PreIndexed.Package"}},"Packages":[{packages}]}}]}}"#).into_bytes()
    }
    #[test]
    fn imports_sorted_ids_and_deduplicates_without_trusting_overrides() {
        let inventory = parse(&fixture(r#"{"PackageIdentifier":"Zed.Tool","Version":"1.2","InitialCustomSwitches":"malicious ignored"},{"PackageIdentifier":"Acme.App"},{"PackageIdentifier":"ACME.APP"}"#)).unwrap();
        assert_eq!(inventory.packages.len(), 2);
        assert_eq!(inventory.packages[0].id, "Acme.App");
        assert_eq!(inventory.duplicates, 1);
        let yaml = configuration(
            &inventory
                .packages
                .iter()
                .map(|p| p.id.clone())
                .collect::<Vec<_>>(),
        )
        .unwrap();
        assert!(!yaml.contains("malicious"));
        assert!(!yaml.contains("1.2"));
        assert!(yaml.contains("allowPrerelease: false"));
    }
    #[test]
    fn rejects_malformed_oversized_and_source_spoofed_inputs() {
        for input in [
            b"{}".to_vec(),
            b"not JSON".to_vec(),
            vec![b' '; MAX_BYTES + 1],
            String::from_utf8(fixture(r#"{"PackageIdentifier":"Acme.App"}"#))
                .unwrap()
                .replace("cdn.winget.microsoft.com", "evil.example")
                .into_bytes(),
        ] {
            assert!(parse(&input).is_err());
        }
        for id in [
            "no-dot",
            "Acme.App\nsettings:",
            "Acme.App'",
            "../Acme.App",
            "Acme..App",
            "Acme.App --silent",
        ] {
            assert!(configuration(&[id.into()]).is_err());
        }
        assert!(configuration(&[]).is_err());
        assert!(configuration(&vec!["Acme.App".into(); 51]).is_err());
        assert!(configuration(&["Acme.App".into(), "acme.app".into()]).is_err());
    }
    #[test]
    fn save_is_complete_and_never_replaces_inventory_or_existing_files() {
        let tmp = crate::util::tempdir().unwrap();
        let path = tmp.path().join("configuration.winget");
        let selected = ["Acme.App".into()];
        save_new(&path, &selected).unwrap();
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            configuration(&selected).unwrap()
        );
        assert!(save_new(&path, &["Other.App".into()]).is_err());
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            configuration(&selected).unwrap()
        );
        assert_eq!(std::fs::read_dir(tmp.path()).unwrap().count(), 1);
        assert!(save_new(Path::new("relative.winget"), &selected).is_err());
        let input = tmp.path().join("inventory.json");
        let original = fixture(r#"{"PackageIdentifier":"Git.Git"}"#);
        std::fs::write(&input, &original).unwrap();
        assert_eq!(load(&input).unwrap().packages[0].id, "Git.Git");
        assert!(save_new(&input, &selected).is_err());
        assert_eq!(std::fs::read(&input).unwrap(), original);
        assert!(load(tmp.path()).is_err());
    }
    #[test]
    fn generated_configuration_matches_schema_validation_fixture() {
        assert!(configuration(&["Microsoft.VCRedist.2015+.x64".into()]).is_ok());
        assert_eq!(
            configuration(&["Git.Git".into()]).unwrap(),
            include_str!("testdata/setup_export/minimal.winget").replace("\r\n", "\n")
        );
    }
    #[test]
    fn byte_source_package_limits_and_bom_are_explicit() {
        let mut bom = vec![0xef, 0xbb, 0xbf];
        bom.extend(fixture(r#"{"PackageIdentifier":"Git.Git"}"#));
        assert!(parse(&bom).is_ok());
        let many = std::iter::repeat_n(r#"{"PackageIdentifier":"Git.Git"}"#, 501)
            .collect::<Vec<_>>()
            .join(",");
        assert!(parse(&fixture(&many)).is_err());
        let mut root =
            crate::json::parse_bytes(&fixture(r#"{"PackageIdentifier":"Git.Git"}"#)).unwrap();
        let original = root["Sources"][0].clone();
        let mut custom = original.clone();
        custom.set("SourceDetails", Value::Null);
        root.set("Sources", Value::Array(vec![original.clone(), custom]));
        let mixed = parse(crate::json::to_json_string(&root).as_bytes()).unwrap();
        assert_eq!((mixed.packages.len(), mixed.skipped_sources), (1, 1));
        root.set("Sources", Value::Array(vec![original; 17]));
        assert!(parse(crate::json::to_json_string(&root).as_bytes()).is_err());
    }
}
