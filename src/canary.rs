//! Canary: post-operation health verification (read-only).
//!
//! After an uninstall, removal, or cleaning operation finishes, Canary
//! re-checks the registrations the operation could plausibly have broken
//! and reports **definite breakage only, never suspicions**:
//!
//! 1. Autostart entries whose target file no longer exists.
//! 2. Registered shell extensions whose backing DLL no longer exists.
//! 3. File associations whose open command no longer resolves to a file.
//! 4. Protected locations the operation's own records show as changed.
//!
//! Precision is the product here: every existence check goes through the
//! same machinery as the orphan scan — [`crate::orphans::target_path`]
//! (returns `None` for targets it cannot judge: bare file names, network
//! paths, unexpanded variables) and [`crate::orphans::Checker::missing`]
//! (suppresses Windows' own targets and 32-bit redirection aliases). All
//! probes are existence checks on paths; Canary never loads or executes any
//! code and uses no network.

use crate::backup::ActionResult;
use crate::leftovers::DeepIndex;
use crate::platform::Registry;
use crate::{autoruns, orphans, shellext, startup, util};
use std::collections::BTreeSet;

/// What a Canary check verifies.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum CanaryCheck {
    /// An autostart entry's target file.
    Autostart,
    /// A registered shell extension's backing DLL.
    ShellExtension,
    /// A file association's open command.
    FileAssociation,
    /// A location the product promises never to touch.
    ProtectedLocation,
}
crate::json_enum!(CanaryCheck {
    Autostart,
    ShellExtension,
    FileAssociation,
    ProtectedLocation
});

impl CanaryCheck {
    pub fn label(&self) -> &'static str {
        match self {
            CanaryCheck::Autostart => "Autostart entry",
            CanaryCheck::ShellExtension => "Shell extension",
            CanaryCheck::FileAssociation => "File association",
            CanaryCheck::ProtectedLocation => "Protected location",
        }
    }
}

/// One definite problem found by a Canary check.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CanaryFinding {
    pub check: CanaryCheck,
    /// The affected item: entry name, extension, or path.
    pub item: String,
    /// Plain-language statement of what is broken.
    pub message: String,
}
crate::json_struct!(CanaryFinding, { check, item, message });

/// A target to verify: the registration name and the command/path it points at.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CanaryTarget {
    pub name: String,
    pub target: String,
}

/// Everything Canary examines after an operation.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CanaryInputs {
    pub autostart: Vec<CanaryTarget>,
    pub shell_extensions: Vec<CanaryTarget>,
    pub associations: Vec<CanaryTarget>,
    /// File paths the operation changed or removed, from its own records.
    pub touched_files: Vec<String>,
    /// Registry keys the operation changed or removed, from its own records.
    pub touched_keys: Vec<String>,
}

/// The outcome of the Canary checks: all-clear, or the findings.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CanaryVerdict {
    pub findings: Vec<CanaryFinding>,
    /// Judgeable targets examined per check (scope of the all-clear).
    pub autostart_checked: usize,
    pub shell_extension_checked: usize,
    pub association_checked: usize,
}
crate::json_struct!(CanaryVerdict, { findings, autostart_checked, shell_extension_checked, association_checked });

impl CanaryVerdict {
    pub fn is_all_clear(&self) -> bool {
        self.findings.is_empty()
    }

    /// Plain-language line for the all-clear case.
    pub fn all_clear_message(&self) -> String {
        format!(
            "Post-operation health check: everything still works. \
             Verified {a} autostart entries, {s} shell extensions and {f} file \
             associations, and no protected locations were touched.",
            a = self.autostart_checked,
            s = self.shell_extension_checked,
            f = self.association_checked,
        )
    }

    /// The verdict as rows for the existing results surface. The existing
    /// per-operation backup (already attached to those results) is the Undo:
    /// findings point at it, Canary adds no second undo mechanism.
    pub fn to_action_results(&self) -> Vec<ActionResult> {
        if self.is_all_clear() {
            return vec![ActionResult::ok(
                "Post-operation health check",
                self.all_clear_message(),
            )];
        }
        self.findings
            .iter()
            .map(|f| {
                ActionResult::err(
                    format!("Health check: {}", f.check.label()),
                    format!("{} — {}", f.item, f.message),
                )
            })
            .collect()
    }
}

