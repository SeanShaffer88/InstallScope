//! Cross-program reference counting: which install logs claim which files,
//! registry keys and values, services, and scheduled tasks.
//!
//! A removal plan says what *this* removal would delete. The index built
//! here answers the question the plan alone cannot: does anything else
//! still need it? Every item gets one of three labels —
//!
//! - [`RefLabel::OnlyThisProgram`]: every log that mentions the item
//!   belongs to the program(s) being removed.
//! - [`RefLabel::Shared`]: logs of other programs mention it too, so
//!   removing it can break them.
//! - [`RefLabel::NotInAnyLog`]: no install log mentions it at all. That is
//!   an absence of evidence, not evidence it is unused — the label never
//!   claims an item is safe or unused.
//!
//! [`annotate_plan`] applies the labels to a plan and enforces the safety
//! rule: shared and unlogged items are never left auto-selected. The user
//! can still tick them by hand in the review.
//!
//! A log "claims" what its installation added or changed: files and keys
//! it added, values it added or modified, and the services and scheduled
//! tasks those records describe. Things a log records as *removed* are not
//! claims — deleting something is not owning it.

use crate::i18n::{t, tf};
use crate::model::{fold, ChangeSet, InstallLog};
use crate::revert::{service_name, tasks_dir, Action, Plan};
use std::collections::{BTreeMap, BTreeSet};

/// What the install logs say about one removal-plan item.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RefLabel {
    /// Only the program(s) being removed mention this item.
    OnlyThisProgram,
    /// Install logs of `others` other programs mention it too.
    Shared {
        /// How many other programs' logs mention the item.
        others: usize,
    },
    /// No install log mentions the item.
    NotInAnyLog,
}

impl RefLabel {
    /// The user-facing wording (also used as the review badge text).
    pub fn text(&self) -> String {
        match self {
            RefLabel::OnlyThisProgram => t("Used only by this program"),
            RefLabel::Shared { others } => tf(
                "Shared with {n} other programs",
                &[("n", others.to_string())],
            ),
            RefLabel::NotInAnyLog => t("No install log mentions this"),
        }
    }
}

// Resource keys inside the index. The one-letter namespace keeps a file
// path, a registry key, and a value that happen to share a spelling from
// ever being treated as the same resource.
fn file_key(path: &str) -> String {
    format!("f:{}", fold(path))
}

fn key_key(path: &str) -> String {
    format!("k:{}", fold(path))
}

fn value_key(key: &str, name: &str) -> String {
    // \u{1} can never appear in a real key path or value name, so the two
    // parts stay unambiguous.
    format!("v:{}\u{1}{}", fold(key), fold(name))
}

fn service_key(name: &str) -> String {
    format!("s:{}", fold(name))
}

fn task_key(name: &str) -> String {
    format!("t:{}", fold(name.trim_start_matches('\\')))
}

/// The resource keys one installation's change set claims, for one log.
fn claims_of(cs: &ChangeSet, tasks: &str) -> Vec<String> {
    let mut out = Vec::new();
    for f in &cs.files_added {
        out.push(file_key(&f.path));
        let folded = fold(&f.path);
        if !f.is_dir && folded.starts_with(tasks) {
            out.push(task_key(&folded[tasks.len()..]));
        }
    }
    for k in &cs.keys_added {
        out.push(key_key(&k.path));
        if let Some(name) = service_name(&k.path) {
            out.push(service_key(&name));
        }
        for v in k.values.values() {
            out.push(value_key(&k.path, &v.name));
        }
    }
    for v in &cs.values_added {
        out.push(key_key(&v.key));
        out.push(value_key(&v.key, &v.value.name));
    }
    for v in &cs.values_modified {
        out.push(key_key(&v.key));
        out.push(value_key(&v.key, &v.after.name));
    }
    out
}

/// An in-memory index over every install log: resource -> claiming logs.
///
/// Built fresh from the logs a caller already has (a clean rebuild is the
/// normal path, so the index can never disagree with the store);
/// [`ReferenceIndex::add_log`] and [`ReferenceIndex::remove_log`] keep it
/// correct incrementally where a caller holds it across edits.
#[derive(Clone, Debug, Default)]
pub struct ReferenceIndex {
    /// Resource key -> ids of the logs that claim it.
    claims: BTreeMap<String, BTreeSet<String>>,
    /// Log id -> program name, for display.
    names: BTreeMap<String, String>,
}

