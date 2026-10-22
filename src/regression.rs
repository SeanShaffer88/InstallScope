//! Explicit local installer checkpoint evidence. Never runs vendor operations.
use crate::{
    model::{ChangeSet, Snapshot},
    platform::{self, Registry},
    settings::{FileRoot, Settings},
    snapshot::{self, Progress},
    store::durable,
};
use anyhow::{ensure, Context, Result};
use std::{
    fs::{self, File, OpenOptions},
    io::Read,
    path::{Component, Path, PathBuf},
};

const MANIFEST_LIMIT: u64 = 1024 * 1024;
const SNAPSHOT_LIMIT: u64 = 128 * 1024 * 1024;
const COMPRESSED_LIMIT: u64 = 64 * 1024 * 1024;
const ATTEMPT_LIMIT: usize = 32;
const ITEM_LIMIT: u64 = 50_000;

// An accidental-cross-machine guard, not an authenticity/security boundary.
// Hash the Windows MachineGuid and current token SID; retain neither raw ID.
#[cfg(windows)]
pub(crate) fn identity_marker() -> Result<String> {
    use windows_sys::Win32::{
        Security::{
            GetLengthSid, GetTokenInformation, IsValidSid, TokenUser, TOKEN_QUERY, TOKEN_USER,
        },
        System::{
            Registry::{RegGetValueW, HKEY_LOCAL_MACHINE, RRF_RT_REG_SZ, RRF_SUBKEY_WOW6464KEY},
            Threading::{GetCurrentProcess, OpenProcessToken},
        },
    };
    let key: Vec<u16> = "SOFTWARE\\Microsoft\\Cryptography\0"
        .encode_utf16()
        .collect();
    let value: Vec<u16> = "MachineGuid\0".encode_utf16().collect();
    let mut guid = [0u16; 256];
    let mut length = std::mem::size_of_val(&guid) as u32;
    let rc = unsafe {
        RegGetValueW(
            HKEY_LOCAL_MACHINE,
            key.as_ptr(),
            value.as_ptr(),
            RRF_RT_REG_SZ | RRF_SUBKEY_WOW6464KEY,
            std::ptr::null_mut(),
            guid.as_mut_ptr().cast(),
            &mut length,
        )
    };
    ensure!(
        rc == 0 && length >= 4 && length as usize <= std::mem::size_of_val(&guid),
        "Cannot establish Windows machine identity (error {rc})"
    );
    let mut token = std::ptr::null_mut();
    ensure!(
        unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) } != 0,
        "Cannot read current user token"
    );
    struct Token(windows_sys::Win32::Foundation::HANDLE);
    impl Drop for Token {
        fn drop(&mut self) {
            unsafe { windows_sys::Win32::Foundation::CloseHandle(self.0) };
        }
    }
    let token = Token(token);
    let mut buffer = [0usize; 512];
    let mut needed = 0;
    ensure!(
        unsafe {
            GetTokenInformation(
                token.0,
                TokenUser,
                buffer.as_mut_ptr().cast(),
                std::mem::size_of_val(&buffer) as u32,
                &mut needed,
            )
        } != 0,
        "Cannot establish current user SID"
    );
    ensure!(
        needed as usize >= std::mem::size_of::<TOKEN_USER>()
            && needed as usize <= std::mem::size_of_val(&buffer),
        "Invalid token identity length"
    );
    let user = unsafe { &*buffer.as_ptr().cast::<TOKEN_USER>() };
    let base = buffer.as_ptr() as usize;
    let sid = user.User.Sid as usize;
    ensure!(
        sid >= base
            && sid
                .checked_add(8)
                .is_some_and(|end| end <= base + needed as usize),
        "Invalid token SID location"
    );
    // SID byte 1 is its subauthority count. Validate the complete range before
    // passing the pointer to the SID APIs.
    let sid_size = 8 + 4 * unsafe { *user.User.Sid.cast::<u8>().add(1) } as usize;
    ensure!(
        sid_size <= 68 && sid + sid_size <= base + needed as usize,
        "Invalid token SID range"
    );
    ensure!(
        unsafe { IsValidSid(user.User.Sid) } != 0
            && unsafe { GetLengthSid(user.User.Sid) } as usize == sid_size,
        "Invalid current user SID"
    );
    let mut bytes = Vec::new();
    for word in &guid[..length as usize / 2] {
        bytes.extend_from_slice(&word.to_le_bytes());
    }
    bytes.extend_from_slice(unsafe {
        std::slice::from_raw_parts(user.User.Sid.cast::<u8>(), sid_size)
    });
    Ok(crate::util::sha256(&bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect())
}
#[cfg(not(windows))]
pub(crate) fn identity_marker() -> Result<String> {
    anyhow::bail!("Installer regression workspaces require Windows identity APIs")
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    Before,
    Installed,
    Upgraded,
    Uninstalled,
}
crate::json_enum!(Step {
    Before,
    Installed,
    Upgraded,
    Uninstalled
});
impl Step {
    pub fn label(self) -> &'static str {
        match self {
            Self::Before => "Before",
            Self::Installed => "After install",
            Self::Upgraded => "After upgrade",
            Self::Uninstalled => "After uninstall",
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Outcome {
    NotApplicable,
    Success,
    Failure,
    Cancelled,
    RebootRequired,
    #[default]
    Unknown,
}
crate::json_enum!(Outcome {
    NotApplicable,
    Success,
    Failure,
    Cancelled,
    RebootRequired,
    Unknown
});
impl Outcome {
    pub const VENDOR: [Self; 5] = [
        Self::Unknown,
        Self::Success,
        Self::Failure,
        Self::Cancelled,
        Self::RebootRequired,
    ];
    pub fn label(self) -> &'static str {
        match self {
            Self::NotApplicable => "Baseline",
            Self::Success => "Reported success",
            Self::Failure => "Reported failure",
            Self::Cancelled => "Reported cancellation",
            Self::RebootRequired => "Reported reboot required",
            Self::Unknown => "Outcome unknown",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Scope {
    pub registry_roots: Vec<String>,
    pub file_roots: Vec<FileRoot>,
    pub registry_exclusions: Vec<String>,
    pub file_exclusions: Vec<String>,
    pub excluded_data: String,
}
crate::json_struct!(Scope, { registry_roots, file_roots, registry_exclusions, file_exclusions, excluded_data });
fn absolute(path: &Path) -> Result<()> {
    ensure!(
        path.is_absolute()
            && !path
                .components()
                .any(|c| matches!(c, Component::CurDir | Component::ParentDir)),
        "Use an absolute path without dot components"
    );
    Ok(())
}
fn display_path(raw: &str) -> Result<String> {
    #[cfg(windows)]
    {
        let value = raw.replace('/', "\\");
        if let Some(rest) = value.strip_prefix("\\\\?\\UNC\\") {
            return Ok(format!("\\\\{rest}"));
        }
        if let Some(rest) = value.strip_prefix("\\\\?\\") {
            ensure!(
                rest.as_bytes().get(1) == Some(&b':')
                    && rest.as_bytes().first().is_some_and(u8::is_ascii_alphabetic),
                "Use a drive or UNC path; device namespaces are unsupported for capture scope"
            );
            return Ok(rest.to_owned());
        }
        ensure!(
            !value.starts_with("\\\\.\\"),
            "Device namespaces are unsupported for capture scope"
        );
        Ok(value)
    }
    #[cfg(not(windows))]
    {
        Ok(raw.into())
    }
}
fn freeze_root(raw: &str) -> Result<String> {
    let path = PathBuf::from(display_path(raw)?);
    absolute(&path)?;
    durable::check_path(&path)?;
    let mut ancestor = path.as_path();
    let mut missing = vec![];
    loop {
        match fs::symlink_metadata(ancestor) {
            Ok(_) => break,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                missing.push(
                    ancestor
                        .file_name()
                        .context("Capture path has no existing ancestor")?
                        .to_owned(),
                );
                ancestor = ancestor.parent().context("Capture path has no parent")?;
            }
            Err(error) => return Err(error.into()),
        }
    }
    let resolved = fs::canonicalize(ancestor)?;
    let mut frozen = PathBuf::from(display_path(
        resolved.to_str().context("Capture paths must be Unicode")?,
    )?);
    for component in missing.into_iter().rev() {
        frozen.push(component);
    }
    display_path(frozen.to_str().context("Capture paths must be Unicode")?)
}
fn text(value: &str, limit: usize) -> Result<()> {
    ensure!(
        value.len() <= limit && !value.chars().any(|c| c.is_control()),
        "Text is too long or contains control characters"
    );
    Ok(())
}
impl Scope {
    pub fn from_settings(settings: &Settings, data: &Path) -> Result<Self> {
        absolute(data)?;
        durable::check_path(data)?;
        let excluded_data = freeze_root(data.to_str().context("Data path must be Unicode")?)?;
        let mut scope = Self {
            registry_roots: settings
                .registry_roots
                .iter()
                .map(|r| platform::normalize_reg_path(&platform::expand_env(r)))
                .collect(),
            file_roots: settings
                .file_roots
                .iter()
                .map(|r| {
                    Ok(FileRoot {
                        path: freeze_root(&platform::expand_env(&r.path))?,
                        recursive: r.recursive,
                    })
                })
                .collect::<Result<Vec<_>>>()?,
            registry_exclusions: settings
                .effective_registry_exclusions()
                .iter()
                .map(|s| platform::expand_env(s))
                .collect(),
            file_exclusions: settings
                .effective_file_exclusions()
                .iter()
                .map(|s| display_path(&platform::expand_env(s)))
                .collect::<Result<Vec<_>>>()?,
            excluded_data,
        };
        scope.file_exclusions.push(scope.excluded_data.clone());
        scope.validate(data)?;
        Ok(scope)
    }
    fn validate(&self, data: &Path) -> Result<()> {
        ensure!(
            self.excluded_data == freeze_root(data.to_str().context("Data path must be Unicode")?)?
                && self.file_exclusions.contains(&self.excluded_data),
            "Evidence data-folder exclusion changed; create a new workspace"
        );
        ensure!(
            !self.registry_roots.is_empty() || !self.file_roots.is_empty(),
            "Choose at least one capture root in Settings"
        );
        for list in [
            &self.registry_roots,
            &self.registry_exclusions,
            &self.file_exclusions,
        ] {
            ensure!(list.len() <= 256, "Too many scope entries");
            for entry in list {
                text(entry, 32768)?;
                ensure!(
                    !entry.contains('%'),
                    "Unresolved environment variable in scope"
                );
            }
        }
        ensure!(self.file_roots.len() <= 64, "Too many file roots");
        for root in &self.file_roots {
            text(&root.path, 32768)?;
            ensure!(
                !root.path.contains('%'),
                "Unresolved environment variable in file root"
            );
            absolute(Path::new(&root.path))?;
            ensure!(
                display_path(&root.path)? == root.path,
                "Capture root spelling changed; prepare a new normalized scope"
            );
            durable::check_path(Path::new(&root.path))?;
        }
        for root in &self.registry_roots {
            ensure!(
                ["HKLM\\", "HKCU\\", "HKCR\\", "HKU\\", "HKCC\\"]
                    .iter()
                    .any(|p| root.starts_with(p)),
                "Unsupported registry root"
            );
        }
        ensure!(
            crate::json::to_json_string(self).len() <= 256 * 1024,
            "Capture scope is too large"
        );
        Ok(())
    }
    fn settings(&self) -> Settings {
        Settings {
            registry_roots: self.registry_roots.clone(),
            file_roots: self.file_roots.clone(),
            registry_exclusions: self.registry_exclusions.clone(),
            file_exclusions: self.file_exclusions.clone(),
            auto_exclusions: false,
            ..Settings::default()
        }
    }
}

#[derive(Clone, Debug)]
pub struct Attempt {
    pub id: String,
    pub step: Step,
    pub started: String,
    pub finished: String,
    pub outcome: Outcome,
    pub reported_exit_code: Option<i32>,
    pub note: String,
    pub evidence: bool,
    pub warning_count: usize,
    pub error: String,
}
crate::json_struct!(Attempt, { id, step, started, finished, outcome, reported_exit_code, note, evidence, warning_count, error });
#[derive(Clone, Debug)]
pub struct Workspace {
    pub version: u32,
    pub id: String,
    pub name: String,
    pub computer: String,
    pub user: String,
    pub created: String,
    pub identity_marker: String,
    pub scope: Scope,
    pub revision: u64,
    pub attempts: Vec<Attempt>,
}
crate::json_struct!(Workspace, { version, id, name, computer, user, created, identity_marker, scope, revision, attempts });
impl Workspace {
    pub fn allowed_steps(&self) -> Vec<Step> {
        match self
            .attempts
            .iter()
            .rev()
            .find(|a| a.evidence)
            .map(|a| a.step)
        {
            None => vec![Step::Before],
            Some(Step::Before) => vec![Step::Installed],
            Some(Step::Installed) => vec![Step::Upgraded, Step::Uninstalled],
            Some(Step::Upgraded) => vec![Step::Uninstalled],
            Some(Step::Uninstalled) => vec![],
        }
    }
    fn validate(&self, data: &Path, id: &str) -> Result<()> {
        ensure!(
            self.version == 1,
            "Unsupported regression workspace version"
        );
        crate::util::validate_local_id(id)?;
        ensure!(self.id == id, "Workspace ID does not match folder");
        text(&self.name, 160)?;
        ensure!(!self.name.trim().is_empty(), "Workspace name is empty");
        for value in [&self.created, &self.computer, &self.user] {
            text(value, 1024)?;
        }
        ensure!(
            self.identity_marker.len() == 64
                && self.identity_marker.bytes().all(|b| b.is_ascii_hexdigit()),
            "Invalid workspace identity marker"
        );
        self.scope.validate(data)?;
        ensure!(
            self.attempts.len() <= ATTEMPT_LIMIT,
            "Too many checkpoint attempts"
        );
        // Each attempt has one begin publication and at most one finish publication.
        let expected = self
            .attempts
            .iter()
            .map(|a| if a.finished.is_empty() { 1u64 } else { 2 })
            .sum::<u64>();
        ensure!(self.revision == expected, "Invalid workspace revision");
        let mut prior = Self {
            attempts: vec![],
            ..self.clone()
        };
        let mut ids = std::collections::HashSet::new();
        for attempt in &self.attempts {
            crate::util::validate_local_id(&attempt.id)?;
            ensure!(
                ids.insert(attempt.id.to_ascii_lowercase()),
                "Duplicate capture ID"
            );
            ensure!(
                prior.allowed_steps().contains(&attempt.step),
                "Invalid checkpoint order"
            );
            ensure!(
                (attempt.step == Step::Before) == (attempt.outcome == Outcome::NotApplicable),
                "Invalid checkpoint outcome"
            );
            ensure!(
                attempt.step != Step::Before || attempt.reported_exit_code.is_none(),
                "Baseline cannot have a vendor exit code"
            );
            for (value, limit) in [
                (&attempt.started, 1024),
                (&attempt.finished, 1024),
                (&attempt.note, 4096),
                (&attempt.error, 8192),
            ] {
                text(value, limit)?;
            }
            ensure!(
                !attempt.evidence || (!attempt.finished.is_empty() && attempt.error.is_empty()),
                "Incomplete evidence receipt"
            );
            ensure!(
                attempt.evidence || attempt.warning_count == 0,
                "Warning count without evidence"
            );
            prior.attempts.push(attempt.clone());
        }
        Ok(())
    }
}
fn workspace_dir(data: &Path, id: &str) -> Result<PathBuf> {
    absolute(data)?;
    crate::util::validate_local_id(id)?;
    let path = data.join("regressions").join(id);
    durable::check_path(&path)?;
    Ok(path)
}
fn lock(dir: &Path) -> Result<File> {
    let path = dir.join(".operation.lock");
    durable::check_path(&path)?;
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(path)?;
    file.try_lock()
        .context("Workspace is busy in another operation; retry after it finishes")?;
    Ok(file)
}
fn read_bounded(path: &Path, limit: u64) -> Result<Vec<u8>> {
    durable::check_path(path)?;
    let file = File::open(path)?;
    ensure!(
        file.metadata()?.is_file() && file.metadata()?.len() <= limit,
        "Evidence file is not regular or exceeds its input limit"
    );
    let mut bytes = Vec::new();
    file.take(limit + 1).read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() as u64 <= limit,
        "Evidence input grew beyond its limit"
    );
    Ok(bytes)
}
pub fn load(data: &Path, id: &str) -> Result<Workspace> {
    let dir = workspace_dir(data, id)?;
    let workspace: Workspace =
        crate::json::from_json_slice(&read_bounded(&dir.join("workspace.json"), MANIFEST_LIMIT)?)?;
    workspace.validate(data, id)?;
    Ok(workspace)
}
fn publish(data: &Path, workspace: &Workspace, replace: bool) -> Result<()> {
    workspace.validate(data, &workspace.id)?;
    let bytes = crate::json::to_json_string_pretty(workspace);
    ensure!(
        bytes.len() as u64 <= MANIFEST_LIMIT,
        "Workspace receipt exceeds its size limit"
    );
    durable::write(
        &workspace_dir(data, &workspace.id)?.join("workspace.json"),
        bytes.as_bytes(),
        replace,
    )?;
    Ok(())
}
pub fn create(data: &Path, name: &str, scope: Scope) -> Result<Workspace> {
    absolute(data)?;
    scope.validate(data)?;
    text(name, 160)?;
    ensure!(!name.trim().is_empty(), "Enter a workspace name");
    let marker = identity_marker()?;
    let root = data.join("regressions");
    durable::check_path(&root)?;
    fs::create_dir_all(&root)?;
    let id = reserve(&root)?;
    let workspace = Workspace {
        version: 1,
        id,
        name: name.trim().into(),
        computer: platform::computer_name(),
        user: platform::user_name(),
        created: crate::util::now_string(),
        identity_marker: marker,
        scope,
        revision: 0,
        attempts: vec![],
    };
    publish(data, &workspace, false)?;
    Ok(workspace)
}
fn reserve(root: &Path) -> Result<String> {
    for _ in 0..32 {
        let id = format!("{}-{}", crate::util::new_id(), std::process::id());
        match fs::create_dir(root.join(&id)) {
            Ok(()) => return Ok(id),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e.into()),
        }
    }
    anyhow::bail!("Could not reserve a unique evidence directory; retry later")
}
fn identity(workspace: &Workspace) -> Result<()> {
    ensure!(
        workspace.identity_marker == identity_marker()?
            && workspace.computer == platform::computer_name()
            && workspace.user == platform::user_name(),
        "Workspace belongs to another computer/user; create a new workspace here"
    );
    Ok(())
}
pub struct CaptureRequest {
    pub revision: u64,
    pub step: Step,
    pub outcome: Outcome,
    pub reported_exit_code: Option<i32>,
    pub note: String,
}
/// Records a pending receipt first. A crash leaves an explicit interrupted attempt.
/// Own evidence writes only; the vendor operation has already happened externally.
pub fn capture(
    data: &Path,
    id: &str,
    request: CaptureRequest,
    reg: &dyn Registry,
    progress: &Progress,
) -> Result<Workspace> {
    let dir = workspace_dir(data, id)?;
    let _lock = lock(&dir)?;
    let mut workspace = load(data, id)?;
    identity(&workspace)?;
    ensure!(
        workspace.revision == request.revision,
        "Workspace changed; reopen before capturing"
    );
    ensure!(
        workspace.attempts.len() < ATTEMPT_LIMIT,
        "32 attempt limit reached; create a new workspace"
    );
    ensure!(
        workspace.allowed_steps().contains(&request.step),
        "Checkpoint is out of order"
    );
    text(&request.note, 4096)?;
    ensure!(
        (request.step == Step::Before) == (request.outcome == Outcome::NotApplicable),
        "Choose a vendor outcome for this checkpoint"
    );
    ensure!(
        request.step != Step::Before || request.reported_exit_code.is_none(),
        "Baseline cannot have a vendor exit code"
    );
    let capture_id = reserve(&dir)?;
    workspace.attempts.push(Attempt {
        id: capture_id.clone(),
        step: request.step,
        started: crate::util::now_string(),
        finished: String::new(),
        outcome: request.outcome,
        reported_exit_code: request.reported_exit_code,
        note: request.note,
        evidence: false,
        warning_count: 0,
        error: String::new(),
    });
    workspace.revision += 1;
    publish(data, &workspace, true)?;
    let result = (|| -> Result<usize> {
        workspace.scope.validate(data)?;
        progress.set_item_limit(ITEM_LIMIT);
        progress.set_byte_limit(SNAPSHOT_LIMIT / 2);
        ensure!(!progress.cancelled(), "Capture cancelled before scan");
        let mut snap = snapshot::take_snapshot(reg, &workspace.scope.settings(), progress)?;
        // take_snapshot skips absent roots. Preserve that fact, including access
        // errors, instead of implying all declared locations were inspected.
        for root in &workspace.scope.file_roots {
            match fs::metadata(&root.path) {
                Ok(m) if m.is_dir() => {}
                Ok(_) => snap
                    .warnings
                    .push(format!("Capture root is not a directory: {}", root.path)),
                Err(e) => snap
                    .warnings
                    .push(format!("Capture root unavailable: {} ({e})", root.path)),
            }
        }
        ensure!(
            !progress.cancelled(),
            "Capture cancelled before evidence save"
        );
        let path = dir.join(&capture_id).join("state.snap");
        durable::check_path(&path)?;
        snapshot::save_snapshot(&path, &snap)?;
        check_snapshot(&path)?;
        // Verify the bounded read and checksum before publishing a successful receipt.
        let _ = snapshot::load_snapshot_bounded(&path, SNAPSHOT_LIMIT)?;
        ensure!(
            !progress.cancelled(),
            "Capture cancelled after evidence save; retained uncommitted snapshot"
        );
        Ok(snap.warnings.len())
    })();
    let attempt = workspace.attempts.last_mut().unwrap();
    attempt.finished = crate::util::now_string();
    match result {
        Ok(warnings) => {
            attempt.evidence = true;
            attempt.warning_count = warnings
        }
        Err(error) => {
            // OS error text can contain control characters or be unusually long.
            attempt.error = format!("{error:#}")
                .chars()
                .filter(|c| !c.is_control())
                .take(2000)
                .collect();
        }
    }
    workspace.revision += 1;
    publish(data, &workspace, true)?;
    Ok(workspace)
}
fn check_snapshot(path: &Path) -> Result<()> {
    durable::check_path(path)?;
    let meta = fs::metadata(path)?;
    ensure!(
        meta.is_file() && meta.len() <= COMPRESSED_LIMIT,
        "Snapshot is not regular or exceeds 64 MiB compressed limit"
    );
    Ok(())
}
fn evidence(data: &Path, workspace: &Workspace, index: usize) -> Result<Snapshot> {
    let attempt = workspace
        .attempts
        .get(index)
        .context("Select a saved checkpoint")?;
    ensure!(attempt.evidence, "Checkpoint has no committed evidence");
    let path = workspace_dir(data, &workspace.id)?
        .join(&attempt.id)
        .join("state.snap");
    check_snapshot(&path)?;
    let snap = snapshot::load_snapshot_bounded(&path, SNAPSHOT_LIMIT)?;
    ensure!(
        snap.computer == workspace.computer && snap.user == workspace.user,
        "Snapshot identity differs from workspace"
    );
    ensure!(
        snap.warnings.len() == attempt.warning_count,
        "Snapshot warnings differ from receipt; evidence changed"
    );
    Ok(snap)
}
pub struct Comparison {
    pub changes: ChangeSet,
    pub warnings: Vec<String>,
    pub complete_observation: bool,
    pub from: Step,
    pub to: Step,
}
pub fn compare(data: &Path, id: &str, revision: u64, from: usize, to: usize) -> Result<Comparison> {
    let dir = workspace_dir(data, id)?;
    let _lock = lock(&dir)?;
    let workspace = load(data, id)?;
    identity(&workspace)?;
    ensure!(
        workspace.revision == revision,
        "Workspace changed; reopen before comparing"
    );
    ensure!(
        from < to,
        "Choose earlier evidence followed by later evidence"
    );
    let a = evidence(data, &workspace, from)?;
    let b = evidence(data, &workspace, to)?;
    let mut warnings = a.warnings.clone();
    warnings.extend(b.warnings.iter().cloned());
    let uncertain = workspace.attempts[from..=to]
        .iter()
        .any(|a| !a.evidence || !matches!(a.outcome, Outcome::NotApplicable | Outcome::Success));
    Ok(Comparison {
        complete_observation: warnings.is_empty() && !uncertain,
        changes: snapshot::diff(&a, &b),
        warnings,
        from: workspace.attempts[from].step,
        to: workspace.attempts[to].step,
    })
}

/// Directory names only. Invalid/incomplete workspaces remain visible for recovery.
pub fn list(data: &Path) -> Result<Vec<String>> {
    absolute(data)?;
    let root = data.join("regressions");
    durable::check_path(&root)?;
    if !root.exists() {
        return Ok(vec![]);
    }
    let mut ids = vec![];
    for entry in fs::read_dir(root)?.take(257) {
        ensure!(
            ids.len() < 256,
            "Workspace list exceeds 256 entries; inspect data directory manually"
        );
        let entry = entry?;
        let id = entry
            .file_name()
            .into_string()
            .map_err(|_| anyhow::anyhow!("Non-Unicode workspace name"))?;
        crate::util::validate_local_id(&id)?;
        durable::check_path(&entry.path())?;
        ensure!(
            entry.file_type()?.is_dir(),
            "Unexpected file in regression workspace directory"
        );
        ids.push(id);
    }
    ids.sort();
    Ok(ids)
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;
    use crate::platform::MemRegistry;
    fn fixture() -> (crate::util::TestDir, PathBuf, Workspace) {
        let temp = crate::util::tempdir().unwrap();
        let data = temp.path().join("data");
        fs::create_dir(&data).unwrap();
        let settings = Settings {
            registry_roots: vec![],
            file_roots: vec![FileRoot {
                path: temp.path().to_str().unwrap().into(),
                recursive: true,
            }],
            file_exclusions: vec![],
            auto_exclusions: false,
            ..Settings::default()
        };
        let workspace = create(
            &data,
            "Fixture",
            Scope::from_settings(&settings, &data).unwrap(),
        )
        .unwrap();
        (temp, data, workspace)
    }
    fn request(workspace: &Workspace, step: Step, outcome: Outcome) -> CaptureRequest {
        CaptureRequest {
            revision: workspace.revision,
            step,
            outcome,
            reported_exit_code: None,
            note: "fixture only".into(),
        }
    }
    fn checkpoint(data: &Path, workspace: &Workspace, step: Step, outcome: Outcome) -> Workspace {
        capture(
            data,
            &workspace.id,
            request(workspace, step, outcome),
            &MemRegistry::new(),
            &Progress::default(),
        )
        .unwrap()
    }
    #[test]
    fn lifecycle_reopens_and_reports_leftovers_without_capturing_own_receipts() {
        let (temp, data, mut workspace) = fixture();
        workspace = checkpoint(&data, &workspace, Step::Before, Outcome::NotApplicable);
        fs::write(temp.path().join("program"), "v1").unwrap();
        workspace = checkpoint(&data, &workspace, Step::Installed, Outcome::Success);
        fs::write(temp.path().join("program"), "version two").unwrap();
        workspace = checkpoint(&data, &workspace, Step::Upgraded, Outcome::Success);
        fs::remove_file(temp.path().join("program")).unwrap();
        fs::write(temp.path().join("leftover"), "review me").unwrap();
        workspace = checkpoint(&data, &workspace, Step::Uninstalled, Outcome::Success);
        workspace = load(&data, &workspace.id).unwrap();
        assert_eq!(workspace.revision, 8);
        assert!(workspace.allowed_steps().is_empty());
        let delta = compare(&data, &workspace.id, 8, 0, 3).unwrap();
        assert!(delta.complete_observation);
        assert_eq!(delta.changes.files_added.len(), 1);
        assert!(delta.changes.files_added[0].path.ends_with("leftover"));
        assert!(!delta
            .changes
            .files_added
            .iter()
            .any(|f| f.path.contains("workspace.json")));
        let upgrade = compare(&data, &workspace.id, 8, 1, 2).unwrap();
        assert_eq!(upgrade.changes.files_modified.len(), 1);
        assert_eq!(list(&data).unwrap(), vec![workspace.id]);
    }
    #[test]
    fn cancellation_is_saved_and_retry_keeps_old_attempt() {
        let (_temp, data, workspace) = fixture();
        let progress = Progress::default();
        progress
            .cancel
            .store(true, std::sync::atomic::Ordering::Relaxed);
        let failed = capture(
            &data,
            &workspace.id,
            request(&workspace, Step::Before, Outcome::NotApplicable),
            &MemRegistry::new(),
            &progress,
        )
        .unwrap();
        assert!(!failed.attempts[0].evidence);
        assert!(failed.attempts[0].error.contains("cancelled"));
        let retried = checkpoint(&data, &failed, Step::Before, Outcome::NotApplicable);
        assert_eq!(retried.attempts.len(), 2);
        assert!(retried.attempts[1].evidence);
        assert!(compare(&data, &retried.id, retried.revision, 0, 1).is_err());
    }
    #[test]
    fn stale_revision_busy_and_invalid_order_do_not_overwrite_evidence() {
        let (_temp, data, workspace) = fixture();
        let dir = workspace_dir(&data, &workspace.id).unwrap();
        let guard = lock(&dir).unwrap();
        assert!(capture(
            &data,
            &workspace.id,
            request(&workspace, Step::Before, Outcome::NotApplicable),
            &MemRegistry::new(),
            &Progress::default()
        )
        .is_err());
        drop(guard);
        assert!(capture(
            &data,
            &workspace.id,
            request(&workspace, Step::Installed, Outcome::Success),
            &MemRegistry::new(),
            &Progress::default()
        )
        .is_err());
        let latest = checkpoint(&data, &workspace, Step::Before, Outcome::NotApplicable);
        assert!(capture(
            &data,
            &workspace.id,
            request(&workspace, Step::Before, Outcome::NotApplicable),
            &MemRegistry::new(),
            &Progress::default()
        )
        .is_err());
        assert_eq!(
            load(&data, &workspace.id).unwrap().revision,
            latest.revision
        );
    }
    #[test]
    fn interrupted_receipt_and_orphan_snapshot_survive_reopen() {
        let (_temp, data, mut workspace) = fixture();
        let dir = workspace_dir(&data, &workspace.id).unwrap();
        let orphan = reserve(&dir).unwrap();
        fs::write(dir.join(&orphan).join("state.snap"), "interrupted").unwrap();
        workspace.attempts.push(Attempt {
            id: orphan.clone(),
            step: Step::Before,
            started: crate::util::now_string(),
            finished: String::new(),
            outcome: Outcome::NotApplicable,
            reported_exit_code: None,
            note: String::new(),
            evidence: false,
            warning_count: 0,
            error: String::new(),
        });
        workspace.revision = 1;
        publish(&data, &workspace, true).unwrap();
        let loaded = load(&data, &workspace.id).unwrap();
        let retried = checkpoint(&data, &loaded, Step::Before, Outcome::NotApplicable);
        assert!(retried.attempts[0].finished.is_empty());
        assert_eq!(
            fs::read(dir.join(orphan).join("state.snap")).unwrap(),
            b"interrupted"
        );
        assert_eq!(retried.revision, 3);
    }
    #[test]
    fn vendor_failures_reboots_and_unknowns_cannot_imply_complete_observation() {
        for outcome in [
            Outcome::Failure,
            Outcome::Cancelled,
            Outcome::RebootRequired,
            Outcome::Unknown,
        ] {
            let (_temp, data, workspace) = fixture();
            let baseline = checkpoint(&data, &workspace, Step::Before, Outcome::NotApplicable);
            let installed = checkpoint(&data, &baseline, Step::Installed, outcome);
            assert!(
                !compare(&data, &installed.id, installed.revision, 0, 1)
                    .unwrap()
                    .complete_observation
            );
            assert_eq!(
                load(&data, &workspace.id).unwrap().attempts[1].outcome,
                outcome
            );
        }
    }
    #[test]
    fn corrupt_missing_oversized_and_foreign_evidence_fail_closed() {
        let (_temp, data, workspace) = fixture();
        let baseline = checkpoint(&data, &workspace, Step::Before, Outcome::NotApplicable);
        let mut latest = checkpoint(&data, &baseline, Step::Installed, Outcome::Success);
        let dir = workspace_dir(&data, &workspace.id).unwrap();
        let snapshot = dir.join(&latest.attempts[1].id).join("state.snap");
        let original = fs::read(&snapshot).unwrap();
        fs::write(&snapshot, b"invalid").unwrap();
        assert!(compare(&data, &latest.id, latest.revision, 0, 1).is_err());
        fs::remove_file(&snapshot).unwrap();
        assert!(compare(&data, &latest.id, latest.revision, 0, 1).is_err());
        fs::write(&snapshot, &original).unwrap();
        OpenOptions::new()
            .write(true)
            .open(&snapshot)
            .unwrap()
            .set_len(COMPRESSED_LIMIT + 1)
            .unwrap();
        assert!(compare(&data, &latest.id, latest.revision, 0, 1).is_err());
        fs::write(&snapshot, original).unwrap();
        latest.computer.push_str("-other");
        publish(&data, &latest, true).unwrap();
        assert!(compare(&data, &latest.id, latest.revision, 0, 1).is_err());
        latest.version = 99;
        assert!(publish(&data, &latest, true).is_err());
    }
    #[test]
    fn unsafe_ids_scope_changes_and_future_schema_are_refused() {
        let (_temp, data, mut workspace) = fixture();
        assert!(load(&data, "../outside").is_err());
        workspace.scope.file_exclusions.clear();
        assert!(publish(&data, &workspace, true).is_err());
        let path = workspace_dir(&data, &workspace.id)
            .unwrap()
            .join("workspace.json");
        let bytes = fs::read_to_string(&path)
            .unwrap()
            .replace("\"version\": 1", "\"version\": 999");
        fs::write(&path, bytes).unwrap();
        assert!(load(&data, &workspace.id).is_err());
    }
    #[test]
    fn absent_root_is_visible_in_evidence_warnings() {
        let (temp, data, workspace) = fixture();
        let mut scope = workspace.scope.clone();
        scope.file_roots.push(FileRoot {
            path: temp.path().join("not-created").to_str().unwrap().into(),
            recursive: true,
        });
        let alternate = create(&data, "missing root", scope).unwrap();
        let baseline = checkpoint(&data, &alternate, Step::Before, Outcome::NotApplicable);
        let installed = checkpoint(&data, &baseline, Step::Installed, Outcome::Success);
        let comparison = compare(&data, &installed.id, installed.revision, 0, 1).unwrap();
        assert!(!comparison.complete_observation);
        assert!(!comparison.warnings.is_empty());
    }
    #[test]
    fn registry_fixture_and_reported_exit_code_survive_checkpoint_reopen() {
        use crate::model::{KeyData, RegValue};
        let (_temp, data, workspace) = fixture();
        let mut scope = workspace.scope.clone();
        scope.registry_roots = vec!["HKCU\\Software\\SetupScopeFixture".into()];
        let mut workspace = create(&data, "registry fixture", scope).unwrap();
        let reg = MemRegistry::new();
        reg.put(KeyData::new("HKCU\\Software\\SetupScopeFixture"));
        workspace = capture(
            &data,
            &workspace.id,
            request(&workspace, Step::Before, Outcome::NotApplicable),
            &reg,
            &Progress::default(),
        )
        .unwrap();
        reg.put(
            KeyData::new("HKCU\\Software\\SetupScopeFixture\\App")
                .with(RegValue::string("Version", "1")),
        );
        let mut after = request(&workspace, Step::Installed, Outcome::RebootRequired);
        after.reported_exit_code = Some(3010);
        workspace = capture(&data, &workspace.id, after, &reg, &Progress::default()).unwrap();
        let comparison = compare(&data, &workspace.id, workspace.revision, 0, 1).unwrap();
        assert_eq!(comparison.changes.keys_added.len(), 1);
        assert!(!comparison.complete_observation);
        assert_eq!(
            load(&data, &workspace.id).unwrap().attempts[1].reported_exit_code,
            Some(3010)
        );
    }
    #[test]
    fn alternate_windows_path_spellings_still_exclude_evidence_directory() {
        let (temp, data, _workspace) = fixture();
        let plain = temp.path().to_str().unwrap();
        for root in [plain.replace('\\', "/"), format!("\\\\?\\{plain}")] {
            let settings = Settings {
                registry_roots: vec![],
                file_roots: vec![FileRoot {
                    path: root,
                    recursive: true,
                }],
                file_exclusions: vec![],
                auto_exclusions: false,
                ..Settings::default()
            };
            let workspace = create(
                &data,
                "path alias fixture",
                Scope::from_settings(&settings, &data).unwrap(),
            )
            .unwrap();
            let before = checkpoint(&data, &workspace, Step::Before, Outcome::NotApplicable);
            let file = temp.path().join("visible-fixture");
            fs::write(&file, "x").unwrap();
            let after = checkpoint(&data, &before, Step::Installed, Outcome::Success);
            let comparison = compare(&data, &after.id, after.revision, 0, 1).unwrap();
            assert!(comparison.complete_observation);
            assert_eq!(comparison.changes.files_added.len(), 1);
            assert!(comparison.changes.files_added[0]
                .path
                .ends_with("visible-fixture"));
            fs::remove_file(file).unwrap();
        }
    }
    #[test]
    fn item_and_payload_budgets_fail_before_publishing_successful_snapshot() {
        use crate::model::{KeyData, RegValue};
        let reg = MemRegistry::new();
        reg.put(KeyData::new("HKCU\\Fixture").with(RegValue::string("data", &"x".repeat(1024))));
        let settings = Settings {
            registry_roots: vec!["HKCU\\Fixture".into()],
            file_roots: vec![],
            registry_exclusions: vec![],
            auto_exclusions: false,
            ..Settings::default()
        };
        let bytes = Progress::default();
        bytes.set_byte_limit(64);
        assert!(snapshot::take_snapshot(&reg, &settings, &bytes)
            .unwrap_err()
            .to_string()
            .contains("byte budget"));
        let items = Progress::default();
        items.set_byte_limit(1024 * 1024);
        items.set_item_limit(1);
        assert!(snapshot::take_snapshot(&reg, &settings, &items)
            .unwrap_err()
            .to_string()
            .contains("item limit"));
    }
    #[cfg(windows)]
    #[test]
    fn failed_finish_publication_leaves_pending_receipt_and_recoverable_evidence() {
        use crate::model::{KeyData, RegValue};
        use std::os::windows::fs::OpenOptionsExt;
        struct LockOnRead {
            path: PathBuf,
            held: std::sync::Mutex<Option<File>>,
        }
        impl Registry for LockOnRead {
            fn exists(&self, _: &str) -> bool {
                true
            }
            fn read_key(&self, path: &str) -> Option<KeyData> {
                *self.held.lock().unwrap() = Some(
                    OpenOptions::new()
                        .read(true)
                        .share_mode(1)
                        .open(&self.path)
                        .unwrap(),
                );
                Some(KeyData::new(path))
            }
            fn subkeys(&self, _: &str) -> Vec<String> {
                vec![]
            }
            fn create_key(&self, _: &str) -> Result<()> {
                anyhow::bail!("read-only fixture")
            }
            fn set_value(&self, _: &str, _: &RegValue) -> Result<()> {
                anyhow::bail!("read-only fixture")
            }
            fn delete_value(&self, _: &str, _: &str) -> Result<()> {
                anyhow::bail!("read-only fixture")
            }
            fn delete_tree(&self, _: &str) -> Result<()> {
                anyhow::bail!("read-only fixture")
            }
        }
        let (_temp, data, workspace) = fixture();
        let mut scope = workspace.scope.clone();
        scope.registry_roots = vec!["HKCU\\Fixture".into()];
        let workspace = create(&data, "publication failure", scope).unwrap();
        let dir = workspace_dir(&data, &workspace.id).unwrap();
        let reg = LockOnRead {
            path: dir.join("workspace.json"),
            held: std::sync::Mutex::new(None),
        };
        assert!(capture(
            &data,
            &workspace.id,
            request(&workspace, Step::Before, Outcome::NotApplicable),
            &reg,
            &Progress::default()
        )
        .is_err());
        drop(reg);
        let pending = load(&data, &workspace.id).unwrap();
        assert_eq!(pending.revision, 1);
        assert!(!pending.attempts[0].evidence);
        let saved = dir.join(&pending.attempts[0].id).join("state.snap");
        assert!(saved.is_file());
        let retried = checkpoint(&data, &pending, Step::Before, Outcome::NotApplicable);
        assert_eq!(retried.revision, 3);
        assert!(saved.is_file());
        assert!(retried.attempts[1].evidence);
    }
    #[test]
    fn attempt_limit_and_identity_marker_changes_preserve_previous_receipt() {
        let (_temp, data, mut workspace) = fixture();
        for index in 0..ATTEMPT_LIMIT {
            workspace.attempts.push(Attempt {
                id: format!("attempt-{index}"),
                step: Step::Before,
                started: "fixture".into(),
                finished: "fixture".into(),
                outcome: Outcome::NotApplicable,
                reported_exit_code: None,
                note: String::new(),
                evidence: false,
                warning_count: 0,
                error: "fixture failure".into(),
            });
        }
        workspace.revision = (ATTEMPT_LIMIT * 2) as u64;
        publish(&data, &workspace, true).unwrap();
        assert!(capture(
            &data,
            &workspace.id,
            request(&workspace, Step::Before, Outcome::NotApplicable),
            &MemRegistry::new(),
            &Progress::default()
        )
        .is_err());
        workspace.identity_marker = "0".repeat(64);
        publish(&data, &workspace, true).unwrap();
        let error = capture(
            &data,
            &workspace.id,
            request(&workspace, Step::Before, Outcome::NotApplicable),
            &MemRegistry::new(),
            &Progress::default(),
        )
        .unwrap_err();
        assert!(error.to_string().contains("another computer/user"));
        assert_eq!(load(&data, &workspace.id).unwrap().revision, 64);
    }
}
