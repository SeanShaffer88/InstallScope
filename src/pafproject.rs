//! Saved wizard projects. These are InstallScope records, not part of PAF.
//! Official tools remain separate processes; no upstream implementation is copied.
use crate::json;
use anyhow::{anyhow, Context, Result};
use std::io::{Read, Write};
use std::path::Path;

pub const FORMAT_SOURCE: &str = "https://portableapps.com/development/portableapps.com_format";
pub const LAUNCH_SOURCE: &str =
    "https://portableapps.com/manuals/PortableApps.comLauncher/ref/launcher.ini/launch.html";
pub const STEPS: [&str; 10] = [
    "Welcome",
    "Source files",
    "Application identity",
    "Portability",
    "Launcher",
    "Icons",
    "Installer",
    "Licenses",
    "Review",
    "Testing and export",
];
const LIMIT: usize = 1024 * 1024;

#[derive(Clone, Debug, PartialEq)]
pub struct Project {
    pub schema: u32,
    pub step: usize,
    pub workspace: String,
    pub source: String,
    pub app_id: String,
    pub name: String,
    pub publisher: String,
    pub homepage: String,
    pub description: String,
    pub category: String,
    pub language: String,
    pub package_version: String,
    pub display_version: String,
    pub program: String,
    pub arguments: String,
    pub portability_notes: String,
    pub icon: String,
    pub installer: String,
    pub generator: String,
    pub rights: Option<bool>,
    pub open_source: Option<bool>,
    pub freeware: Option<bool>,
    pub commercial_use: Option<bool>,
    pub shareable: Option<bool>,
    pub components: Vec<Component>,
    pub tests: Vec<String>,
}
crate::json_struct!(Project, { schema, step, workspace, source, app_id, name, publisher, homepage, description, category, language, package_version, display_version, program, arguments, portability_notes, icon, installer, generator, rights, open_source, freeware, commercial_use, shareable, components, tests });

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Component {
    pub name: String,
    pub license: String,
    pub notice: String,
    pub rights: Option<bool>,
}
crate::json_struct!(Component, { name, license, notice, rights });