impl ReferenceIndex {
    /// Build the index from all known install logs.
    pub fn build(logs: &[InstallLog]) -> ReferenceIndex {
        let mut index = ReferenceIndex::default();
        for log in logs {
            index.add_log(log);
        }
        index
    }

    /// Add one log's claims (replacing any earlier claims for its id, so
    /// re-adding a merged or edited log stays correct).
    pub fn add_log(&mut self, log: &InstallLog) {
        self.remove_log(&log.id);
        let tasks = tasks_dir();
        for resource in claims_of(&log.changes, &tasks) {
            self.claims
                .entry(resource)
                .or_default()
                .insert(log.id.clone());
        }
        self.names.insert(log.id.clone(), log.name.clone());
    }

    /// Drop one log's claims. Returns whether the log was present.
    pub fn remove_log(&mut self, log_id: &str) -> bool {
        if self.names.remove(log_id).is_none() {
            return false;
        }
        self.claims.retain(|_, ids| {
            ids.remove(log_id);
            !ids.is_empty()
        });
        true
    }

    /// How many install logs the index covers.
    pub fn log_count(&self) -> usize {
        self.names.len()
    }

    /// The display name recorded for a log id, if the log is indexed.
    pub fn program_name(&self, log_id: &str) -> Option<&str> {
        self.names.get(log_id).map(String::as_str)
    }

    /// "Reference data covers X of your Y programs", using the real
    /// counts. `None` when there is no meaningful total to report.
    pub fn coverage_line(&self, total_programs: usize) -> Option<String> {
        if total_programs == 0 {
            return None;
        }
        let covered = self.log_count().min(total_programs);
        Some(tf(
            "Reference data covers {covered} of your {total} programs",
            &[
                ("covered", covered.to_string()),
                ("total", total_programs.to_string()),
            ],
        ))
    }

    fn gather(&self, out: &mut BTreeSet<String>, resource: &str) {
        if let Some(ids) = self.claims.get(resource) {
            out.extend(ids.iter().cloned());
        }
    }

    /// Claimants of a resource and of everything recorded underneath it
    /// (a folder's contents, a key's subkeys).
    fn gather_tree(&self, out: &mut BTreeSet<String>, ns: &str, folded_path: &str) {
        self.gather(out, &format!("{ns}{folded_path}"));
        let prefix = format!("{ns}{folded_path}\\");
        for (k, ids) in self.claims.range(prefix.clone()..) {
            if !k.starts_with(&prefix) {
                break;
            }
            out.extend(ids.iter().cloned());
        }
    }

    /// Claimants of the values recorded in a key or any of its subkeys.
    /// A value key is `v:<key>\u{1}<name>`; a subkey's values start with
    /// `v:<key>\` instead, and neither can collide with a longer sibling
    /// key's values (those continue with more path text, not a separator).
    fn gather_values(&self, out: &mut BTreeSet<String>, folded_key: &str) {
        for sep in ['\u{1}', '\\'] {
            let prefix = format!("v:{folded_key}{sep}");
            for (k, ids) in self.claims.range(prefix.clone()..) {
                if !k.starts_with(&prefix) {
                    break;
                }
                out.extend(ids.iter().cloned());
            }
        }
    }

