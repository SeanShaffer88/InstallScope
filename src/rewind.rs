//! App Rewind (early prototype): per-program time travel.
//!
//! An install log records *that* files changed and what the registry looked
//! like before, but not file *contents*. This module adds the missing piece:
//! a content-addressed store of file contents captured before a program's
//! files change, so one program can be rewound to a recorded state — its
//! files restored byte-for-byte and its registry restored from the log's
//! before-images — without touching the rest of the PC.
//!
//! Honesty rules (ROADMAP, Innovation track): a rewind is only ever
//! reported complete when everything the chosen state records can actually
//! be restored. Anything without a recorded before-state is listed as a
//! gap, by name, before anything runs.
//!
//! Store layout, under `<data folder>/rewind/`:
//! - `blobs/<first 2 hex chars>/<sha256 hex>` — file contents, named by
//!   their SHA-256, stored once however many states reference them.
//! - `index.json` — the capture sets (which log, which state, which files,
//!   which blobs) and the eviction record. Versioned like every other
//!   persistent format in the product.
//!
//! ## The monitoring hook point (not wired in this prototype)
//!
//! Before-contents only exist *before* the change happens, so capture must
//! run before an installer does. The precise hook: in the monitoring flow,
//! after the before-snapshot is taken and the pending session is saved
//! (`Store::save_pending` in `src/cli.rs`'s `monitor` command and the GUI
//! equivalent), call [`capture_before_images`] for the program being
//! updated — in practice the file scope of that program's existing install
//! log — and bind the resulting set to the new log's id when the session is
//! saved as a log. Until that wiring lands, capture is explicit:
//! `installscope rewind <log-id> --capture [--before]`.

use crate::backup::{ActionResult, BackupWriter};
use crate::model::{fold, hexbytes, InstallLog};
use crate::platform::Registry;
use crate::revert::{self, ExecOptions, Plan};
use anyhow::{Context, Result};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

/// What a capture set holds contents for, and when it was taken.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CaptureKind {
    /// Contents as they were *before* the log's recorded changes: the
    /// before-state that makes rewinding this log possible.
    Before,
    /// Contents as they are now, recorded on request (`--capture`).
    Snapshot,
    /// Contents as they were immediately before a rewind ran, recorded
    /// automatically so the rewind itself can be travelled back from.
    PreRewind,
}
crate::json_enum!(CaptureKind {
    Before,
    Snapshot,
    PreRewind
});

impl CaptureKind {
    pub fn label(self) -> &'static str {
        match self {
            CaptureKind::Before => "before-state",
            CaptureKind::Snapshot => "snapshot",
            CaptureKind::PreRewind => "pre-rewind snapshot",
        }
    }
}

/// One file whose contents are in the store.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CapturedFile {
    /// Path as recorded (original spelling).
    pub path: String,
    /// SHA-256 of the contents, hex — the blob's name in the store.
    pub blob: String,
    pub size: u64,
}
crate::json_struct!(CapturedFile, { path, blob, size });

/// The contents of one program's files at one moment: a recorded state.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CaptureSet {
    /// `"before"` for the before-state, `snap-<id>` for snapshots.
    pub id: String,
    pub log_id: String,
    pub kind: CaptureKind,
    /// When the contents were captured (local time).
    pub created: String,
    pub files: Vec<CapturedFile>,
}
crate::json_struct!(CaptureSet, { id, log_id, kind, created, files });

impl CaptureSet {
    pub fn bytes(&self) -> u64 {
        self.files.iter().map(|f| f.size).sum()
    }
    pub fn find(&self, path: &str) -> Option<&CapturedFile> {
        let f = fold(path);
        self.files.iter().find(|c| fold(&c.path) == f)
    }
}

/// A capture set removed to stay within the storage budget. Kept in the
/// index so eviction is visible after the fact, never silent.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EvictionRecord {
    pub set_id: String,
    pub log_id: String,
    pub kind: CaptureKind,
    pub files: u32,
    pub bytes: u64,
    pub evicted_at: String,
}
crate::json_struct!(EvictionRecord, { set_id, log_id, kind, files, bytes, evicted_at });

#[derive(Clone, Debug, PartialEq)]
pub struct ContentIndex {
    pub version: u32,
    pub sets: Vec<CaptureSet>,
    pub evictions: Vec<EvictionRecord>,
}
crate::json_struct!(ContentIndex, default, { version, sets, evictions });

impl Default for ContentIndex {
    fn default() -> Self {
        ContentIndex {
            version: 1,
            sets: Vec::new(),
            evictions: Vec::new(),
        }
    }
}

/// Which of a log's files a capture reads from disk.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CaptureScope {
    /// The files that existed before the log's changes: the before-images
    /// of modified files, and the files the changes deleted.
    BeforeImages,
    /// Every file the log names (added, modified, removed): the program's
    /// current footprint, for snapshot and pre-rewind captures.
    WholeLog,
}