impl Default for Project {
    fn default() -> Self {
        Self {
            schema: 1,
            step: 0,
            workspace: String::new(),
            source: String::new(),
            app_id: String::new(),
            name: String::new(),
            publisher: String::new(),
            homepage: String::new(),
            description: String::new(),
            category: String::new(),
            language: String::new(),
            package_version: String::new(),
            display_version: String::new(),
            program: String::new(),
            arguments: String::new(),
            portability_notes: String::new(),
            icon: String::new(),
            installer: String::new(),
            generator: String::new(),
            rights: None,
            open_source: None,
            freeware: None,
            commercial_use: None,
            shareable: None,
            components: Vec::new(),
            tests: Vec::new(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RuleClass {
    Format,
    Tool,
    Wizard,
}
impl RuleClass {
    pub fn label(self) -> &'static str {
        match self {
            Self::Format => "Official format",
            Self::Tool => "Official tool behavior",
            Self::Wizard => "Wizard safeguard",
        }
    }
}
#[derive(Clone, Debug)]
pub struct Finding {
    pub step: usize,
    pub blocking: bool,
    pub field: &'static str,
    pub message: String,
    pub class: RuleClass,
    pub source: &'static str,
}

impl Project {
    /// Incomplete projects can be saved. Invalid/unknown schemas cannot be loaded.
    fn check_record(&self) -> Result<()> {
        if self.schema != 1 {
            return Err(anyhow!("Unsupported project version {}; this program supports version 1. Keep the original file.", self.schema));
        }
        if self.step >= STEPS.len() {
            return Err(anyhow!("Project has an invalid wizard step"));
        }
        Ok(())
    }

    pub fn load(path: &Path) -> Result<Self> {
        let mut bytes = Vec::new();
        std::fs::File::open(path)?
            .take((LIMIT + 1) as u64)
            .read_to_end(&mut bytes)?;
        if bytes.len() > LIMIT {
            return Err(anyhow!("Project exceeds the 1 MiB limit"));
        }
        let value: json::Value = json::from_json_slice(&bytes)
            .context("Invalid project JSON; original file was not changed")?;
        // Unknown fields might belong to a newer editor. Refuse rather than lose them on save.
        let known = json::ToJson::to_json(&Self::default());
        if let Some(fields) = value.as_object() {
            for (key, _) in fields {
                if known.get(key).is_none() {
                    return Err(anyhow!(
                        "Unknown project field {key}; use the editor that created this file"
                    ));
                }
            }
        }
        if let Some(components) = value.get("components").and_then(json::Value::as_array) {
            let known = json::ToJson::to_json(&Component::default());
            for component in components {
                if let Some(fields) = component.as_object() {
                    for (key, _) in fields {
                        if known.get(key).is_none() {
                            return Err(anyhow!(
                                "Unknown component field {key}; original file was not changed"
                            ));
                        }
                    }
                }
            }
        }
        let project = <Self as json::FromJson>::from_json(&value)?;
        project.check_record()?;
        Ok(project)
    }

    /// Save a new snapshot, never replace an existing file. A failed write keeps
    /// its partial file for diagnosis and reports its location; originals survive.
    pub fn save_new(&self, path: &Path) -> Result<()> {
        self.check_record()?;
        let bytes = json::to_json_string_pretty(self);
        if bytes.len() > LIMIT {
            return Err(anyhow!("Project exceeds the 1 MiB limit"));
        }
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)
            .context("Choose a new project filename; existing files are never replaced")?;
        file.write_all(bytes.as_bytes()).and_then(|_| file.sync_all())
            .with_context(|| format!("Could not finish saving {}; original files are intact. This new snapshot may be incomplete.", path.display()))
    }

    /// Pure answer validation: no filesystem or network access.
    pub fn findings(&self) -> Vec<Finding> {
        let mut out = Vec::new();
        let mut add = |step, blocking, field, message: String, class, source| {
            out.push(Finding {
                step,
                blocking,
                field,
                message,
                class,
                source,
            })
        };
        for (step, field, value) in [
            (0, "Workspace", &self.workspace),
            (1, "Source folder", &self.source),
        ] {
            if value.is_empty() || !Path::new(value).is_absolute() {
                add(
                    step,
                    true,
                    field,
                    "Choose an existing folder using an absolute path.".into(),
                    RuleClass::Wizard,
                    "",
                );
            }
        }
        for (field, value) in [
            ("Name", &self.name),
            ("AppID", &self.app_id),
            ("Publisher", &self.publisher),
            ("Homepage", &self.homepage),
            ("Description", &self.description),
            ("Category", &self.category),
            ("Language", &self.language),
            ("Package version", &self.package_version),
            ("Display version", &self.display_version),
        ] {
            if value.trim().is_empty() || value.contains(['\r', '\n', '\0', '"']) {
                add(
                    2,
                    true,
                    field,
                    "Enter a value without line breaks, NUL, or double quotes.".into(),
                    RuleClass::Format,
                    FORMAT_SOURCE,
                );
            }
        }
        if let Err(e) = portable_builder_kit::validate_app_id(&self.app_id) {
            add(
                2,
                true,
                "AppID",
                e.to_string(),
                RuleClass::Wizard,
                FORMAT_SOURCE,
            );
        }
        if !crate::pafbuild::CATEGORIES.contains(&self.category.as_str()) {
            add(
                2,
                true,
                "Category",
                "Choose a category from the official list.".into(),
                RuleClass::Format,
                FORMAT_SOURCE,
            );
        }
        if self.description.chars().count() > 512 {
            add(
                2,
                true,
                "Description",
                "Use no more than 512 characters.".into(),
                RuleClass::Format,
                FORMAT_SOURCE,
            );
        }
        if self.package_version.split('.').count() != 4
            || self.package_version.split('.').any(|p| {
                p.is_empty() || !p.bytes().all(|b| b.is_ascii_digit()) || p.parse::<u16>().is_err()
            })
        {
            add(2,true,"Package version","Enter four numbers separated by periods, each 0–65535. The wizard never derives this silently from a display version.".into(),RuleClass::Wizard,FORMAT_SOURCE);
        }
        if self.portability_notes.trim().is_empty() {
            add(3,true,"Portability notes","Record where settings are stored and what still needs testing. Unknown is an acceptable observation; it is not a passing test.".into(),RuleClass::Wizard,"");
        }
        if portable_builder_kit::relative_path(&self.program).is_err() {
            add(
                4,
                true,
                "Program executable",
                "Choose a safe path relative to App, such as Example\\example.exe.".into(),
                RuleClass::Tool,
                LAUNCH_SOURCE,
            );
        }
        if self.arguments.contains(['\r', '\n', '\0']) {
            add(
                4,
                true,
                "Arguments",
                "Arguments must fit on one INI line.".into(),
                RuleClass::Wizard,
                LAUNCH_SOURCE,
            );
        }
        if self.icon.is_empty() || !Path::new(&self.icon).is_absolute() {
            add(5,true,"App icon","Choose a square PNG from 256 to 4096 pixels. Review the converted icon before preparing files.".into(),RuleClass::Wizard,FORMAT_SOURCE);
        }
        for (field, value) in [
            ("Official Installer", &self.installer),
            ("Official Launcher Generator", &self.generator),
        ] {
            if value.is_empty() || !Path::new(value).is_absolute() {
                add(6,true,field,"Locate the installed official tool. Selecting a file does not verify its publisher or version.".into(),RuleClass::Wizard,"");
            }
        }
        if self.rights != Some(true)
            || [
                self.open_source,
                self.freeware,
                self.commercial_use,
                self.shareable,
            ]
            .contains(&None)
        {
            add(7,true,"Application rights","Review redistribution permission and answer every license declaration explicitly. Unknown permission blocks package preparation.".into(),RuleClass::Wizard,"");
        }
        for c in &self.components {
            if c.name.trim().is_empty()
                || c.license.trim().is_empty()
                || c.notice.trim().is_empty()
                || c.rights != Some(true)
            {
                add(7,true,"Included component",format!("Review name, license, notice location and redistribution permission for {}.",c.name),RuleClass::Wizard,"");
            }
        }
        add(9,false,"Manual testing","Configuration drafts do not establish portability, official compliance, or certification. Build and clean-machine tests remain required.".into(),RuleClass::Wizard,"");
        out
    }

    /// Filesystem checks must run on a worker: removable or network paths can block.
    pub fn inspect_paths(&self) -> Vec<Finding> {
        let mut out = Vec::new();
        for (step, field, value, directory) in [
            (0, "Workspace", &self.workspace, true),
            (1, "Source folder", &self.source, true),
            (5, "App icon", &self.icon, false),
            (6, "Official Installer", &self.installer, false),
            (6, "Official Launcher Generator", &self.generator, false),
        ] {
            let result = std::fs::metadata(value);
            if !result
                .as_ref()
                .is_ok_and(|m| if directory { m.is_dir() } else { m.is_file() })
            {
                out.push(Finding {
                    step,
                    blocking: true,
                    field,
                    message: match result {
                        Err(e) => format!("Cannot access {value}: {e}"),
                        Ok(_) => "The selected path has the wrong file type.".into(),
                    },
                    class: RuleClass::Wizard,
                    source: "",
                });
            }
        }
        if !self.workspace.is_empty() && !self.source.is_empty() {
            if let (Ok(w), Ok(s)) = (
                std::fs::canonicalize(&self.workspace),
                std::fs::canonicalize(&self.source),
            ) {
                if w.starts_with(&s) || s.starts_with(&w) {
                    out.push(Finding { step:1, blocking:true, field:"Source folder", message:"Keep workspace and source folders separate, with neither inside the other.".into(), class:RuleClass::Wizard, source:"" });
                }
            }
        }
        out
    }

    /// Draft only. This neither writes a package nor runs tools.
    pub fn drafts(&self) -> Result<Vec<(String, String)>> {
        if self
            .findings()
            .iter()
            .any(|f| f.blocking && [2, 4, 7].contains(&f.step))
        {
            return Err(anyhow!("Resolve identity, launcher and licensing findings before generating configuration previews"));
        }
        let yes = |v: Option<bool>| if v == Some(true) { "true" } else { "false" };
        let metadata = format!("[Format]\r\nType=PortableApps.comFormat\r\nVersion=3.9\r\n\r\n[Details]\r\nName={}\r\nAppID={}\r\nPublisher={}\r\nHomepage={}\r\nCategory={}\r\nDescription={}\r\nLanguage={}\r\n\r\n[License]\r\nShareable={}\r\nOpenSource={}\r\nFreeware={}\r\nCommercialUse={}\r\n\r\n[Version]\r\nPackageVersion={}\r\nDisplayVersion={}\r\n\r\n[Control]\r\nIcons=1\r\nStart={}.exe\r\n",self.name,self.app_id,self.publisher,self.homepage,self.category,self.description,self.language,yes(self.shareable),yes(self.open_source),yes(self.freeware),yes(self.commercial_use),self.package_version,self.display_version,self.app_id);
        let mut launcher = format!(
            "[Launch]\r\nProgramExecutable={}\r\n",
            self.program.replace('/', "\\")
        );
        if !self.arguments.is_empty() {
            launcher.push_str(&format!("CommandLineArguments={}\r\n", self.arguments));
        }
        if metadata.len() + launcher.len() > 64 * 1024 {
            return Err(anyhow!(
                "Configuration previews exceed the 64 KiB wizard limit"
            ));
        }
        Ok(vec![
            ("App/AppInfo/appinfo.ini".into(), metadata),
            (
                format!("App/AppInfo/Launcher/{}.ini", self.app_id),
                launcher,
            ),
        ])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn temp() -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "paf-project-{}-{}",
            std::process::id(),
            crate::util::new_id()
        ))
    }
    fn ready_answers() -> Project {
        Project {
            name: "Example Portable".into(),
            app_id: "ExamplePortable".into(),
            publisher: "Example Publisher".into(),
            homepage: "https://example.org/portable".into(),
            category: "Utilities".into(),
            description: "Example application".into(),
            language: "English".into(),
            package_version: "1.2.0.1".into(),
            display_version: "1.2 Revision 1".into(),
            program: "Example/example.exe".into(),
            rights: Some(true),
            open_source: Some(false),
            freeware: Some(true),
            commercial_use: Some(false),
            shareable: Some(false),
            ..Default::default()
        }
    }
    #[test]
    fn generated_metadata_is_readable_and_preserves_explicit_declarations() {
        let p = ready_answers();
        let files = p.drafts().unwrap();
        assert_eq!(files.len(), 2); // No unnecessary installer.ini or invented receipt.
        assert!(files[0].1.contains("CommercialUse=false\r\n"));
        assert!(files[0].1.contains("Shareable=false\r\n"));
        assert!(files[0].1.contains("PackageVersion=1.2.0.1\r\n"));
        assert_eq!(
            files[1].1,
            "[Launch]\r\nProgramExecutable=Example\\example.exe\r\n"
        );
        let root = temp();
        std::fs::create_dir_all(root.join("App/AppInfo")).unwrap();
        std::fs::write(root.join(&files[0].0), &files[0].1).unwrap();
        assert_eq!(
            portable_builder_kit::package_app_id(&root).unwrap(),
            p.app_id
        );
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn drafts_reject_overflow_arguments_and_unresolved_components() {
        let mut p = ready_answers();
        p.package_version = "1.2.65536.0".into();
        assert!(p.drafts().is_err());
        p.package_version = "1.2.0.1".into();
        p.arguments = "ok\r\n[FileWrite1]".into();
        assert!(p.drafts().is_err());
        p.arguments.clear();
        p.components.push(Component::default());
        assert!(p.drafts().is_err());
        p.components.clear();
        p.rights = Some(false);
        assert!(p.drafts().is_err());
        p.rights = Some(true);
        p.arguments = "x".repeat(65536);
        assert!(p.drafts().is_err());
    }
    #[test]
    fn path_inspection_detects_overlap_and_missing_files_without_mutation() {
        let root = temp();
        std::fs::create_dir_all(root.join("source/work")).unwrap();
        let p = Project {
            source: root.join("source").to_string_lossy().into(),
            workspace: root.join("source/work").to_string_lossy().into(),
            ..Default::default()
        };
        let findings = p.inspect_paths();
        assert!(findings
            .iter()
            .any(|f| f.field == "Source folder" && f.message.contains("separate")));
        assert!(findings.iter().any(|f| f.field == "Official Installer"));
        assert_eq!(
            std::fs::read_dir(root.join("source/work")).unwrap().count(),
            0
        );
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn incomplete_unicode_project_round_trips_without_overwrite() {
        let path = temp();
        let p = Project {
            name: "工具 — résumé".into(),
            step: 4,
            ..Default::default()
        };
        p.save_new(&path).unwrap();
        assert_eq!(Project::load(&path).unwrap(), p);
        assert!(p.save_new(&path).is_err());
        assert_eq!(Project::load(&path).unwrap(), p);
        std::fs::remove_file(path).unwrap();
    }
    #[test]
    fn rejects_newer_corrupt_and_oversize_projects() {
        let path = temp();
        for data in [
            "{}".to_string(),
            "{".into(),
            json::to_json_string(&Project {
                schema: 2,
                ..Default::default()
            }),
            " ".repeat(LIMIT + 1),
        ] {
            std::fs::write(&path, data).unwrap();
            assert!(Project::load(&path).is_err());
        }
        let mut v = json::ToJson::to_json(&Project::default());
        if let json::Value::Object(ref mut fields) = v {
            fields.push(("future".into(), json::Value::Bool(true)));
        }
        std::fs::write(&path, json::to_json_string(&v)).unwrap();
        assert!(Project::load(&path).is_err());
        std::fs::remove_file(path).unwrap();
    }
    #[test]
    fn rights_are_unknown_and_drafts_cannot_inject_sections() {
        let mut p = Project::default();
        assert_eq!(p.rights, None);
        assert_eq!(p.shareable, None);
        assert!(p.drafts().is_err());
        p.name = "x\r\n[Control]".into();
        assert!(p.findings().iter().any(|f| f.field == "Name" && f.blocking));
        p.program = "..\\outside.exe".into();
        assert!(p
            .findings()
            .iter()
            .any(|f| f.field == "Program executable" && f.blocking));
    }
}