    /// Every log that claims the resource an action would remove.
    fn claimants(&self, action: &Action) -> BTreeSet<String> {
        let mut out = BTreeSet::new();
        match action {
            Action::DeleteFile { path } => {
                self.gather(&mut out, &file_key(path));
                let tasks = tasks_dir();
                let folded = fold(path);
                if folded.starts_with(&tasks) {
                    self.gather(&mut out, &task_key(&folded[tasks.len()..]));
                }
            }
            Action::DeleteDir { path } | Action::DeleteTree { path } => {
                self.gather_tree(&mut out, "f:", &fold(path));
            }
            Action::DeleteKey { path } => {
                let fp = fold(path);
                self.gather_tree(&mut out, "k:", &fp);
                self.gather_values(&mut out, &fp);
                if let Some(name) = service_name(path) {
                    self.gather(&mut out, &service_key(&name));
                }
            }
            Action::DeleteValue { key, name } | Action::EditList { key, name, .. } => {
                self.gather(&mut out, &value_key(key, name));
                if out.is_empty() {
                    // No log itemizes this value; fall back to who owns
                    // the key it lives in.
                    self.gather(&mut out, &key_key(key));
                }
            }
            Action::SetValue { key, value } => {
                self.gather(&mut out, &value_key(key, &value.name));
                if out.is_empty() {
                    self.gather(&mut out, &key_key(key));
                }
            }
            Action::RestoreKey { key } => {
                let fp = fold(&key.path);
                self.gather_tree(&mut out, "k:", &fp);
                self.gather_values(&mut out, &fp);
            }
            Action::StopDeleteService { name } => {
                self.gather(&mut out, &service_key(name));
                if out.is_empty() {
                    let key = format!("HKLM\\SYSTEM\\CurrentControlSet\\Services\\{name}");
                    self.gather(&mut out, &key_key(&key));
                }
            }
            Action::DeleteTask { name } => {
                self.gather(&mut out, &task_key(name));
                if out.is_empty() {
                    let path = format!("{}{}", tasks_dir(), name.trim_start_matches('\\'));
                    self.gather(&mut out, &file_key(&path));
                }
            }
        }
        out
    }

    /// Label one action against the set of logs being removed.
    pub fn label(&self, action: &Action, removing: &BTreeSet<String>) -> RefLabel {
        let claimants = self.claimants(action);
        if claimants.is_empty() {
            return RefLabel::NotInAnyLog;
        }
        let others = claimants
            .iter()
            .filter(|id| !removing.contains(*id))
            .count();
        if others == 0 {
            RefLabel::OnlyThisProgram
        } else {
            RefLabel::Shared { others }
        }
    }

    /// Label every item of a plan, in item order.
    pub fn labels(&self, plan: &Plan, removing: &BTreeSet<String>) -> Vec<RefLabel> {
        plan.items
            .iter()
            .map(|i| self.label(&i.action, removing))
            .collect()
    }
}