fn scope_paths(log: &InstallLog, scope: CaptureScope) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut out: Vec<String> = Vec::new();
    let mut push = |p: &str, out: &mut Vec<String>| {
        if seen.insert(fold(p)) {
            out.push(p.to_string());
        }
    };
    match scope {
        CaptureScope::BeforeImages => {
            for c in &log.changes.files_modified {
                push(&c.before.path, &mut out);
            }
            for f in &log.changes.files_removed {
                push(&f.path, &mut out);
            }
        }
        CaptureScope::WholeLog => {
            for f in &log.changes.files_added {
                if !f.is_dir {
                    push(&f.path, &mut out);
                }
            }
            for c in &log.changes.files_modified {
                push(&c.after.path, &mut out);
            }
            for f in &log.changes.files_removed {
                push(&f.path, &mut out);
            }
        }
    }
    out
}

/// The outcome of one capture: what was recorded, what could not be, and
/// what the budget forced out.
#[derive(Clone, Debug, Default)]
pub struct CaptureReport {
    pub set: Option<CaptureSet>,
    /// Paths that were in scope but not captured, each with its reason.
    pub skipped: Vec<String>,
    pub evicted: Vec<EvictionRecord>,
    /// Store usage after the capture.
    pub bytes_used: u64,
    pub budget: u64,
}

/// The content store: blobs on disk plus the index that gives them meaning.
pub struct ContentStore {
    dir: PathBuf,
    index: ContentIndex,
}

