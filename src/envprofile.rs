//! Explicit developer-path overlays. No global process environment mutation.
use crate::{
    model::{reg_type, RegValue},
    store::durable,
};
use anyhow::{ensure, Context, Result};
use std::{
    collections::HashSet,
    fs::{self, File, OpenOptions},
    io::Read,
    path::{Path, PathBuf},
    process::{Child, Command},
};
#[cfg(windows)]
mod windows;
#[cfg(windows)]
pub use windows::UserEnvironment;

pub const VARIABLES: &[&str] = &[
    "CARGO_HOME",
    "RUSTUP_HOME",
    "JAVA_HOME",
    "DOTNET_ROOT",
    "DOTNET_ROOT_X64",
    "DOTNET_ROOT_ARM64",
    "GOPATH",
    "GOROOT",
    "GRADLE_USER_HOME",
    "MAVEN_HOME",
    "VCPKG_ROOT",
    "CMAKE_PREFIX_PATH",
    "PYTHONPATH",
    "VIRTUAL_ENV",
];
const PROFILE_LIMIT: u64 = 64 * 1024;
const RECEIPT_LIMIT: u64 = 4 * 1024 * 1024;
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Variable {
    pub name: String,
    pub value: Option<String>,
}
crate::json_struct!(Variable,{name,value});
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Profile {
    pub version: u32,
    pub name: String,
    pub project: String,
    pub path_prepend: Vec<String>,
    pub variables: Vec<Variable>,
}
crate::json_struct!(Profile,{version,name,project,path_prepend,variables});
fn name(raw: &str) -> Result<&'static str> {
    if raw.eq_ignore_ascii_case("Path") {
        return Ok("Path");
    }
    VARIABLES.iter().copied().find(|n|n.eq_ignore_ascii_case(raw)).context("Only the listed developer path variables are supported; secrets and arbitrary variables are excluded")
}
fn clean(value: &str, max: usize) -> Result<()> {
    ensure!(
        value.len() <= max && !value.chars().any(|c| c.is_control()),
        "Text is too long or contains control characters"
    );
    Ok(())
}
fn path_value(value: &str) -> Result<()> {
    clean(value, 4096)?;
    let bytes = value.as_bytes();
    ensure!(
        !value.is_empty()
            && !value.contains(['%', ';', '"', '*', '?'])
            && !value.starts_with("\\\\.\\"),
        "Use an explicit absolute path, without variables, quotes, wildcards or semicolons"
    );
    ensure!(
        (bytes.len() >= 3
            && bytes[0].is_ascii_alphabetic()
            && bytes[1] == b':'
            && matches!(bytes[2], b'\\' | b'/'))
            || value.starts_with("\\\\"),
        "Use an absolute Windows drive or UNC path"
    );
    ensure!(
        !value.contains("://"),
        "URLs and credentials are not supported profile values"
    );
    Ok(())
}
fn variable_value(key: &str, value: &str) -> Result<()> {
    ensure!(
        !value.is_empty(),
        "Use Remove rather than an empty variable value"
    );
    if matches!(key, "PYTHONPATH" | "CMAKE_PREFIX_PATH" | "GOPATH") {
        for part in value.split(';') {
            path_value(part)?;
        }
    } else {
        path_value(value)?;
    }
    ensure!(
        value.encode_utf16().count() <= 16_000,
        "Variable value exceeds the profile limit"
    );
    Ok(())
}
fn path_key(raw: &str) -> String {
    let path = raw.replace('/', "\\");
    if path.len() == 3 && path.as_bytes()[1] == b':' && path.ends_with('\\') {
        path.to_lowercase()
    } else {
        path.trim_end_matches('\\').to_lowercase()
    }
}
impl Profile {
    pub fn validate(&self) -> Result<()> {
        ensure!(self.version == 1, "Unsupported environment profile version");
        clean(&self.name, 640)?;
        ensure!(
            !self.name.trim().is_empty() && self.name.chars().count() <= 160,
            "Enter a name of at most 160 characters"
        );
        path_value(&self.project)?;
        ensure!(
            self.path_prepend.len() <= 64 && self.variables.len() <= 16,
            "Profile exceeds 64 PATH entries or 16 variables"
        );
        ensure!(
            !self.path_prepend.is_empty() || !self.variables.is_empty(),
            "Add a PATH directory or developer variable"
        );
        let mut paths = HashSet::new();
        for path in &self.path_prepend {
            path_value(path)?;
            ensure!(
                paths.insert(
                    path.replace('/', "\\")
                        .trim_end_matches('\\')
                        .to_lowercase()
                ),
                "Duplicate PATH directory"
            );
        }
        let mut names = HashSet::new();
        for variable in &self.variables {
            let canonical = name(&variable.name)?;
            ensure!(
                canonical != "Path" && names.insert(canonical),
                "Duplicate variable or PATH override; use PATH prepend entries"
            );
            if let Some(value) = &variable.value {
                variable_value(canonical, value)?;
            }
        }
        ensure!(
            crate::json::to_json_string(self).len() as u64 <= PROFILE_LIMIT,
            "Profile exceeds 64 KiB"
        );
        Ok(())
    }
}
fn read_bounded(path: &Path, limit: u64) -> Result<Vec<u8>> {
    durable::check_path(path)?;
    let file = File::open(path)?;
    let meta = file.metadata()?;
    ensure!(
        meta.is_file() && meta.len() <= limit,
        "Input is not a regular file or exceeds its size limit"
    );
    let mut bytes = vec![];
    file.take(limit + 1).read_to_end(&mut bytes)?;
    ensure!(bytes.len() as u64 <= limit, "Input grew beyond its limit");
    Ok(bytes)
}
pub fn import(path: &Path) -> Result<Profile> {
    use crate::json::FromJson;
    let value = crate::json::parse_bytes(&read_bounded(path, PROFILE_LIMIT)?)?;
    let object = value.as_object().context("Profile must be a JSON object")?;
    ensure!(
        object.len() == 5
            && object.iter().all(|(key, _)| [
                "version",
                "name",
                "project",
                "path_prepend",
                "variables"
            ]
            .contains(&key.as_str())),
        "Missing or unknown profile field; only explicit developer paths are supported"
    );
    if let Some(variables) =
        crate::json::field(object, "variables").and_then(crate::json::Value::as_array)
    {
        for entry in variables {
            let entry = entry.as_object().context("Variable must be an object")?;
            ensure!(
                entry.len() == 2
                    && entry
                        .iter()
                        .all(|(key, _)| ["name", "value"].contains(&key.as_str())),
                "Missing or unknown variable field; removal requires an explicit null value"
            );
        }
    }
    let profile = Profile::from_json(&value)?;
    profile.validate()?;
    Ok(profile)
}
pub fn export(path: &Path, profile: &Profile) -> Result<()> {
    profile.validate()?;
    ensure!(path.is_absolute(), "Choose an absolute export path");
    durable::write(
        path,
        crate::json::to_json_string_pretty(profile).as_bytes(),
        false,
    )?;
    Ok(())
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Change {
    pub name: String,
    pub before: Option<RegValue>,
    pub after: Option<RegValue>,
}
crate::json_struct!(Change,{name,before,after});
fn raw_text(value: &RegValue) -> Result<String> {
    ensure!(
        matches!(value.kind, reg_type::SZ | reg_type::EXPAND_SZ)
            && value.data.len() <= 64 * 1024
            && value.data.len().is_multiple_of(2),
        "Existing variable has unsupported type or oversized/invalid UTF-16 data"
    );
    let words: Vec<u16> = value
        .data
        .as_chunks::<2>()
        .0
        .iter()
        .map(|b| u16::from_le_bytes([b[0], b[1]]))
        .collect();
    ensure!(
        words.last() == Some(&0) && !words[..words.len().saturating_sub(1)].contains(&0),
        "Existing variable is not one terminated UTF-16 string"
    );
    String::from_utf16(&words[..words.len() - 1])
        .context("Existing variable contains invalid Unicode")
}
pub fn display_value(value: &Option<RegValue>) -> String {
    match value {
        None => "<absent>".into(),
        Some(v) => raw_text(v).unwrap_or_else(|_| "<unsupported value>".into()),
    }
}
/// Backends must distinguish missing values from access/read failures.
pub trait Environment {
    fn read(&self, name: &str) -> Result<Option<RegValue>>;
    fn write(&self, name: &str, value: Option<&RegValue>) -> Result<()>;
    fn flush(&self) -> Result<()> {
        Ok(())
    }
    fn acquire(&self) -> Result<Box<dyn std::any::Any>> {
        Ok(Box::new(()))
    }
    fn notify(&self) -> String {
        "Restart terminals and IDEs to inherit the updated user environment; existing processes may retain old values.".into()
    }
}
pub fn preview(profile: &Profile, environment: &dyn Environment) -> Result<Vec<Change>> {
    profile.validate()?;
    let mut changes = vec![];
    let mut add = |key: &str, new: Option<String>, prepend: bool| -> Result<()> {
        let before = environment.read(key)?;
        let old = before.as_ref().map(raw_text).transpose()?;
        if let Some(raw) = &before {
            ensure!(
                raw.name.eq_ignore_ascii_case(key),
                "Variable name differs from requested value"
            );
        }
        let after = if let Some(mut value) = new {
            if prepend {
                if let Some(old) = old.filter(|s| !s.is_empty()) {
                    let prefix: Vec<_> = value.split(';').map(path_key).collect();
                    if old
                        .split(';')
                        .take(prefix.len())
                        .map(path_key)
                        .collect::<Vec<_>>()
                        == prefix
                    {
                        value = old;
                    } else {
                        value.push(';');
                        value.push_str(&old);
                    }
                }
            }
            ensure!(
                value.encode_utf16().count() <= 32766,
                "Resulting variable exceeds the 32,766 UTF-16 unit profile limit"
            );
            let mut raw = RegValue::string(
                before.as_ref().map(|b| b.name.as_str()).unwrap_or(key),
                &value,
            );
            if let Some(old) = &before {
                raw.kind = old.kind;
            }
            Some(raw)
        } else {
            None
        };
        if before != after {
            changes.push(Change {
                name: key.into(),
                before,
                after,
            });
        }
        Ok(())
    };
    if !profile.path_prepend.is_empty() {
        add("Path", Some(profile.path_prepend.join(";")), true)?;
    }
    for variable in &profile.variables {
        add(name(&variable.name)?, variable.value.clone(), false)?;
    }
    Ok(changes)
}
struct ProcessEnvironment;
impl Environment for ProcessEnvironment {
    fn read(&self, key: &str) -> Result<Option<RegValue>> {
        std::env::var_os(key)
            .map(|v| {
                v.into_string()
                    .map(|s| RegValue::string(key, &s))
                    .map_err(|_| anyhow::anyhow!("Process variable {key} is not valid Unicode"))
            })
            .transpose()
    }
    fn write(&self, _: &str, _: Option<&RegValue>) -> Result<()> {
        anyhow::bail!("Global process environment mutation is disabled")
    }
}
#[derive(Clone, Debug)]
pub struct ProcessPlan {
    pub profile: Profile,
    pub changes: Vec<Change>,
}
pub fn preview_process(profile: &Profile) -> Result<ProcessPlan> {
    Ok(ProcessPlan {
        profile: profile.clone(),
        changes: preview(profile, &ProcessEnvironment)?,
    })
}
pub fn launch(
    plan: &ProcessPlan,
    executable: &Path,
    arguments: &str,
    new_console: bool,
) -> Result<Child> {
    plan.profile.validate()?;
    clean(arguments, 8192)?;
    ensure!(
        preview(&plan.profile, &ProcessEnvironment)? == plan.changes,
        "Inherited environment changed; preview again"
    );
    ensure!(
        executable.is_absolute()
            && executable
                .extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("exe")),
        "Choose an absolute .exe path; scripts require an explicitly reviewed interpreter"
    );
    durable::check_path(executable)?;
    ensure!(
        fs::metadata(executable)?.is_file(),
        "Executable is not a regular file"
    );
    let project = Path::new(&plan.profile.project);
    durable::check_path(project)?;
    ensure!(
        fs::metadata(project)?.is_dir(),
        "Project working directory is unavailable"
    );
    let mut command = Command::new(executable);
    command.current_dir(project);
    for change in &plan.changes {
        if let Some(value) = &change.after {
            command.env(&change.name, raw_text(value)?);
        } else {
            command.env_remove(&change.name);
        }
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.raw_arg(arguments).creation_flags(if new_console {
            windows_sys::Win32::System::Threading::CREATE_NEW_CONSOLE
        } else {
            windows_sys::Win32::System::Threading::CREATE_NO_WINDOW
        });
    }
    #[cfg(not(windows))]
    {
        let _ = (arguments, new_console);
        anyhow::bail!("Environment profile launching requires Windows");
    }
    #[cfg(windows)]
    {
        command
            .spawn()
            .context("Could not launch the selected executable; no elevation fallback is performed")
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    Prepared,
    Applying,
    Applied,
    Undoing,
    Undone,
}
crate::json_enum!(State {
    Prepared,
    Applying,
    Applied,
    Undoing,
    Undone
});
#[derive(Clone, Debug)]
pub struct Transaction {
    pub version: u32,
    pub id: String,
    pub identity: String,
    pub created: String,
    pub profile_name: String,
    pub changes: Vec<Change>,
    pub state: State,
    pub notice: String,
}
crate::json_struct!(Transaction,{version,id,identity,created,profile_name,changes,state,notice});
impl Transaction {
    fn validate(&self) -> Result<()> {
        ensure!(
            self.version == 1,
            "Unsupported environment transaction version"
        );
        crate::util::validate_local_id(&self.id)?;
        ensure!(
            self.identity.len() == 64 && self.identity.bytes().all(|b| b.is_ascii_hexdigit()),
            "Invalid transaction identity"
        );
        clean(&self.created, 128)?;
        clean(&self.profile_name, 640)?;
        clean(&self.notice, 2048)?;
        ensure!(
            !self.changes.is_empty() && self.changes.len() <= 17,
            "Invalid transaction change count"
        );
        let mut names = HashSet::new();
        for change in &self.changes {
            let canonical = name(&change.name)?;
            ensure!(
                names.insert(canonical) && change.before != change.after,
                "Duplicate or empty transaction change"
            );
            for raw in [&change.before, &change.after].into_iter().flatten() {
                ensure!(
                    raw.name.eq_ignore_ascii_case(canonical),
                    "Invalid raw variable name"
                );
                raw_text(raw)?;
            }
        }
        ensure!(
            crate::json::to_json_string(self).len() as u64 <= RECEIPT_LIMIT,
            "Transaction exceeds 4 MiB receipt limit"
        );
        Ok(())
    }
}
pub fn prepare_user(profile: &Profile, environment: &dyn Environment) -> Result<Transaction> {
    let changes = preview(profile, environment)?;
    ensure!(
        !changes.is_empty(),
        "This profile makes no changes to the saved user environment"
    );
    let transaction = Transaction {
        version: 1,
        id: format!("{}-{}", crate::util::new_id(), std::process::id()),
        identity: crate::regression::identity_marker()?,
        created: crate::util::now_string(),
        profile_name: profile.name.clone(),
        changes,
        state: State::Prepared,
        notice: String::new(),
    };
    transaction.validate()?;
    ensure!(
        crate::json::to_json_string(&transaction).len() + 4096 <= RECEIPT_LIMIT as usize,
        "Prior-state receipt is too large to reserve completion/recovery metadata"
    );
    Ok(transaction)
}
fn root(data: &Path) -> Result<PathBuf> {
    ensure!(data.is_absolute(), "Data directory must be absolute");
    let root = data.join("environment-profiles");
    durable::check_path(&root)?;
    Ok(root)
}
fn operation_lock(root: &Path) -> Result<File> {
    fs::create_dir_all(root)?;
    let path = root.join(".operation.lock");
    durable::check_path(&path)?;
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(path)?;
    file.try_lock()
        .context("Another environment transaction is active; wait and reload")?;
    Ok(file)
}
fn receipt(data: &Path, id: &str) -> Result<PathBuf> {
    crate::util::validate_local_id(id)?;
    let path = root(data)?
        .join("transactions")
        .join(id)
        .join("receipt.json");
    durable::check_path(&path)?;
    Ok(path)
}
pub fn load_transaction(data: &Path, id: &str) -> Result<Transaction> {
    use crate::json::FromJson;
    let value = crate::json::parse_bytes(&read_bounded(&receipt(data, id)?, RECEIPT_LIMIT)?)?;
    let object = value.as_object().context("Transaction must be an object")?;
    ensure!(
        object.len() == 8
            && object.iter().all(|(key, _)| [
                "version",
                "id",
                "identity",
                "created",
                "profile_name",
                "changes",
                "state",
                "notice"
            ]
            .contains(&key.as_str())),
        "Missing or unknown transaction field; preserve this receipt for manual recovery"
    );
    if let Some(changes) =
        crate::json::field(object, "changes").and_then(crate::json::Value::as_array)
    {
        for change in changes {
            let change = change.as_object().context("Change must be an object")?;
            ensure!(
                change.len() == 3
                    && change
                        .iter()
                        .all(|(key, _)| ["name", "before", "after"].contains(&key.as_str())),
                "Missing or unknown change field; prior absence must be explicit"
            );
        }
    }
    let transaction = Transaction::from_json(&value)?;
    transaction.validate()?;
    ensure!(transaction.id == id, "Transaction ID does not match folder");
    Ok(transaction)
}
fn save(data: &Path, transaction: &Transaction, replace: bool) -> Result<()> {
    transaction.validate()?;
    let bytes = crate::json::to_json_string_pretty(transaction);
    ensure!(
        bytes.len() as u64 <= RECEIPT_LIMIT,
        "Transaction receipt exceeds 4 MiB"
    );
    durable::write(&receipt(data, &transaction.id)?, bytes.as_bytes(), replace)?;
    Ok(())
}
fn same_identity(transaction: &Transaction) -> Result<()> {
    ensure!(
        transaction.identity == crate::regression::identity_marker()?,
        "Transaction belongs to another machine/user; cannot apply or undo it here"
    );
    Ok(())
}
pub fn apply_user(
    data: &Path,
    prepared: &Transaction,
    environment: &dyn Environment,
) -> Result<Transaction> {
    prepared.validate()?;
    ensure!(
        prepared.notice.is_empty()
            && crate::json::to_json_string(prepared).len() + 4096 <= RECEIPT_LIMIT as usize,
        "Prepared receipt lacks space for completion/recovery metadata; preview again"
    );
    same_identity(prepared)?;
    ensure!(
        prepared.state == State::Prepared,
        "Preview a fresh transaction before applying"
    );
    let _system_lock = environment.acquire()?;
    let root = root(data)?;
    let _lock = operation_lock(&root)?;
    for change in &prepared.changes {
        ensure!(
            environment.read(&change.name)? == change.before,
            "{} changed since preview; nothing applied, preview again",
            change.name
        );
    }
    let path = receipt(data, &prepared.id)?;
    let parent = path.parent().unwrap();
    fs::create_dir_all(parent.parent().unwrap())?;
    fs::create_dir(parent)
        .context("Transaction ID already exists; preview again without overwriting its receipt")?;
    let mut transaction = prepared.clone();
    save(data, &transaction, false)?;
    transaction.state = State::Applying;
    save(data, &transaction, true)?;
    for change in &transaction.changes {
        ensure!(
            environment.read(&change.name)? == change.before,
            "{} changed during apply; reload receipt {} and review undo",
            change.name,
            transaction.id
        );
        environment
            .write(&change.name, change.after.as_ref())
            .with_context(|| {
                format!(
                    "Apply interrupted; receipt {} retains exact prior values for reviewed undo",
                    transaction.id
                )
            })?;
        ensure!(
            environment.read(&change.name)? == change.after,
            "Write verification failed; reload receipt {} for recovery",
            transaction.id
        );
    }
    environment.flush().with_context(|| {
        format!(
            "User values changed but flush failed; reload receipt {} before recovery",
            transaction.id
        )
    })?;
    transaction.state = State::Applied;
    transaction.notice = environment.notify();
    save(data, &transaction, true)?;
    Ok(transaction)
}
pub fn undo_user(
    data: &Path,
    reviewed: &Transaction,
    environment: &dyn Environment,
) -> Result<Transaction> {
    reviewed.validate()?;
    let id = &reviewed.id;
    let _system_lock = environment.acquire()?;
    let root = root(data)?;
    let _lock = operation_lock(&root)?;
    let mut transaction = load_transaction(data, id)?;
    same_identity(&transaction)?;
    ensure!(
        transaction.changes == reviewed.changes
            && transaction.identity == reviewed.identity
            && transaction.created == reviewed.created,
        "Receipt changed since review; reload before undo"
    );
    ensure!(
        transaction.state != State::Undone,
        "Transaction is already undone"
    );
    // Check the whole transaction before starting restoration. Never clobber a
    // third-party value. External writers do not participate in our file lock.
    for change in &transaction.changes {
        let current = environment.read(&change.name)?;
        ensure!(
            current == change.before || current == change.after,
            "{} conflicts with both saved states; no undo started",
            change.name
        );
    }
    transaction.state = State::Undoing;
    save(data, &transaction, true)?;
    for change in transaction.changes.iter().rev() {
        let current = environment.read(&change.name)?;
        if current == change.before {
            continue;
        }
        ensure!(
            current == change.after,
            "{} changed during undo; stop and inspect receipt {}",
            change.name,
            id
        );
        environment
            .write(&change.name, change.before.as_ref())
            .with_context(|| {
                format!("Undo interrupted; preserve receipt {id} and reload before retrying")
            })?;
        ensure!(
            environment.read(&change.name)? == change.before,
            "Undo verification failed; preserve receipt {id}"
        );
    }
    environment.flush().with_context(|| {
        format!("Prior values restored but flush failed; reload receipt {id} before recovery")
    })?;
    transaction.state = State::Undone;
    transaction.notice = environment.notify();
    save(data, &transaction, true)?;
    Ok(transaction)
}
pub fn list_transactions(data: &Path) -> Result<Vec<String>> {
    let root = root(data)?.join("transactions");
    durable::check_path(&root)?;
    if !root.exists() {
        return Ok(vec![]);
    }
    let mut ids = vec![];
    for entry in fs::read_dir(root)?.take(257) {
        ensure!(
            ids.len() < 256,
            "More than 256 transactions; inspect the data directory manually"
        );
        let entry = entry?;
        durable::check_path(&entry.path())?;
        ensure!(entry.file_type()?.is_dir(), "Unexpected transaction entry");
        let id = entry
            .file_name()
            .into_string()
            .map_err(|_| anyhow::anyhow!("Non-Unicode transaction ID"))?;
        crate::util::validate_local_id(&id)?;
        ids.push(id);
    }
    ids.sort();
    Ok(ids)
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;
    use std::{
        cell::{Cell, RefCell},
        collections::BTreeMap,
    };
    #[derive(Default)]
    struct Memory {
        values: RefCell<BTreeMap<String, RegValue>>,
        writes: Cell<usize>,
        fail_at: Cell<Option<usize>>,
    }
    impl Environment for Memory {
        fn read(&self, key: &str) -> Result<Option<RegValue>> {
            Ok(self.values.borrow().get(&key.to_ascii_uppercase()).cloned())
        }
        fn write(&self, key: &str, value: Option<&RegValue>) -> Result<()> {
            let n = self.writes.get() + 1;
            self.writes.set(n);
            ensure!(self.fail_at.get() != Some(n), "injected write failure");
            if let Some(value) = value {
                self.values
                    .borrow_mut()
                    .insert(key.to_ascii_uppercase(), value.clone());
            } else {
                self.values.borrow_mut().remove(&key.to_ascii_uppercase());
            }
            Ok(())
        }
    }
    fn profile() -> Profile {
        Profile {
            version: 1,
            name: "SDK fixture".into(),
            project: "C:\\fixture".into(),
            path_prepend: vec!["C:\\SDK\\bin".into()],
            variables: vec![Variable {
                name: "JAVA_HOME".into(),
                value: Some("C:\\Java".into()),
            }],
        }
    }
    #[test]
    fn exported_profile_contains_only_explicit_overrides_and_never_replaces_files() {
        let temp = crate::util::tempdir().unwrap();
        let path = temp.path().join("profile.json");
        let profile = profile();
        export(&path, &profile).unwrap();
        assert_eq!(import(&path).unwrap(), profile);
        let bytes = fs::read(&path).unwrap();
        assert!(export(&path, &profile).is_err());
        assert_eq!(fs::read(&path).unwrap(), bytes);
        OpenOptions::new()
            .write(true)
            .open(&path)
            .unwrap()
            .set_len(PROFILE_LIMIT + 1)
            .unwrap();
        assert!(import(&path).is_err());
    }
    #[test]
    fn secret_names_urls_duplicate_variables_and_unknown_schema_are_refused() {
        let mut profile = profile();
        profile.variables[0].name = "API_TOKEN".into();
        assert!(profile.validate().is_err());
        profile = super::tests::profile();
        profile.variables[0].value = Some("https://name:password@example.test".into());
        assert!(profile.validate().is_err());
        profile = super::tests::profile();
        profile.variables.push(profile.variables[0].clone());
        assert!(profile.validate().is_err());
        profile = super::tests::profile();
        profile.version = 2;
        assert!(profile.validate().is_err());
    }
    #[test]
    fn user_path_preserves_expansion_type_case_and_is_idempotent() {
        let environment = Memory::default();
        let mut original = RegValue::string("pAtH", "%USERPROFILE%\\bin");
        original.kind = reg_type::EXPAND_SZ;
        environment.write("Path", Some(&original)).unwrap();
        let profile = profile();
        let changes = preview(&profile, &environment).unwrap();
        let path = changes.iter().find(|c| c.name == "Path").unwrap();
        assert_eq!(path.before, Some(original));
        assert_eq!(path.after.as_ref().unwrap().kind, reg_type::EXPAND_SZ);
        assert_eq!(path.after.as_ref().unwrap().name, "pAtH");
        assert_eq!(
            display_value(&path.after),
            "C:\\SDK\\bin;%USERPROFILE%\\bin"
        );
        for change in changes {
            environment
                .write(&change.name, change.after.as_ref())
                .unwrap();
        }
        assert!(preview(&profile, &environment).unwrap().is_empty());
        let mut root_profile = profile.clone();
        root_profile.path_prepend = vec!["C:\\".into()];
        environment
            .write("Path", Some(&RegValue::string("Path", "C:;C:\\Other")))
            .unwrap();
        let root_changes = preview(&root_profile, &environment).unwrap();
        assert_eq!(
            display_value(
                &root_changes
                    .iter()
                    .find(|c| c.name == "Path")
                    .unwrap()
                    .after
            ),
            "C:\\;C:;C:\\Other"
        );
    }
    #[test]
    fn exact_prior_bytes_and_absence_restore_after_apply() {
        let temp = crate::util::tempdir().unwrap();
        let environment = Memory::default();
        let mut prior = RegValue::string("pAtH", "C:\\Original");
        prior.kind = reg_type::EXPAND_SZ;
        environment.write("Path", Some(&prior)).unwrap();
        let prepared = prepare_user(&profile(), &environment).unwrap();
        let applied = apply_user(temp.path(), &prepared, &environment).unwrap();
        assert_eq!(applied.state, State::Applied);
        let loaded = load_transaction(temp.path(), &applied.id).unwrap();
        assert_eq!(loaded.changes[0].before, Some(prior.clone()));
        let undone = undo_user(temp.path(), &applied, &environment).unwrap();
        assert_eq!(undone.state, State::Undone);
        assert_eq!(environment.read("Path").unwrap(), Some(prior));
        assert!(environment.read("JAVA_HOME").unwrap().is_none());
        assert!(undo_user(temp.path(), &applied, &environment).is_err());
    }
    #[test]
    fn stale_preview_and_undo_conflicts_do_not_overwrite_third_party_values() {
        let temp = crate::util::tempdir().unwrap();
        let environment = Memory::default();
        let prepared = prepare_user(&profile(), &environment).unwrap();
        let external = RegValue::string("JAVA_HOME", "C:\\Other");
        environment.write("JAVA_HOME", Some(&external)).unwrap();
        assert!(apply_user(temp.path(), &prepared, &environment).is_err());
        assert!(list_transactions(temp.path()).unwrap().is_empty());
        let fresh = prepare_user(&profile(), &environment).unwrap();
        let applied = apply_user(temp.path(), &fresh, &environment).unwrap();
        environment.write("JAVA_HOME", Some(&external)).unwrap();
        // This equals the saved BEFORE state, so undo may safely leave it there.
        let third = RegValue::string("JAVA_HOME", "C:\\ThirdParty");
        environment.write("JAVA_HOME", Some(&third)).unwrap();
        let path = environment.read("Path").unwrap();
        assert!(undo_user(temp.path(), &applied, &environment).is_err());
        assert_eq!(environment.read("JAVA_HOME").unwrap(), Some(third));
        assert_eq!(environment.read("Path").unwrap(), path);
    }
    #[test]
    fn partial_apply_is_durable_and_reviewed_undo_can_resume() {
        let temp = crate::util::tempdir().unwrap();
        let environment = Memory::default();
        environment.fail_at.set(Some(2));
        let prepared = prepare_user(&profile(), &environment).unwrap();
        assert!(apply_user(temp.path(), &prepared, &environment).is_err());
        let pending = load_transaction(temp.path(), &prepared.id).unwrap();
        assert_eq!(pending.state, State::Applying);
        assert!(environment.read("Path").unwrap().is_some());
        assert!(environment.read("JAVA_HOME").unwrap().is_none());
        environment.fail_at.set(None);
        undo_user(temp.path(), &pending, &environment).unwrap();
        assert!(environment.read("Path").unwrap().is_none());
    }
    #[test]
    fn removal_undo_restores_value_and_malformed_prior_data_blocks_preview() {
        let temp = crate::util::tempdir().unwrap();
        let environment = Memory::default();
        let original = RegValue::string("JAVA_HOME", "C:\\Original");
        environment.write("JAVA_HOME", Some(&original)).unwrap();
        let mut profile = profile();
        profile.path_prepend.clear();
        profile.variables[0].value = None;
        let prepared = prepare_user(&profile, &environment).unwrap();
        let applied = apply_user(temp.path(), &prepared, &environment).unwrap();
        assert!(environment.read("JAVA_HOME").unwrap().is_none());
        undo_user(temp.path(), &applied, &environment).unwrap();
        assert_eq!(environment.read("JAVA_HOME").unwrap(), Some(original));
        let bad = RegValue {
            name: "JAVA_HOME".into(),
            kind: reg_type::SZ,
            data: vec![0, 1, 2],
        };
        environment.write("JAVA_HOME", Some(&bad)).unwrap();
        assert!(prepare_user(&profile, &environment).is_err());
    }
    #[test]
    fn unsafe_receipt_ids_and_foreign_identity_are_refused() {
        let temp = crate::util::tempdir().unwrap();
        let environment = Memory::default();
        let mut prepared = prepare_user(&profile(), &environment).unwrap();
        prepared.identity = "0".repeat(64);
        assert!(apply_user(temp.path(), &prepared, &environment).is_err());
        assert!(load_transaction(temp.path(), "../outside").is_err());
    }
    #[test]
    fn interrupted_undo_resumes_and_a_changed_reviewed_receipt_is_rejected() {
        let temp = crate::util::tempdir().unwrap();
        let environment = Memory::default();
        let prepared = prepare_user(&profile(), &environment).unwrap();
        let applied = apply_user(temp.path(), &prepared, &environment).unwrap();
        environment.fail_at.set(Some(environment.writes.get() + 2));
        assert!(undo_user(temp.path(), &applied, &environment).is_err());
        let pending = load_transaction(temp.path(), &applied.id).unwrap();
        assert_eq!(pending.state, State::Undoing);
        environment.fail_at.set(None);
        let mut altered = pending.clone();
        altered.changes[0].before = Some(RegValue::string("Path", "C:\\EditedReceipt"));
        save(temp.path(), &altered, true).unwrap();
        assert!(undo_user(temp.path(), &pending, &environment).is_err());
        save(temp.path(), &pending, true).unwrap();
        undo_user(temp.path(), &pending, &environment).unwrap();
        assert!(environment.read("Path").unwrap().is_none());
        assert!(environment.read("JAVA_HOME").unwrap().is_none());
    }
    #[test]
    fn per_user_mutex_serializes_native_adapters_without_writing_the_registry() {
        let guard = UserEnvironment.acquire().unwrap();
        assert!(std::thread::spawn(|| UserEnvironment.acquire().is_err())
            .join()
            .unwrap());
        drop(guard);
        let _guard = UserEnvironment.acquire().unwrap();
    }
    #[test]
    fn locked_receipt_retains_original_bytes_without_environment_writes() {
        use std::os::windows::fs::OpenOptionsExt;
        let temp = crate::util::tempdir().unwrap();
        let environment = Memory::default();
        let prepared = prepare_user(&profile(), &environment).unwrap();
        let applied = apply_user(temp.path(), &prepared, &environment).unwrap();
        let path = receipt(temp.path(), &applied.id).unwrap();
        let original = fs::read(&path).unwrap();
        let guard = OpenOptions::new()
            .read(true)
            .share_mode(1)
            .open(&path)
            .unwrap();
        let writes = environment.writes.get();
        assert!(undo_user(temp.path(), &applied, &environment).is_err());
        assert_eq!(environment.writes.get(), writes);
        assert_eq!(fs::read(&path).unwrap(), original);
        drop(guard);
        undo_user(temp.path(), &applied, &environment).unwrap();
    }
    #[test]
    fn present_empty_value_is_restored_as_present_not_absent() {
        let temp = crate::util::tempdir().unwrap();
        let environment = Memory::default();
        let empty = RegValue::string("JAVA_HOME", "");
        environment.write("JAVA_HOME", Some(&empty)).unwrap();
        let prepared = prepare_user(&profile(), &environment).unwrap();
        let applied = apply_user(temp.path(), &prepared, &environment).unwrap();
        undo_user(temp.path(), &applied, &environment).unwrap();
        assert_eq!(environment.read("JAVA_HOME").unwrap(), Some(empty));
    }
    #[test]
    fn flush_failure_after_writes_leaves_recoverable_applying_state() {
        struct FlushFailure {
            inner: Memory,
            fail: Cell<bool>,
        }
        impl Environment for FlushFailure {
            fn read(&self, key: &str) -> Result<Option<RegValue>> {
                self.inner.read(key)
            }
            fn write(&self, key: &str, value: Option<&RegValue>) -> Result<()> {
                self.inner.write(key, value)
            }
            fn flush(&self) -> Result<()> {
                ensure!(!self.fail.get(), "injected flush failure");
                Ok(())
            }
        }
        let temp = crate::util::tempdir().unwrap();
        let environment = FlushFailure {
            inner: Memory::default(),
            fail: Cell::new(true),
        };
        let prepared = prepare_user(&profile(), &environment).unwrap();
        assert!(apply_user(temp.path(), &prepared, &environment).is_err());
        assert!(environment.read("JAVA_HOME").unwrap().is_some());
        let pending = load_transaction(temp.path(), &prepared.id).unwrap();
        assert_eq!(pending.state, State::Applying);
        environment.fail.set(false);
        undo_user(temp.path(), &pending, &environment).unwrap();
        assert!(environment.read("JAVA_HOME").unwrap().is_none());
    }
    #[test]
    fn missing_optional_fields_cannot_silently_mean_delete_or_prior_absence() {
        let temp = crate::util::tempdir().unwrap();
        let path = temp.path().join("missing.json");
        fs::write(&path,r#"{"version":1,"name":"fixture","project":"C:\\fixture","path_prepend":[],"variables":[{"name":"JAVA_HOME"}]}"#).unwrap();
        assert!(import(&path).is_err());
        let environment = Memory::default();
        let prepared = prepare_user(&profile(), &environment).unwrap();
        let applied = apply_user(temp.path(), &prepared, &environment).unwrap();
        let receipt = receipt(temp.path(), &applied.id).unwrap();
        let original = fs::read_to_string(&receipt).unwrap();
        let missing = original.replacen("\"before\": null,", "", 1);
        assert_ne!(missing, original);
        fs::write(&receipt, missing).unwrap();
        assert!(load_transaction(temp.path(), &applied.id).is_err());
    }
}