/// Label every item of `plan` from the index and enforce the safety rule:
/// items that are shared with other programs, or that no install log
/// mentions, are never left auto-selected. Returns the labels, parallel to
/// `plan.items`.
pub fn annotate_plan(
    plan: &mut Plan,
    index: &ReferenceIndex,
    removing: &BTreeSet<String>,
) -> Vec<RefLabel> {
    let labels = index.labels(plan, removing);
    for (item, label) in plan.items.iter_mut().zip(&labels) {
        if !matches!(label, RefLabel::OnlyThisProgram) {
            item.selected = false;
        }
    }
    labels
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{FileEntry, KeyData, RegValue, ValueChange, ValueRef};
    use crate::revert::PlanItem;

    const SHARED_DLL: &str = "C:\\Program Files\\Vendor\\Shared\\runtime.dll";
    const CLSID: &str = "HKLM\\SOFTWARE\\Classes\\CLSID\\{11111111-2222-3333-4444-555555555555}";

    fn file(path: &str) -> FileEntry {
        FileEntry {
            path: path.into(),
            is_dir: false,
            size: 10,
            mtime: 0,
        }
    }

    fn dir(path: &str) -> FileEntry {
        FileEntry {
            is_dir: true,
            ..file(path)
        }
    }

    /// Program A: owns its folder, the shared DLL, and a COM class.
    fn log_a() -> InstallLog {
        let mut cs = ChangeSet::default();
        cs.files_added = vec![
            dir("C:\\Program Files\\Vendor\\AppA"),
            file("C:\\Program Files\\Vendor\\AppA\\appa.exe"),
            file(SHARED_DLL),
        ];
        cs.keys_added = vec![
            KeyData::new("HKCU\\Software\\Vendor\\AppA").with(RegValue::string("Version", "1.0")),
            KeyData::new(&format!("{CLSID}\\InprocServer32"))
                .with(RegValue::string("", SHARED_DLL)),
        ];
        cs.values_added = vec![ValueRef {
            key: "HKCU\\Software\\Vendor\\AppA".into(),
            value: RegValue::string("InstallDir", "C:\\Program Files\\Vendor\\AppA"),
        }];
        InstallLog {
            id: "a".into(),
            name: "App A".into(),
            changes: cs,
            ..Default::default()
        }
    }

    /// Program B: also ships the shared DLL and registers the same COM class.
    fn log_b() -> InstallLog {
        let mut cs = ChangeSet::default();
        cs.files_added = vec![file(SHARED_DLL)];
        cs.keys_added = vec![
            KeyData::new(&format!("{CLSID}\\InprocServer32"))
                .with(RegValue::string("", SHARED_DLL)),
            KeyData::new("HKLM\\SYSTEM\\CurrentControlSet\\Services\\BSvc"),
        ];
        cs.values_modified = vec![ValueChange {
            key: "HKCU\\Software\\Vendor\\AppA".into(),
            before: RegValue::string("SharedFlag", "0"),
            after: RegValue::string("SharedFlag", "1"),
        }];
        InstallLog {
            id: "b".into(),
            name: "App B".into(),
            changes: cs,
            ..Default::default()
        }
    }

    fn removing(ids: &[&str]) -> BTreeSet<String> {
        ids.iter().map(|s| s.to_string()).collect()
    }

    fn item(action: Action) -> PlanItem {
        PlanItem {
            action,
            selected: true,
            exists: true,
            note: String::new(),
        }
    }

    #[test]
    fn labels_only_shared_and_unknown() {
        let index = ReferenceIndex::build(&[log_a(), log_b()]);
        let rm = removing(&["a"]);
        // A's own file: only A mentions it.
        assert_eq!(
            index.label(
                &Action::DeleteFile {
                    path: "c:\\program files\\vendor\\appa\\appa.exe".into()
                },
                &rm
            ),
            RefLabel::OnlyThisProgram
        );
        // The shared DLL: B mentions it too.
        assert_eq!(
            index.label(
                &Action::DeleteFile {
                    path: SHARED_DLL.into()
                },
                &rm
            ),
            RefLabel::Shared { others: 1 }
        );
        // Nothing mentions a stray file.
        assert_eq!(
            index.label(
                &Action::DeleteFile {
                    path: "C:\\Temp\\stray.tmp".into()
                },
                &rm
            ),
            RefLabel::NotInAnyLog
        );
        // Removing both programs: the DLL is no longer shared.
        let rm = removing(&["a", "b"]);
        assert_eq!(
            index.label(
                &Action::DeleteFile {
                    path: SHARED_DLL.into()
                },
                &rm
            ),
            RefLabel::OnlyThisProgram
        );
    }

    #[test]
    fn shared_subtree_and_values() {
        let index = ReferenceIndex::build(&[log_a(), log_b()]);
        let rm = removing(&["a"]);
        // Deleting the COM class key removes a subtree B also registered.
        assert_eq!(
            index.label(&Action::DeleteKey { path: CLSID.into() }, &rm),
            RefLabel::Shared { others: 1 }
        );
        // A value B modified inside A's key is shared...
        assert_eq!(
            index.label(
                &Action::DeleteValue {
                    key: "HKCU\\Software\\Vendor\\AppA".into(),
                    name: "SharedFlag".into()
                },
                &rm
            ),
            RefLabel::Shared { others: 1 }
        );
        // ...while a value only A set is A's alone.
        assert_eq!(
            index.label(
                &Action::DeleteValue {
                    key: "HKCU\\Software\\Vendor\\AppA".into(),
                    name: "InstallDir".into()
                },
                &rm
            ),
            RefLabel::OnlyThisProgram
        );
        // A value no log itemizes falls back to the key's owner.
        assert_eq!(
            index.label(
                &Action::DeleteValue {
                    key: "HKCU\\Software\\Vendor\\AppA".into(),
                    name: "Unrecorded".into()
                },
                &rm
            ),
            RefLabel::Shared { others: 1 } // B modified a value in this key
        );
        assert_eq!(
            index.label(
                &Action::DeleteValue {
                    key: "HKCU\\Software\\Vendor\\AppA".into(),
                    name: "Unrecorded".into()
                },
                &removing(&["a", "b"])
            ),
            RefLabel::OnlyThisProgram
        );
    }

    #[test]
    fn service_claims() {
        let index = ReferenceIndex::build(&[log_a(), log_b()]);
        assert_eq!(
            index.label(
                &Action::StopDeleteService {
                    name: "BSvc".into()
                },
                &removing(&["b"])
            ),
            RefLabel::OnlyThisProgram
        );
        assert_eq!(
            index.label(
                &Action::StopDeleteService {
                    name: "bsvc".into()
                },
                &removing(&["a"])
            ),
            RefLabel::Shared { others: 1 }
        );
    }

    #[test]
    fn add_and_remove_update_labels() {
        let mut index = ReferenceIndex::build(&[log_a()]);
        let action = Action::DeleteFile {
            path: SHARED_DLL.into(),
        };
        let rm = removing(&["a"]);
        assert_eq!(index.label(&action, &rm), RefLabel::OnlyThisProgram);
        index.add_log(&log_b());
        assert_eq!(index.label(&action, &rm), RefLabel::Shared { others: 1 });
        assert!(index.remove_log("b"));
        assert_eq!(index.label(&action, &rm), RefLabel::OnlyThisProgram);
        assert!(index.remove_log("a"));
        assert_eq!(index.label(&action, &rm), RefLabel::NotInAnyLog);
        assert!(!index.remove_log("a"));
        assert_eq!(index.log_count(), 0);
    }

    #[test]
    fn re_adding_a_log_replaces_its_claims() {
        let mut index = ReferenceIndex::build(&[log_a()]);
        let mut shrunk = log_a();
        shrunk.changes.files_added.clear();
        index.add_log(&shrunk);
        assert_eq!(
            index.label(
                &Action::DeleteFile {
                    path: SHARED_DLL.into()
                },
                &removing(&["a"])
            ),
            RefLabel::NotInAnyLog
        );
    }

    #[test]
    fn annotate_deselects_shared_and_unknown() {
        let index = ReferenceIndex::build(&[log_a(), log_b()]);
        let mut plan = Plan {
            items: vec![
                item(Action::DeleteFile {
                    path: "C:\\Program Files\\Vendor\\AppA\\appa.exe".into(),
                }),
                item(Action::DeleteFile {
                    path: SHARED_DLL.into(),
                }),
                item(Action::DeleteFile {
                    path: "C:\\Temp\\stray.tmp".into(),
                }),
            ],
            info: Vec::new(),
        };
        let labels = annotate_plan(&mut plan, &index, &removing(&["a"]));
        assert_eq!(
            labels,
            vec![
                RefLabel::OnlyThisProgram,
                RefLabel::Shared { others: 1 },
                RefLabel::NotInAnyLog
            ]
        );
        assert!(plan.items[0].selected);
        assert!(!plan.items[1].selected);
        assert!(!plan.items[2].selected);
    }

    #[test]
    fn task_claims_follow_the_tasks_folder() {
        // Derive the path from the same helper production code uses, so
        // the test does not depend on a Windows environment.
        let tasks = tasks_dir();
        let mut cs = ChangeSet::default();
        cs.files_added = vec![file(&format!("{tasks}Vendor\\NightlyJob"))];
        let log = InstallLog {
            id: "t".into(),
            name: "Task App".into(),
            changes: cs,
            ..Default::default()
        };
        let index = ReferenceIndex::build(&[log]);
        assert_eq!(
            index.label(
                &Action::DeleteTask {
                    name: "\\Vendor\\NightlyJob".into()
                },
                &removing(&["t"])
            ),
            RefLabel::OnlyThisProgram
        );
    }

    #[test]
    fn coverage_line_uses_real_counts() {
        let index = ReferenceIndex::build(&[log_a(), log_b()]);
        assert_eq!(index.log_count(), 2);
        let line = index.coverage_line(30).unwrap();
        assert!(line.contains('2'), "line was: {line}");
        assert!(line.contains("30"), "line was: {line}");
        assert!(index.coverage_line(0).is_none());
        assert_eq!(index.program_name("b"), Some("App B"));
    }
}