fn check_targets(
    verdict: &mut CanaryVerdict,
    seen: &mut BTreeSet<(CanaryCheck, String)>,
    check: CanaryCheck,
    targets: &[CanaryTarget],
    checker: &orphans::Checker,
    message: &str,
) -> usize {
    let mut checked = 0;
    for t in targets {
        if t.target.trim().is_empty() {
            continue;
        }
        // Only judgeable targets count; unjudgeable ones are skipped silently.
        if orphans::target_path(&t.target).is_none() {
            continue;
        }
        checked += 1;
        if checker.missing(&t.target) && seen.insert((check, t.name.clone())) {
            let missing = orphans::target_path(&t.target).unwrap_or_else(|| t.target.clone());
            verdict.findings.push(CanaryFinding {
                check,
                item: t.name.clone(),
                message: message.replace("{path}", &missing),
            });
        }
    }
    checked
}

/// Run the Canary checks over the collected inputs. Read-only.
pub fn run_checks(
    inputs: &CanaryInputs,
    checker: &orphans::Checker,
    protected: &[String],
) -> CanaryVerdict {
    let mut verdict = CanaryVerdict::default();
    let mut seen = BTreeSet::new();

    verdict.autostart_checked = check_targets(
        &mut verdict,
        &mut seen,
        CanaryCheck::Autostart,
        &inputs.autostart,
        checker,
        "The autostart entry still exists, but its target file \"{path}\" is gone.",
    );
    verdict.shell_extension_checked = check_targets(
        &mut verdict,
        &mut seen,
        CanaryCheck::ShellExtension,
        &inputs.shell_extensions,
        checker,
        "The shell extension is still registered, but its DLL \"{path}\" is gone.",
    );
    verdict.association_checked = check_targets(
        &mut verdict,
        &mut seen,
        CanaryCheck::FileAssociation,
        &inputs.associations,
        checker,
        "The file type's open command still points at \"{path}\", which is gone.",
    );

    // Protected locations: compare the operation's own records against the
    // protected set. A touched file counts when it is protected itself or
    // nested anywhere beneath a protected folder (the audit question);
    // `util::is_protected_dir` additionally covers the deletion guard's
    // folder rules.
    for path in &inputs.touched_files {
        if (util::is_protected_dir(path, protected) || touches_protected_tree(path, protected))
            && seen.insert((CanaryCheck::ProtectedLocation, path.clone()))
        {
            verdict.findings.push(CanaryFinding {
                check: CanaryCheck::ProtectedLocation,
                item: path.clone(),
                message: "This operation changed a protected location. \
                    Nothing should ever write there."
                    .to_string(),
            });
        }
    }
    for key in &inputs.touched_keys {
        if util::is_protected_key(key) && seen.insert((CanaryCheck::ProtectedLocation, key.clone()))
        {
            verdict.findings.push(CanaryFinding {
                check: CanaryCheck::ProtectedLocation,
                item: key.clone(),
                message: "This operation changed a protected registry key. Nothing should ever write there.".to_string(),
            });
        }
    }
    verdict
}

/// Whether `path` is a protected folder or nested anywhere beneath one.
/// Both sides are compared through `util::lexical_path`, so casing, slash
/// style and `..` segments cannot hide a touch. Only list entries that mark
/// a whole tree (`\dir\*`) extend below themselves: a bare entry protects
/// just that folder (files inside it may still be removed individually, per
/// the removal engine's rules), and the drive-root entry therefore does not
/// swallow the whole drive.
fn touches_protected_tree(path: &str, protected: &[String]) -> bool {
    let p = util::lexical_path(path);
    protected.iter().any(|d| {
        let d = util::lexical_path(d);
        if let Some(tree) = d.strip_suffix("\\*") {
            let root = tree.trim_end_matches('\\');
            !root.is_empty() && (p == root || p.starts_with(&format!("{root}\\")))
        } else {
            let bare = d.trim_end_matches('\\');
            !bare.is_empty() && p == bare
        }
    })
}