impl ContentStore {
    /// Open (creating if needed) the store inside a data folder.
    pub fn open(data_root: &Path) -> Result<ContentStore> {
        let dir = data_root.join("rewind");
        std::fs::create_dir_all(dir.join("blobs"))
            .with_context(|| format!("create {}", dir.display()))?;
        let index = match std::fs::read_to_string(dir.join("index.json")) {
            Ok(s) => crate::json::compat::from_str(&s).unwrap_or_default(),
            Err(_) => ContentIndex::default(),
        };
        Ok(ContentStore { dir, index })
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    pub fn index(&self) -> &ContentIndex {
        &self.index
    }

    fn blob_path(&self, hash: &str) -> PathBuf {
        let (fan, rest) = hash.split_at(2.min(hash.len()));
        self.dir.join("blobs").join(fan).join(rest)
    }

    pub fn has_blob(&self, hash: &str) -> bool {
        self.blob_path(hash).is_file()
    }

    /// True when the blob exists and its contents still hash to its name —
    /// a damaged store is detected here, never silently restored from.
    pub fn verify_blob(&self, hash: &str) -> bool {
        match std::fs::read(self.blob_path(hash)) {
            Ok(bytes) => hexbytes::to_hex(&crate::util::sha256(&bytes)) == hash,
            Err(_) => false,
        }
    }

    /// Read a blob, verifying its contents against its name.
    pub fn blob_bytes(&self, hash: &str) -> Result<Vec<u8>> {
        let bytes = std::fs::read(self.blob_path(hash))
            .with_context(|| format!("read content-store blob {hash}"))?;
        if hexbytes::to_hex(&crate::util::sha256(&bytes)) != hash {
            anyhow::bail!("content-store blob {hash} is damaged (contents do not match its name)");
        }
        Ok(bytes)
    }

    fn store_blob(&self, data: &[u8]) -> Result<String> {
        let hash = hexbytes::to_hex(&crate::util::sha256(data));
        let path = self.blob_path(&hash);
        if !path.exists() {
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            // Staged write: a crash mid-capture never leaves a torn blob
            // under its final name.
            let tmp = path.with_extension("tmp");
            std::fs::write(&tmp, data)?;
            std::fs::rename(&tmp, &path)?;
        }
        Ok(hash)
    }

    /// Total bytes of blobs still referenced by at least one capture set
    /// (a blob shared by several sets is counted once).
    pub fn referenced_bytes(&self) -> u64 {
        let mut sizes: HashMap<&str, u64> = HashMap::new();
        for set in &self.index.sets {
            for f in &set.files {
                sizes.insert(&f.blob, f.size);
            }
        }
        sizes.values().sum()
    }

    pub fn sets_for(&self, log_id: &str) -> Vec<&CaptureSet> {
        self.index
            .sets
            .iter()
            .filter(|s| s.log_id == log_id)
            .collect()
    }

    pub fn before_set(&self, log_id: &str) -> Option<&CaptureSet> {
        self.index
            .sets
            .iter()
            .find(|s| s.log_id == log_id && s.kind == CaptureKind::Before)
    }

    pub fn get_set(&self, log_id: &str, set_id: &str) -> Option<&CaptureSet> {
        self.index
            .sets
            .iter()
            .find(|s| s.log_id == log_id && s.id == set_id)
    }

    /// Capture the current contents of a log's files as a new recorded
    /// state, then bring the store back within `budget`.
    ///
    /// A `Before` capture replaces any previous before-state for the log
    /// (re-capturing is an explicit act); snapshots accumulate. Files that
    /// cannot be read are reported in `skipped`, never silently dropped.
    pub fn capture(
        &mut self,
        log: &InstallLog,
        kind: CaptureKind,
        scope: CaptureScope,
        budget: u64,
    ) -> Result<CaptureReport> {
        let mut report = CaptureReport {
            budget,
            ..Default::default()
        };
        if kind == CaptureKind::Before {
            self.index
                .sets
                .retain(|s| !(s.log_id == log.id && s.kind == CaptureKind::Before));
        }
        let mut files = Vec::new();
        for path in scope_paths(log, scope) {
            let p = Path::new(&path);
            let meta = match std::fs::metadata(p) {
                Ok(m) => m,
                Err(_) => {
                    report.skipped.push(format!("{path} (not present)"));
                    continue;
                }
            };
            if meta.is_dir() {
                report.skipped.push(format!("{path} (a folder)"));
                continue;
            }
            if meta.len() > budget {
                report.skipped.push(format!(
                    "{path} ({} — larger than the whole Rewind storage budget)",
                    crate::model::human_size(meta.len())
                ));
                continue;
            }
            match std::fs::read(p) {
                Ok(bytes) => {
                    let size = bytes.len() as u64;
                    let blob = self.store_blob(&bytes)?;
                    files.push(CapturedFile { path, blob, size });
                }
                Err(e) => report.skipped.push(format!("{path} ({e})")),
            }
        }
        let set = CaptureSet {
            id: if kind == CaptureKind::Before {
                "before".to_string()
            } else {
                format!("snap-{}", crate::util::new_id())
            },
            log_id: log.id.clone(),
            kind,
            created: crate::util::now_string(),
            files,
        };
        report.set = Some(set.clone());
        self.index.sets.push(set);
        report.evicted = self.enforce_budget(budget)?;
        report.bytes_used = self.referenced_bytes();
        self.save_index()?;
        Ok(report)
    }

    /// Capture the before-images a rewind of this log needs (the contents
    /// of files the log's changes modified or deleted). Must run *before*
    /// those changes happen — see the module docs for the hook point.
    pub fn capture_before_images(
        &mut self,
        log: &InstallLog,
        budget: u64,
    ) -> Result<CaptureReport> {
        self.capture(log, CaptureKind::Before, CaptureScope::BeforeImages, budget)
    }

    /// Oldest-first eviction down to `budget`. Never evicts a before-state
    /// or the newest set of any log — a program always keeps its most
    /// recent recorded state. Every eviction is recorded in the index.
    fn enforce_budget(&mut self, budget: u64) -> Result<Vec<EvictionRecord>> {
        let mut evicted = Vec::new();
        while self.referenced_bytes() > budget {
            // The newest set of each log (sets are appended in time order).
            let mut newest: HashMap<&str, usize> = HashMap::new();
            for (i, s) in self.index.sets.iter().enumerate() {
                newest.insert(&s.log_id, i);
            }
            let victim = self.index.sets.iter().enumerate().find(|(i, s)| {
                s.kind != CaptureKind::Before && newest.get(s.log_id.as_str()) != Some(i)
            });
            let Some((at, _)) = victim else {
                break; // only protected sets remain; the store stays over budget and says so
            };
            let set = self.index.sets.remove(at);
            let rec = EvictionRecord {
                files: set.files.len() as u32,
                bytes: set.bytes(),
                set_id: set.id.clone(),
                log_id: set.log_id.clone(),
                kind: set.kind,
                evicted_at: crate::util::now_string(),
            };
            // Delete blobs no remaining set references.
            let still: HashSet<&str> = self
                .index
                .sets
                .iter()
                .flat_map(|s| s.files.iter().map(|f| f.blob.as_str()))
                .collect();
            for f in &set.files {
                if !still.contains(f.blob.as_str()) {
                    let _ = std::fs::remove_file(self.blob_path(&f.blob));
                }
            }
            self.index.evictions.push(rec.clone());
            evicted.push(rec);
        }
        if !evicted.is_empty() {
            self.save_index()?;
        }
        Ok(evicted)
    }

    fn save_index(&self) -> Result<()> {
        let json = crate::json::compat::to_string_pretty(&self.index)?;
        let tmp = self.dir.join("index.json.tmp");
        std::fs::write(&tmp, json)?;
        std::fs::rename(&tmp, self.dir.join("index.json"))?;
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Recorded states and rewind planning
// ---------------------------------------------------------------------------

/// One recorded state of a program, as shown by `rewind --list`.
#[derive(Clone, Debug)]
pub struct StateInfo {
    pub id: String,
    pub label: String,
    /// When the state was recorded (the log's creation for `before`).
    pub created: String,
    /// Whether the state's registry side comes from the log's before-images.
    pub registry: bool,
    /// Files in scope that have recorded contents.
    pub files_with_content: usize,
    /// Files in scope with no recorded contents (gaps for this state).
    pub files_without_content: usize,
}

/// Every recorded state of a log: its before-state first, then snapshots
/// oldest to newest.
pub fn list_states(store: &ContentStore, log: &InstallLog) -> Vec<StateInfo> {
    let mut out = Vec::new();
    let scope = scope_paths(log, CaptureScope::BeforeImages);
    let before = store.before_set(&log.id);
    let covered = scope
        .iter()
        .filter(|p| before.is_some_and(|s| s.find(p).is_some()))
        .count();
    out.push(StateInfo {
        id: "before".to_string(),
        label: format!("Before \u{201c}{}\u{201d} was recorded", log.name),
        created: log.created.clone(),
        registry: true,
        files_with_content: covered,
        files_without_content: scope.len() - covered,
    });
    for set in store.sets_for(&log.id) {
        if set.kind == CaptureKind::Before {
            continue;
        }
        out.push(StateInfo {
            id: set.id.clone(),
            label: format!("File contents {}", set.kind.label()),
            created: set.created.clone(),
            registry: false,
            files_with_content: set.files.len(),
            files_without_content: 0,
        });
    }
    out
}

/// One file whose contents a rewind will put back.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileRestore {
    pub path: String,
    /// The blob holding the recorded contents.
    pub blob: String,
    pub size: u64,
    /// The file is there now and will be overwritten (otherwise recreated).
    pub exists_now: bool,
}

impl FileRestore {
    pub fn describe(&self) -> String {
        format!(
            "{} {} ({})",
            if self.exists_now {
                "Restore file"
            } else {
                "Recreate file"
            },
            self.path,
            crate::model::human_size(self.size)
        )
    }
}

/// A complete rewind plan: everything the rewind will do, and — just as
/// important — everything it cannot do, named in `gaps`.
#[derive(Clone, Debug)]
pub struct RewindPlan {
    pub log_id: String,
    pub state_id: String,
    pub state_label: String,
    /// The existing removal machinery's plan for the registry before-images
    /// and the deletion of files the changes added. Empty for snapshot
    /// states (which record file contents only).
    pub revert_plan: Plan,
    pub file_restores: Vec<FileRestore>,
    /// "cannot restore X — …" lines. A plan with gaps is never complete.
    pub gaps: Vec<String>,
    pub notes: Vec<String>,
}

impl RewindPlan {
    /// True only when every item the state records can be restored. A
    /// partial rewind is never presented as complete.
    pub fn complete(&self) -> bool {
        self.gaps.is_empty()
    }

    /// The plan as printable lines (used by the CLI for `--dry-run` and as
    /// the pre-execution review).
    pub fn describe_lines(&self) -> Vec<String> {
        let mut lines = vec![format!(
            "Rewind to: {} [{}]",
            self.state_label, self.state_id
        )];
        for n in &self.notes {
            lines.push(format!("  {n}"));
        }
        if !self.revert_plan.items.is_empty() {
            lines.push("From the install log:".to_string());
            for i in &self.revert_plan.items {
                if i.selected {
                    lines.push(format!("  {}", i.action.describe()));
                }
            }
        }
        if !self.file_restores.is_empty() {
            lines.push("File contents from the content store:".to_string());
            for r in &self.file_restores {
                lines.push(format!("  {}", r.describe()));
            }
        }
        if !self.gaps.is_empty() {
            lines.push("Cannot be restored:".to_string());
            for g in &self.gaps {
                lines.push(format!("  {g}"));
            }
        }
        lines.push(if self.complete() {
            "Verdict: COMPLETE — everything this state records can be restored.".to_string()
        } else {
            format!(
                "Verdict: PARTIAL — {} item(s) cannot be restored. \
                 A partial rewind is never reported as a complete one.",
                self.gaps.len()
            )
        });
        lines
    }
}

/// Build the rewind plan for one recorded state of a log.
///
/// The registry side and the deletion of added files reuse
/// [`revert::build_plan`]: the same reviewed machinery that uninstalls.
/// File *contents* come from the content store — the part the removal
/// engine deliberately does not do (its plan reports modified files as
/// "left as is" and deleted files as "cannot be restored"; here those
/// become restores, or explicit gaps when no content was captured).
pub fn build_rewind_plan(
    store: &ContentStore,
    log: &InstallLog,
    state_id: &str,
    reg: &dyn Registry,
) -> Result<RewindPlan> {
    let mut plan = RewindPlan {
        log_id: log.id.clone(),
        state_id: state_id.to_string(),
        state_label: String::new(),
        revert_plan: Plan::default(),
        file_restores: Vec::new(),
        gaps: Vec::new(),
        notes: Vec::new(),
    };

    if state_id == "before" {
        plan.state_label = format!("Before \u{201c}{}\u{201d} was recorded", log.name);
        plan.notes.push(
            "Registry is restored from the install log's recorded before-images; \
             files the installation added are removed."
                .to_string(),
        );
        plan.notes.push(
            "Only the changes recorded in this install log are reversed. Changes \
             from other installs, later updates or by hand are not touched."
                .to_string(),
        );
        // The revert machinery handles everything except file contents.
        // Emptying files_modified/files_removed in the copy stops it from
        // reporting them as unrestorable — this plan handles them below.
        let mut cs = log.changes.clone();
        cs.files_modified.clear();
        cs.files_removed.clear();
        plan.revert_plan = revert::build_plan(&cs, reg);
        // Anything the machinery left unselected, with a reason, will not
        // in fact be reversed — say so rather than implying it will be.
        for item in plan.revert_plan.items.clone() {
            if !item.selected && !item.note.is_empty() {
                plan.gaps.push(format!(
                    "cannot restore {} — {}",
                    item.action.describe(),
                    lowercase_first(&item.note)
                ));
            }
        }
        for i in plan.revert_plan.info.clone() {
            plan.notes.push(i);
        }

        let before = store.before_set(&log.id);
        let mut restore = |path: &str, plan: &mut RewindPlan, what: &str| match before
            .and_then(|s| s.find(path))
        {
            Some(c) if store.verify_blob(&c.blob) => {
                plan.file_restores.push(FileRestore {
                    path: path.to_string(),
                    blob: c.blob.clone(),
                    size: c.size,
                    exists_now: Path::new(path).exists(),
                });
            }
            Some(_) => plan.gaps.push(format!(
                "cannot restore {path} — its recorded before-content is missing \
                     or damaged in the content store"
            )),
            None => plan.gaps.push(format!(
                "cannot restore {path} — no recorded before-state \
                     ({what} was not captured)"
            )),
        };
        for c in &log.changes.files_modified {
            restore(&c.before.path, &mut plan, "file content");
        }
        for f in &log.changes.files_removed {
            restore(&f.path, &mut plan, "file content");
        }
        return Ok(plan);
    }

    // Snapshot states: file contents only.
    let set = store.get_set(&log.id, state_id).ok_or_else(|| {
        let states = list_states(store, log);
        let ids: Vec<&str> = states.iter().map(|s| s.id.as_str()).collect();
        anyhow::anyhow!(
            "no recorded state {state_id} for this log (states: {})",
            ids.join(", ")
        )
    })?;
    if set.kind == CaptureKind::Before {
        return build_rewind_plan(store, log, "before", reg);
    }
    plan.state_label = format!("File contents {} ({})", set.kind.label(), set.created);
    plan.notes
        .push("This state recorded file contents only; the registry is not changed.".to_string());
    for c in &set.files {
        if store.verify_blob(&c.blob) {
            plan.file_restores.push(FileRestore {
                path: c.path.clone(),
                blob: c.blob.clone(),
                size: c.size,
                exists_now: Path::new(&c.path).exists(),
            });
        } else {
            plan.gaps.push(format!(
                "cannot restore {} — its recorded content is missing or damaged \
                 in the content store",
                c.path
            ));
        }
    }
    Ok(plan)
}

fn lowercase_first(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        Some(c) => c.to_lowercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

// ---------------------------------------------------------------------------
// Execution
// ---------------------------------------------------------------------------

/// What an executed rewind produced.
pub struct RewindOutcome {
    pub results: Vec<ActionResult>,
    /// The backup that makes this rewind itself undoable
    /// (`installscope restore <id>`), when anything was backed up.
    pub backup_id: Option<String>,
    /// The pre-rewind state recorded just before running, so the moment
    /// before the rewind is itself a recorded state.
    pub pre_rewind_set: Option<CaptureSet>,
    pub evicted: Vec<EvictionRecord>,
}

impl RewindOutcome {
    pub fn failed(&self) -> usize {
        self.results.iter().filter(|r| !r.ok).count()
    }
}

/// Carry out a rewind plan.
///
/// Order matters: first the current state of the log's files is recorded
/// as a pre-rewind state and a standard backup of everything about to be
/// touched is opened; then the revert machinery runs (it backs up its own
/// targets into the same backup), then recorded file contents are put
/// back — each write staged and verified against its recorded hash.
pub fn execute_rewind(
    store: &mut ContentStore,
    plan: &RewindPlan,
    log: &InstallLog,
    reg: &dyn Registry,
    backups_dir: &Path,
    budget: u64,
    opts: &ExecOptions,
) -> Result<RewindOutcome> {
    let mut outcome = RewindOutcome {
        results: Vec::new(),
        backup_id: None,
        pre_rewind_set: None,
        evicted: Vec::new(),
    };

    // 1. The moment before the rewind becomes a recorded state itself.
    let pre = store.capture(log, CaptureKind::PreRewind, CaptureScope::WholeLog, budget)?;
    outcome.pre_rewind_set = pre.set;
    outcome.evicted = pre.evicted;

    // 2. A backup of the current state of everything the content restores
    //    will overwrite or recreate, so the rewind is undoable as a whole.
    let mut writer = BackupWriter::new(
        backups_dir,
        &format!("Rewind {} to {}", log.name, plan.state_id),
    )?;
    for r in &plan.file_restores {
        let p = Path::new(&r.path);
        if p.exists() {
            writer.add_file(p)?;
        } else {
            writer.add_file_absent(&r.path)?;
        }
    }

    // 3. Registry before-images, added-file deletions, services and tasks —
    //    the existing, reviewed machinery, backing up as it goes.
    if !plan.revert_plan.items.is_empty() {
        let results = revert::execute_plan(&plan.revert_plan, reg, Some(&mut writer), opts);
        outcome.results.extend(results);
    }

    // 4. Recorded file contents.
    for r in &plan.file_restores {
        let desc = r.describe();
        match put_back(store, r) {
            Ok(msg) => outcome.results.push(ActionResult::ok(desc, msg)),
            Err(e) => outcome
                .results
                .push(ActionResult::err(desc, format!("{e:#}"))),
        }
    }

    if writer.is_empty() {
        writer.discard();
    } else {
        outcome.backup_id = Some(writer.finish()?.id);
    }
    Ok(outcome)
}

/// Write one recorded blob back to its path: staged write, then verified
/// against the recorded hash before it counts as restored.
fn put_back(store: &ContentStore, r: &FileRestore) -> Result<String> {
    let bytes = store.blob_bytes(&r.blob)?;
    let p = Path::new(&r.path);
    if let Some(parent) = p.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("create folder {}", parent.display()))?;
        }
    }
    let tmp = p.with_extension("installscope-rewind-tmp");
    std::fs::write(&tmp, &bytes).with_context(|| format!("write {}", r.path))?;
    if hexbytes::to_hex(&crate::util::sha256(&bytes)) != r.blob {
        let _ = std::fs::remove_file(&tmp);
        anyhow::bail!("recorded content for {} failed verification", r.path);
    }
    // Replacing an existing file: rename over it (on Windows this needs the
    // old file gone first; remove it — the backup already holds it).
    if p.exists() {
        std::fs::remove_file(p).with_context(|| format!("replace {}", r.path))?;
    }
    std::fs::rename(&tmp, p).with_context(|| format!("restore {}", r.path))?;
    Ok(format!(
        "restored recorded content ({})",
        crate::model::human_size(r.size)
    ))
}