/// Extract what an executed plan changed or removed, from its own records,
/// split into file paths and registry keys for the protected-location check.
pub fn touched_paths(plan: &crate::revert::Plan) -> (Vec<String>, Vec<String>) {
    use crate::revert::Action;
    let (mut files, mut keys) = (Vec::new(), Vec::new());
    for item in plan.items.iter().filter(|i| i.selected) {
        match &item.action {
            Action::DeleteFile { path }
            | Action::DeleteDir { path }
            | Action::DeleteTree { path } => {
                files.push(path.clone());
            }
            Action::DeleteKey { path } => keys.push(path.clone()),
            Action::DeleteValue { key, .. } | Action::SetValue { key, .. } => {
                keys.push(key.clone())
            }
            Action::RestoreKey { key } => keys.push(key.path.clone()),
            Action::EditList { key, .. } => keys.push(key.clone()),
            // Services and tasks have no file/key path on the action itself.
            Action::StopDeleteService { .. } | Action::DeleteTask { .. } => {}
        }
    }
    files.sort();
    files.dedup();
    keys.sort();
    keys.dedup();
    (files, keys)
}

/// Collect the Canary inputs from the live system. All reads are read-only
/// registry scans and existence probes; nothing is loaded or executed.
pub fn collect_inputs(
    reg: &dyn Registry,
    ix: &DeepIndex,
    plan: &crate::revert::Plan,
) -> CanaryInputs {
    let mut inputs = CanaryInputs::default();

    // (1) Autostart entries: Run keys/Startup folders/services plus the
    // exotic autostart locations the Autoruns-style scan covers.
    for e in startup::list_startup(reg) {
        if !e.command.trim().is_empty() {
            inputs.autostart.push(CanaryTarget {
                name: e.name,
                target: e.command,
            });
        }
    }
    for e in autoruns::scan(reg, ix) {
        if !e.target.trim().is_empty() {
            inputs.autostart.push(CanaryTarget {
                name: e.name,
                target: e.target,
            });
        }
    }

    // (2) Shell extension handlers: the DLL each registered class loads.
    for e in shellext::scan(reg) {
        if e.kind == shellext::Kind::Handler && !e.target.trim().is_empty() {
            inputs.shell_extensions.push(CanaryTarget {
                name: e.name,
                target: e.target,
            });
        }
    }

    // (3) File associations: the open command of each extension's ProgID.
    let mut seen_progids = BTreeSet::new();
    for (key, progid, _) in &ix.extensions {
        let Some(progid) = progid else { continue };
        if !seen_progids.insert(crate::model::fold(progid)) {
            continue;
        }
        let ext = key.rsplit('\\').next().unwrap_or(key);
        for base in [
            "HKLM\\SOFTWARE\\Classes",
            "HKLM\\SOFTWARE\\WOW6432Node\\Classes",
            "HKCU\\Software\\Classes",
        ] {
            let path = format!("{base}\\{progid}\\shell\\open\\command");
            if let Some(command) = reg.read_key(&path).and_then(|k| k.get_string("")) {
                if !command.trim().is_empty() {
                    inputs.associations.push(CanaryTarget {
                        name: ext.to_string(),
                        target: command,
                    });
                    break;
                }
            }
        }
    }

    // (4) What the operation itself reports having changed.
    let (files, keys) = touched_paths(plan);
    inputs.touched_files = files;
    inputs.touched_keys = keys;
    inputs
}

/// Convenience entry: collect the inputs, run the checks, return the verdict.
/// `exists` is the existence probe (on real systems: `Path::new(p).exists()`).
pub fn run_from_plan(
    reg: &dyn Registry,
    ix: &DeepIndex,
    plan: &crate::revert::Plan,
    exists: &dyn Fn(&str) -> bool,
    windows_dir: &str,
    protected: &[String],
) -> CanaryVerdict {
    let checker = orphans::Checker::new(exists, windows_dir);
    let inputs = collect_inputs(reg, ix, plan);
    run_checks(&inputs, &checker, protected)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::json::{from_json_str, to_json_string, FromJson, ToJson};
    use std::collections::HashSet;

    /// Run the checks with a fabricated existence set ("C:\Windows" as the
    /// Windows folder), so tests exercise the judging logic without the disk.
    fn run(
        inputs: &CanaryInputs,
        present: &HashSet<String>,
        protected: &[String],
    ) -> CanaryVerdict {
        // The production probe is the Windows file system, which is
        // case-insensitive; mirror that here so alias paths produced with
        // different casing still count as present.
        let exists = |p: &str| {
            let f = crate::model::fold(p);
            present.iter().any(|q| crate::model::fold(q) == f)
        };
        let checker = orphans::Checker::new(&exists, "C:\\Windows");
        run_checks(inputs, &checker, protected)
    }

    fn target(name: &str, t: &str) -> CanaryTarget {
        CanaryTarget {
            name: name.to_string(),
            target: t.to_string(),
        }
    }

    #[test]
    fn all_clear_when_targets_exist() {
        let present: HashSet<String> = ["C:\\App\\tool.exe", "C:\\App\\ext.dll"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let inputs = CanaryInputs {
            autostart: vec![target("Tool", "C:\\App\\tool.exe")],
            shell_extensions: vec![target("Ext", "C:\\App\\ext.dll")],
            ..Default::default()
        };
        let v = run(&inputs, &present, &[]);
        assert!(v.is_all_clear());
        assert_eq!(v.autostart_checked, 1);
        assert_eq!(v.shell_extension_checked, 1);
        let rows = v.to_action_results();
        assert_eq!(rows.len(), 1);
        assert!(rows[0].ok);
        assert!(rows[0].message.contains("everything still works"));
    }

    #[test]
    fn missing_autostart_target_is_a_finding() {
        let present = HashSet::new();
        let inputs = CanaryInputs {
            autostart: vec![target("Helper", "\"C:\\App\\helper.exe\" --start")],
            ..Default::default()
        };
        let v = run(&inputs, &present, &[]);
        assert_eq!(v.findings.len(), 1);
        let f = &v.findings[0];
        assert_eq!(f.check, CanaryCheck::Autostart);
        assert_eq!(f.item, "Helper");
        assert!(f.message.contains("C:\\App\\helper.exe"));
        let rows = v.to_action_results();
        assert_eq!(rows.len(), 1);
        assert!(!rows[0].ok);
        assert!(rows[0].description.contains("Autostart entry"));
    }

    #[test]
    fn missing_shell_extension_and_association_are_findings() {
        let present = HashSet::new();
        let inputs = CanaryInputs {
            shell_extensions: vec![target("Zip menu", "C:\\App\\zipshell.dll")],
            associations: vec![target(".txt", "\"C:\\App\\editor.exe\" \"%1\"")],
            ..Default::default()
        };
        let v = run(&inputs, &present, &[]);
        assert_eq!(v.findings.len(), 2);
        assert!(v
            .findings
            .iter()
            .any(|f| f.check == CanaryCheck::ShellExtension));
        assert!(v
            .findings
            .iter()
            .any(|f| f.check == CanaryCheck::FileAssociation));
    }

    #[test]
    fn unjudgeable_targets_are_silently_skipped() {
        let present = HashSet::new();
        let inputs = CanaryInputs {
            autostart: vec![
                target("Bare", "foo.exe"),
                target("Env", "%SOMEAPP%\\bar.exe"),
                target("Empty", ""),
            ],
            ..Default::default()
        };
        let v = run(&inputs, &present, &[]);
        assert!(v.is_all_clear());
        assert_eq!(v.autostart_checked, 0);
    }

    #[test]
    fn windows_own_targets_are_not_reported() {
        let present = HashSet::new();
        let inputs = CanaryInputs {
            autostart: vec![target("Defender", "C:\\Windows\\System32\\foo.exe")],
            ..Default::default()
        };
        let v = run(&inputs, &present, &[]);
        assert!(v.is_all_clear());
    }

    #[test]
    fn thirty_two_bit_alias_counts_as_present() {
        let present: HashSet<String> = ["C:\\Program Files (x86)\\App\\x.exe"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let inputs = CanaryInputs {
            autostart: vec![target("X", "C:\\Program Files\\App\\x.exe")],
            ..Default::default()
        };
        let v = run(&inputs, &present, &[]);
        assert!(v.is_all_clear());
    }

    #[test]
    fn protected_locations_in_touched_records_are_findings() {
        // Build the protected list with a test environment expander (the
        // way `util`'s own tests do) so the list is usable off Windows.
        let expand = |s: &str| {
            s.replace("%SystemRoot%", "C:\\Windows")
                .replace("%SystemDrive%", "C:")
                .replace("%ProgramFiles(x86)%", "C:\\Program Files (x86)")
                .replace("%ProgramFiles%", "C:\\Program Files")
                .replace("%ProgramData%", "C:\\ProgramData")
                .replace("%USERPROFILE%", "C:\\Users\\Test")
        };
        let protected = crate::util::protected_dirs_with(&expand);
        let present = HashSet::new();
        let inputs = CanaryInputs {
            touched_files: vec![
                "C:\\Windows\\System32\\drivers\\etc\\hosts".to_string(),
                "C:\\App\\data.bin".to_string(),
            ],
            touched_keys: vec![
                "HKLM\\SYSTEM\\CurrentControlSet\\Control\\Session Manager".to_string(),
                "HKCU\\Software\\Vendor\\App".to_string(),
            ],
            ..Default::default()
        };
        let v = run(&inputs, &present, &protected);
        let protected: Vec<_> = v
            .findings
            .iter()
            .filter(|f| f.check == CanaryCheck::ProtectedLocation)
            .collect();
        assert_eq!(protected.len(), 2);
        assert!(protected.iter().any(|f| f.item.contains("drivers\\etc")));
        assert!(protected.iter().any(|f| f.item.contains("Session Manager")));
    }

    #[test]
    fn touched_paths_classifies_plan_actions() {
        use crate::revert::{Action, Plan, PlanItem};
        let item = |action: Action| PlanItem {
            action,
            selected: true,
            exists: true,
            note: String::new(),
        };
        let plan = Plan {
            items: vec![
                item(Action::DeleteFile {
                    path: "C:\\App\\a.exe".to_string(),
                }),
                item(Action::DeleteTree {
                    path: "C:\\App\\dir".to_string(),
                }),
                item(Action::DeleteKey {
                    path: "HKCU\\Software\\App".to_string(),
                }),
                item(Action::DeleteValue {
                    key: "HKCU\\Software\\App".to_string(),
                    name: "v".to_string(),
                }),
            ],
            ..Default::default()
        };
        let (files, keys) = touched_paths(&plan);
        assert_eq!(files, vec!["C:\\App\\a.exe", "C:\\App\\dir"]);
        assert_eq!(keys, vec!["HKCU\\Software\\App"]);
    }

    #[test]
    fn duplicate_targets_are_reported_once() {
        let present = HashSet::new();
        let inputs = CanaryInputs {
            autostart: vec![
                target("Helper", "C:\\App\\helper.exe"),
                target("Helper", "C:\\App\\helper.exe"),
            ],
            ..Default::default()
        };
        let v = run(&inputs, &present, &[]);
        assert_eq!(v.findings.len(), 1);
    }

    #[test]
    fn verdict_serializes_for_the_result_model() {
        let present = HashSet::new();
        let inputs = CanaryInputs {
            autostart: vec![target("Helper", "C:\\App\\helper.exe")],
            ..Default::default()
        };
        let v = run(&inputs, &present, &[]);
        // The verdict travels with the operation's results as JSON.
        let text = to_json_string(&v);
        assert!(text.contains("Helper"));
        let back: CanaryVerdict = from_json_str(&text).unwrap();
        assert_eq!(back, v);
    }
}