// ---------------------------------------------------------------------------
// Tests: fixture programs in temp dirs, byte-identical restore proofs.
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backup::restore_backup;
    use crate::model::{
        ChangeSet, FileChange, FileEntry, KeyData, RegValue, ValueChange, ValueRef,
    };
    use crate::platform::{MemRegistry, Registry};

    const BIG: u64 = 1 << 30;
    const KEY: &str = "HKCU\\Software\\FixtureApp";
    const LEGACY: &str = "HKCU\\Software\\FixtureApp\\Legacy";

    fn fe(path: &str, size: u64) -> FileEntry {
        FileEntry {
            path: path.to_string(),
            is_dir: false,
            size,
            mtime: 0,
        }
    }

    fn hash_of(bytes: &[u8]) -> String {
        hexbytes::to_hex(&crate::util::sha256(bytes))
    }

    /// A fixture program and the log of its "bad update":
    /// app.exe existed ("ORIGINAL-EXE") and was overwritten, data.db
    /// existed ("ORIGINAL-DB") and was deleted, new.dll was added, the
    /// Version value changed 1.0 → 2.0, a Telemetry value was added, and
    /// the Legacy key was deleted.
    struct Fixture {
        td: crate::util::TestDir,
        app: PathBuf,
        db: PathBuf,
        dll: PathBuf,
        log: InstallLog,
    }

    fn fixture() -> Fixture {
        let td = crate::util::tempdir().unwrap();
        let dir = td.path().join("prog");
        std::fs::create_dir_all(&dir).unwrap();
        let app = dir.join("app.exe");
        let db = dir.join("data.db");
        let dll = dir.join("new.dll");
        let (a, d, n) = (
            app.to_string_lossy().to_string(),
            db.to_string_lossy().to_string(),
            dll.to_string_lossy().to_string(),
        );
        let mut changes = ChangeSet::default();
        changes.files_modified = vec![FileChange {
            before: fe(&a, 12),
            after: fe(&a, 14),
        }];
        changes.files_removed = vec![fe(&d, 11)];
        changes.files_added = vec![fe(&n, 3)];
        changes.values_modified = vec![ValueChange {
            key: KEY.into(),
            before: RegValue::string("Version", "1.0"),
            after: RegValue::string("Version", "2.0"),
        }];
        changes.values_added = vec![ValueRef {
            key: KEY.into(),
            value: RegValue::string("Telemetry", "on"),
        }];
        changes.keys_removed = vec![KeyData::new(LEGACY).with(RegValue::string("Mode", "classic"))];
        let log = InstallLog {
            id: "fixturelog".into(),
            name: "Fixture App".into(),
            created: "2026-01-01 10:00:00".into(),
            changes,
            ..Default::default()
        };
        Fixture {
            td,
            app,
            db,
            dll,
            log,
        }
    }

    impl Fixture {
        /// The program as it was before the bad update.
        fn write_before_state(&self) {
            std::fs::write(&self.app, b"ORIGINAL-EXE").unwrap();
            std::fs::write(&self.db, b"ORIGINAL-DB").unwrap();
        }
        /// Apply the bad update: overwrite, delete, add, registry changes.
        fn apply_bad_update(&self, reg: &MemRegistry) {
            std::fs::write(&self.app, b"BAD-UPDATE-EXE").unwrap();
            std::fs::remove_file(&self.db).unwrap();
            std::fs::write(&self.dll, b"NEW").unwrap();
            reg.put(
                KeyData::new(KEY)
                    .with(RegValue::string("Version", "2.0"))
                    .with(RegValue::string("Telemetry", "on")),
            );
        }
        fn store(&self) -> ContentStore {
            ContentStore::open(self.td.path()).unwrap()
        }
        fn backups_dir(&self) -> PathBuf {
            let d = self.td.path().join("backups");
            std::fs::create_dir_all(&d).unwrap();
            d
        }
    }

    #[test]
    fn rewind_restores_bytes_registry_and_undoes_itself() {
        let fx = fixture();
        fx.write_before_state();
        let reg = MemRegistry::new();

        // Capture the before-contents (what the monitoring hook will do).
        let mut store = fx.store();
        let cap = store.capture_before_images(&fx.log, BIG).unwrap();
        let set = cap.set.unwrap();
        assert_eq!(set.files.len(), 2, "app.exe and data.db contents captured");
        assert!(cap.skipped.is_empty(), "skipped: {:?}", cap.skipped);
        let exe_hash = hash_of(b"ORIGINAL-EXE");
        assert!(set.files.iter().any(|f| f.blob == exe_hash));

        fx.apply_bad_update(&reg);

        let states = list_states(&store, &fx.log);
        assert_eq!(states[0].id, "before");
        assert_eq!(states[0].files_with_content, 2);
        assert_eq!(states[0].files_without_content, 0);

        let plan = build_rewind_plan(&store, &fx.log, "before", &reg).unwrap();
        assert!(plan.complete(), "gaps: {:?}", plan.gaps);
        assert_eq!(plan.file_restores.len(), 2);
        // The revert machinery carries the rest: delete the added file,
        // restore the changed value, delete the added value, recreate the
        // removed key.
        let descs: Vec<String> = plan
            .revert_plan
            .items
            .iter()
            .map(|i| i.action.describe())
            .collect();
        assert!(descs.iter().any(|d| d.contains("new.dll")), "{descs:?}");
        assert!(descs.iter().any(|d| d.contains("Version")), "{descs:?}");
        assert!(descs.iter().any(|d| d.contains("Telemetry")), "{descs:?}");
        assert!(descs.iter().any(|d| d.contains("Legacy")), "{descs:?}");

        let outcome = execute_rewind(
            &mut store,
            &plan,
            &fx.log,
            &reg,
            &fx.backups_dir(),
            BIG,
            &ExecOptions::default(),
        )
        .unwrap();
        assert_eq!(outcome.failed(), 0, "results: {:?}", outcome.results.len());
        assert!(outcome.backup_id.is_some());
        assert!(outcome.pre_rewind_set.is_some());

        // Byte-identical restoration, proven by hash, not just by eye.
        let exe = std::fs::read(&fx.app).unwrap();
        assert_eq!(exe, b"ORIGINAL-EXE");
        assert_eq!(hash_of(&exe), exe_hash);
        assert_eq!(std::fs::read(&fx.db).unwrap(), b"ORIGINAL-DB");
        assert!(!fx.dll.exists(), "the file the update added is removed");

        // Registry before-state restored.
        let k = reg.read_key(KEY).unwrap();
        assert_eq!(k.get_string("Version").as_deref(), Some("1.0"));
        assert!(k.get("Telemetry").is_none());
        let legacy = reg.read_key(LEGACY).expect("removed key recreated");
        assert_eq!(legacy.get_string("Mode").as_deref(), Some("classic"));

        // The rewind is itself undoable through the standard backup.
        let results = restore_backup(
            &fx.backups_dir(),
            outcome.backup_id.as_deref().unwrap(),
            &reg,
        )
        .unwrap();
        assert!(results.iter().all(|r| r.ok), "{results:?}");
        assert_eq!(std::fs::read(&fx.app).unwrap(), b"BAD-UPDATE-EXE");
        assert!(!fx.db.exists(), "undo removes the recreated file again");
        assert!(fx.dll.exists(), "undo brings back the update's file");
        let k = reg.read_key(KEY).unwrap();
        assert_eq!(k.get_string("Version").as_deref(), Some("2.0"));
        assert_eq!(k.get_string("Telemetry").as_deref(), Some("on"));
    }

    #[test]
    fn missing_capture_is_an_explicit_gap_never_a_quiet_partial() {
        let fx = fixture();
        fx.write_before_state();
        let reg = MemRegistry::new();
        fx.apply_bad_update(&reg);
        // No capture_before_images: the contents were never recorded.
        let store = fx.store();
        let plan = build_rewind_plan(&store, &fx.log, "before", &reg).unwrap();
        assert!(!plan.complete());
        assert_eq!(plan.gaps.len(), 2, "gaps: {:?}", plan.gaps);
        for g in &plan.gaps {
            assert!(g.contains("no recorded before-state"), "{g}");
        }
        let lines = plan.describe_lines().join("\n");
        assert!(lines.contains("PARTIAL"), "{lines}");
        assert!(!lines.contains("Verdict: COMPLETE"), "{lines}");
        // The registry side is unaffected by the missing file contents.
        assert!(!plan.revert_plan.items.is_empty());
    }

    #[test]
    fn damaged_blob_is_a_gap_not_a_restore() {
        let fx = fixture();
        fx.write_before_state();
        let reg = MemRegistry::new();
        let mut store = fx.store();
        store.capture_before_images(&fx.log, BIG).unwrap();
        // Corrupt every blob in the store.
        let blobs = store.dir().join("blobs");
        for fan in std::fs::read_dir(&blobs).unwrap().flatten() {
            for f in std::fs::read_dir(fan.path()).unwrap().flatten() {
                std::fs::write(f.path(), b"garbage").unwrap();
            }
        }
        fx.apply_bad_update(&reg);
        let plan = build_rewind_plan(&store, &fx.log, "before", &reg).unwrap();
        assert!(!plan.complete());
        assert!(
            plan.gaps.iter().all(|g| g.contains("missing or damaged")),
            "{:?}",
            plan.gaps
        );
    }

    #[test]
    fn eviction_is_oldest_first_and_never_touches_protected_sets() {
        let td = crate::util::tempdir().unwrap();
        let mk_log = |id: &str, file: &Path| {
            let p = file.to_string_lossy().to_string();
            let mut changes = ChangeSet::default();
            changes.files_modified = vec![FileChange {
                before: fe(&p, 40),
                after: fe(&p, 40),
            }];
            InstallLog {
                id: id.into(),
                name: id.into(),
                changes,
                ..Default::default()
            }
        };
        let fa = td.path().join("a.bin");
        let fb = td.path().join("b.bin");
        let log_a = mk_log("loga", &fa);
        let log_b = mk_log("logb", &fb);
        let budget = 120; // exactly three 40-byte blobs
        let mut store = ContentStore::open(td.path()).unwrap();

        std::fs::write(&fa, vec![b'A'; 40]).unwrap();
        let before = store.capture_before_images(&log_a, budget).unwrap();
        assert!(before.evicted.is_empty());

        std::fs::write(&fa, vec![b'B'; 40]).unwrap();
        let s1 = store
            .capture(
                &log_a,
                CaptureKind::Snapshot,
                CaptureScope::WholeLog,
                budget,
            )
            .unwrap();
        let s1_id = s1.set.unwrap().id;

        std::fs::write(&fa, vec![b'C'; 40]).unwrap();
        let s2 = store
            .capture(
                &log_a,
                CaptureKind::Snapshot,
                CaptureScope::WholeLog,
                budget,
            )
            .unwrap();
        let s2_id = s2.set.unwrap().id;
        assert_eq!(store.referenced_bytes(), budget);

        // One more blob (another program) overflows the budget: the oldest
        // unprotected set — log A's first snapshot — must go, nothing else.
        std::fs::write(&fb, vec![b'D'; 40]).unwrap();
        let sb = store
            .capture(
                &log_b,
                CaptureKind::Snapshot,
                CaptureScope::WholeLog,
                budget,
            )
            .unwrap();
        assert_eq!(sb.evicted.len(), 1, "evicted: {:?}", sb.evicted);
        assert_eq!(sb.evicted[0].set_id, s1_id);
        assert_eq!(store.referenced_bytes(), budget);
        assert!(store.before_set("loga").is_some(), "before-state protected");
        assert!(
            store.get_set("loga", &s2_id).is_some(),
            "newest set protected"
        );
        assert!(store.get_set("loga", &s1_id).is_none());
        assert_eq!(store.index().evictions.len(), 1, "eviction is on record");
        // And the eviction was reported, with its size.
        assert_eq!(sb.evicted[0].bytes, 40);
    }

    #[test]
    fn snapshot_state_rewinds_file_contents_only() {
        let fx = fixture();
        fx.write_before_state();
        let reg = MemRegistry::new();
        let mut store = fx.store();
        let cap = store
            .capture(&fx.log, CaptureKind::Snapshot, CaptureScope::WholeLog, BIG)
            .unwrap();
        let set_id = cap.set.unwrap().id;

        // Later: app.exe changes by hand (no registry change involved).
        std::fs::write(&fx.app, b"SOMETHING-ELSE").unwrap();

        let plan = build_rewind_plan(&store, &fx.log, &set_id, &reg).unwrap();
        assert!(plan.complete(), "gaps: {:?}", plan.gaps);
        assert!(plan.revert_plan.items.is_empty(), "registry untouched");
        assert!(
            plan.notes
                .iter()
                .any(|n| n.contains("registry is not changed")),
            "{:?}",
            plan.notes
        );
        let outcome = execute_rewind(
            &mut store,
            &plan,
            &fx.log,
            &reg,
            &fx.backups_dir(),
            BIG,
            &ExecOptions::default(),
        )
        .unwrap();
        assert_eq!(outcome.failed(), 0);
        assert_eq!(std::fs::read(&fx.app).unwrap(), b"ORIGINAL-EXE");
        // data.db was in the snapshot scope (files_removed of the log) and
        // never left, so it is simply verified in place.
        assert_eq!(std::fs::read(&fx.db).unwrap(), b"ORIGINAL-DB");
    }

    #[test]
    fn unknown_state_names_the_valid_ones() {
        let fx = fixture();
        let reg = MemRegistry::new();
        let store = fx.store();
        let err = build_rewind_plan(&store, &fx.log, "snap-nope", &reg).unwrap_err();
        assert!(format!("{err}").contains("before"), "{err}");
    }
}
