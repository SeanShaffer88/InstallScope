//! Explicit downloads, pinned offline bundles and individually reviewed installers.
use crate::{json, snapshot::Progress, store::durable, trust, util};
use anyhow::{bail, ensure, Context, Result};
use std::{
    collections::BTreeSet,
    fs::{self, File, OpenOptions},
    io::{Read, Seek, SeekFrom, Write},
    path::{Component, Path, PathBuf},
};
#[cfg(test)]
mod tests;
#[cfg(windows)]
mod windows;
pub mod native;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Package {
    pub id: String,
    pub name: String,
    pub version: String,
    pub arch: String,
    pub filename: String,
    pub url: String,
    pub bytes: u64,
    pub algorithm: String,
    pub digest: String,
    pub publisher: String,
    pub kind: String,
    pub source: String,
    pub license_url: String,
    pub notes: String,
}
crate::json_struct!(Package,{id,name,version,arch,filename,url,bytes,algorithm,digest,publisher,kind,source,license_url,notes});
#[derive(Clone, Debug)]
pub struct Catalog {
    pub schema: u32,
    pub verified: String,
    pub packages: Vec<Package>,
}
crate::json_struct!(Catalog,{schema,verified,packages});
pub fn catalog() -> Result<Catalog> {
    let c: Catalog = json::from_json_str(include_str!("catalog.json"))?;
    ensure!(
        c.schema == 1 && c.packages.len() <= 32,
        "Unsupported catalog"
    );
    let mut ids = BTreeSet::new();
    let mut files = BTreeSet::new();
    for p in &c.packages {
        ensure!(
            ids.insert(&p.id) && files.insert(&p.filename),
            "Duplicate catalog entry"
        );
        ensure!(
            safe_name(&p.filename) && safe_name(&p.id),
            "Invalid catalog filename/ID"
        );
        ensure!(
            matches!(p.arch.as_str(), "x64" | "arm64") && matches!(p.kind.as_str(), "exe" | "msi"),
            "Unsupported package type"
        );
        ensure!(
            p.bytes > 0 && p.bytes <= 512 * 1024 * 1024,
            "Unsupported installer size"
        );
        ensure!(
            p.digest.len()
                == if p.algorithm == "SHA256" {
                    64
                } else if p.algorithm == "SHA512" {
                    128
                } else {
                    0
                },
            "Unsupported digest"
        );
        ensure!(
            p.digest.bytes().all(|b| b.is_ascii_hexdigit()),
            "Invalid digest"
        );
        allowed_url(&p.url)?;
    }
    Ok(c)
}
fn package(id: &str) -> Result<Package> {
    catalog()?
        .packages
        .into_iter()
        .find(|p| p.id == id)
        .context("Unknown package; update SetupScope's trusted catalog first")
}
fn safe_name(s: &str) -> bool {
    !s.is_empty()
        && s.len() < 160
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-_.+".contains(&b))
        && !s.starts_with('.')
}
pub(super) fn allowed_url(url: &str) -> Result<(&str, &str)> {
    ensure!(
        url.len() <= 8192 && !url.chars().any(|c| c.is_control() || c == '\\' || c == '#'),
        "Invalid download URL"
    );
    let rest = url.strip_prefix("https://").context("HTTPS is required")?;
    let (host, path) = rest.split_once('/').context("Missing HTTPS path")?;
    ensure!(
        matches!(
            host,
            "builds.dotnet.microsoft.com"
                | "github.com"
                | "release-assets.githubusercontent.com"
                | "objects.githubusercontent.com"
        ),
        "Unapproved download host: {host}"
    );
    if host == "github.com" {
        ensure!(
            path.starts_with("Kitware/CMake/releases/download/"),
            "Only official CMake release downloads are allowed"
        );
    }
    if host == "builds.dotnet.microsoft.com" {
        ensure!(
            path.starts_with("dotnet/"),
            "Only official .NET downloads are allowed"
        );
    }
    Ok((host, path))
}
fn checkpoint(p: &Progress) -> Result<()> {
    ensure!(
        !p.cancelled(),
        "Cancelled; complete cached files were preserved"
    );
    Ok(())
}
pub fn local_path(path: &Path) -> Result<()> {
    ensure!(
        path.is_absolute() && !path.components().any(|c| matches!(c, Component::ParentDir)),
        "Choose an absolute local path without parent traversal"
    );
    let text = path.to_string_lossy();
    ensure!(
        text.len() < 240 && !text.chars().any(|c| c.is_control() || c == '"'),
        "Choose a shorter local path without control characters or quotes"
    );
    #[cfg(windows)]
    ensure!(
        !text.starts_with("\\\\") && !text.starts_with("//") && !text[2..].contains(':'),
        "Network, device and alternate-stream paths are unsupported"
    );
    #[cfg(windows)]
    {
        let drive: Vec<u16> = format!("{}:\\", &text[..1])
            .encode_utf16()
            .chain(Some(0))
            .collect();
        let kind =
            unsafe { windows_sys::Win32::Storage::FileSystem::GetDriveTypeW(drive.as_ptr()) };
        ensure!(
            matches!(kind, 2 | 3 | 5 | 6),
            "Use a ready local volume; mapped network drives are unsupported"
        );
    }
    durable::check_path(path)?;
    Ok(())
}
fn directory(path: &Path) -> Result<()> {
    local_path(path)?;
    fs::create_dir_all(path)?;
    local_path(path)
}
pub fn cache_dir(root: &Path) -> PathBuf {
    root.join("dependencies").join("downloads")
}
fn receipts_dir(root: &Path) -> PathBuf {
    root.join("dependencies").join("receipts")
}

/// Holds the file against writes/deletion and ancestor directories against rename.
pub(crate) struct Locked {
    file: File,
    _parents: Vec<File>,
}
pub(crate) fn lock(path: &Path) -> Result<Locked> {
    local_path(path)?;
    let mut parents = Vec::new();
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        for parent in path.ancestors().skip(1) {
            parents.push(
                OpenOptions::new()
                    .read(true)
                    .share_mode(3)
                    .custom_flags(0x02000000)
                    .open(parent)
                    .with_context(|| format!("Lock parent {}", parent.display()))?,
            );
        }
    }
    local_path(path)?;
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.share_mode(1).custom_flags(0x00200000);
    }
    let file = options
        .open(path)
        .with_context(|| format!("Open installer {}", path.display()))?;
    ensure!(
        file.metadata()?.is_file(),
        "Installer is not a regular file"
    );
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        ensure!(
            file.metadata()?.file_attributes() & 0x400 == 0,
            "Reparse files are unsupported"
        );
    }
    Ok(Locked {
        file,
        _parents: parents,
    })
}

#[cfg(windows)]
fn digest(reader: &mut impl Read, algorithm: &str, p: &Progress) -> Result<String> {
    windows::hash(reader, algorithm, p)
}
#[cfg(not(windows))]
fn digest(reader: &mut impl Read, algorithm: &str, p: &Progress) -> Result<String> {
    ensure!(
        algorithm == "SHA256",
        "SHA512 verification requires Windows"
    );
    let mut data = Vec::new();
    reader.take(512 * 1024 * 1024 + 1).read_to_end(&mut data)?;
    checkpoint(p)?;
    ensure!(data.len() <= 512 * 1024 * 1024, "File too large");
    Ok(util::sha256(&data)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect())
}
fn check_bytes(p: &Package, file: &mut File, progress: &Progress) -> Result<()> {
    checkpoint(progress)?;
    ensure!(
        file.metadata()?.len() == p.bytes,
        "{} has the wrong size; preserve it and choose a clean cache",
        p.filename
    );
    file.seek(SeekFrom::Start(0))?;
    ensure!(
        digest(file, &p.algorithm, progress)?.eq_ignore_ascii_case(&p.digest),
        "Integrity mismatch for {}; installation is blocked",
        p.filename
    );
    file.seek(SeekFrom::Start(0))?;
    Ok(())
}
fn signature(p: &Package, path: &Path) -> Result<String> {
    match trust::check_file(&path.to_string_lossy()) {
        trust::Trust::Signed{signer} => {
            #[cfg(windows)] let organization=crate::platform::windows::authenticode_organization(&path.to_string_lossy());
            #[cfg(not(windows))] let organization:Option<String>=None;
            ensure!(organization.as_deref()==Some(p.publisher.as_str()),"Signing certificate organization mismatch: expected {}, got {:?} (display name {})",p.publisher,organization,signer);
            Ok(format!("{} (certificate display name: {})",p.publisher,signer))
        },
        other=>bail!("Publisher verification blocked {}: {:?}. Expected {}. Offline trust uses cached certificates; revocation is not queried.",p.filename,other,p.publisher)
    }
}
fn verified(p: &Package, path: &Path, progress: &Progress) -> Result<Locked> {
    let mut held = lock(path)?;
    check_bytes(p, &mut held.file, progress)?;
    signature(p, path)?;
    Ok(held)
}
pub fn verify(root: &Path, ids: &[String], progress: &Progress) -> Result<String> {
    let packages = selection(ids)?;
    for p in &packages {
        progress.set_phase(format!("Verify {} {}", p.name, p.arch));
        let _held = verified(p, &cache_dir(root).join(&p.filename), progress)?;
    }
    Ok(format!("{} installer(s): pinned size/hash and cached Authenticode publisher verified. No revocation lookup or installation.",packages.len()))
}
fn selection(ids: &[String]) -> Result<Vec<Package>> {
    ensure!(
        !ids.is_empty() && ids.len() <= 16,
        "Select 1–16 catalog packages"
    );
    ensure!(
        ids.iter().collect::<BTreeSet<_>>().len() == ids.len(),
        "Duplicate selection"
    );
    ids.iter().map(|id| package(id)).collect()
}
pub fn download(root: &Path, ids: &[String], progress: &Progress) -> Result<String> {
    let packages = selection(ids)?;
    let cache = cache_dir(root);
    directory(&cache)?;
    for p in &packages {
        checkpoint(progress)?;
        let path = cache.join(&p.filename);
        if path.exists() {
            let _held = verified(p, &path, progress)?;
            continue;
        }
        progress.set_phase(format!(
            "Download {} {} ({} MiB)",
            p.name,
            p.arch,
            p.bytes / 1048576
        ));
        #[cfg(windows)]
        durable::write_with(&path, false, |out| {
            windows::download(p, out, progress).map_err(std::io::Error::other)
        })?;
        #[cfg(not(windows))]
        bail!("Official downloads require Windows");
        let _held = verified(p, &path, progress)?;
    }
    Ok(format!("{} installer(s) cached and verified. Nothing installed. Partial downloads are never published.",packages.len()))
}

#[derive(Clone, Debug)]
struct Manifest {
    schema: u32,
    catalog: String,
    packages: Vec<String>,
}
crate::json_struct!(Manifest,{schema,catalog,packages});
fn read_manifest(dir: &Path) -> Result<Manifest> {
    let mut f = lock(&dir.join("bundle.json"))?;
    ensure!(
        f.file.metadata()?.len() <= 32768,
        "Bundle manifest exceeds 32 KiB"
    );
    let mut bytes = Vec::new();
    Read::by_ref(&mut f.file)
        .take(32769)
        .read_to_end(&mut bytes)?;
    ensure!(bytes.len() <= 32768, "Bundle manifest grew beyond limit");
    let value: json::Value = json::from_json_slice(&bytes)?;
    let fields = value
        .as_object()
        .context("Bundle manifest must be an object")?;
    ensure!(
        fields.len() == 3
            && fields
                .iter()
                .all(|(k, _)| matches!(k.as_str(), "schema" | "catalog" | "packages")),
        "Unknown bundle fields; external URLs, hashes and commands are not accepted"
    );
    let m: Manifest = json::from_json_slice(&bytes)?;
    ensure!(
        m.schema == 1 && m.catalog == catalog()?.verified,
        "Unsupported catalog revision; use a matching SetupScope build"
    );
    selection(&m.packages)?;
    Ok(m)
}
fn copy_verified(p: &Package, from: &Path, to: &Path, progress: &Progress) -> Result<()> {
    let mut held = verified(p, from, progress)?;
    durable::write_with(to, false, |out| {
        let mut buf = [0u8; 65536];
        let mut total = 0u64;
        loop {
            checkpoint(progress).map_err(std::io::Error::other)?;
            let n = held.file.read(&mut buf)?;
            if n == 0 {
                break;
            }
            total += n as u64;
            if total > p.bytes {
                return Err(std::io::Error::other("Source grew during copy"));
            }
            out.write_all(&buf[..n])?;
        }
        if total != p.bytes {
            return Err(std::io::Error::other("Source changed during copy"));
        }
        Ok(())
    })?;
    let _held = verified(p, to, progress)?;
    Ok(())
}
pub fn export_bundle(
    root: &Path,
    ids: &[String],
    destination: &Path,
    license_reviewed: bool,
    progress: &Progress,
) -> Result<String> {
    ensure!(
        license_reviewed,
        "Review vendor licensing and redistribution before export"
    );
    let packages = selection(ids)?;
    local_path(destination)?;
    // Validate every source before creating any destination.
    for p in &packages {
        let _held = verified(p, &cache_dir(root).join(&p.filename), progress)?;
    }
    fs::create_dir(destination)
        .context("Choose a new bundle folder; existing folders are never replaced")?;
    for p in &packages {
        progress.set_phase(format!("Export {}", p.filename));
        copy_verified(
            p,
            &cache_dir(root).join(&p.filename),
            &destination.join(&p.filename),
            progress,
        )?;
    }
    let manifest = Manifest {
        schema: 1,
        catalog: catalog()?.verified,
        packages: ids.to_vec(),
    };
    checkpoint(progress)?;
    durable::write(
        &destination.join("bundle.json"),
        json::to_json_string_pretty(&manifest).as_bytes(),
        false,
    )?;
    Ok(format!("Offline bundle completed: {}. Only selected installers; workloads, packages and additional prerequisites are excluded.",destination.display()))
}
pub fn import_bundle(root: &Path, source: &Path, progress: &Progress) -> Result<String> {
    local_path(source)?;
    let manifest = read_manifest(source)?;
    let packages = selection(&manifest.packages)?;
    // Reject missing/tampered/unsigned source entries before cache publication.
    for p in &packages {
        let _held = verified(p, &source.join(&p.filename), progress)?;
    }
    let cache = cache_dir(root);
    directory(&cache)?;
    for p in &packages {
        checkpoint(progress)?;
        let destination = cache.join(&p.filename);
        if destination.exists() {
            let _held = verified(p, &destination, progress)?;
        } else {
            copy_verified(p, &source.join(&p.filename), &destination, progress)?;
        }
    }
    Ok(format!(
        "{} offline installer(s) imported and reverified. Nothing installed.",
        packages.len()
    ))
}

pub struct InstallReview {
    id: String,
    path: PathBuf,
    elevated: bool,
    text: String,
    _held: Locked,
    command: Option<String>,
}
impl InstallReview {
    pub fn text(&self) -> &str {
        &self.text
    }
}
pub fn review_install(
    root: &Path,
    id: &str,
    elevated: bool,
    progress: &Progress,
) -> Result<InstallReview> {
    let p = package(id)?;
    #[cfg(windows)]
    ensure!(
        p.arch != "arm64" || windows::native_arm64(),
        "ARM64 installers cannot be launched on this host"
    );
    let path = cache_dir(root).join(&p.filename);
    let held = verified(&p, &path, progress)?;
    let command = command_preview(&p, &path)?;
    let text=format!("{} {} / {}\nFile: {}\n{}: {}\nPublisher: {}\nCommand: {}\nAdministrator prompt requested: {}\n\nVendor interactive UI remains visible. Vendor installation can change Program Files, registry, PATH, shared runtimes and require a reboot. /norestart requests no automatic restart; vendor/OS behavior is outside SetupScope's control. Only the launched vendor process is tracked; detached children may continue. This is NOT a sandbox. No silent installation or automatic rollback. Recover with the vendor uninstaller/Installed Apps; that may affect other projects and may not restore prior state. Exit success alone does not prove build readiness.\n\n{}\nLicense: {}\nSource: {}\nSignature check uses cached Windows trust; no online revocation lookup. Review every vendor/UAC prompt yourself. The installer file and parent paths stay held during this review and launch; these are race-reduction safeguards, not a security boundary against other processes running as your user, administrators or kernel attackers.",p.name,p.version,p.arch,path.display(),p.algorithm,p.digest,p.publisher,command,elevated,p.notes,p.license_url,p.source);
    Ok(InstallReview {
        id: p.id,
        path,
        elevated,
        text,
        _held: held,
        command: None,
    })
}
fn command_preview(p: &Package, path: &Path) -> Result<String> {
    if p.kind == "msi" {
        Ok(format!(
            "\"{}\" /i \"{}\" /norestart",
            msiexec()?.display(),
            path.display()
        ))
    } else {
        Ok(format!("\"{}\" /install /norestart", path.display()))
    }
}
fn msiexec() -> Result<PathBuf> {
    #[cfg(windows)]
    {
        windows::msiexec()
    }
    #[cfg(not(windows))]
    {
        bail!("Installation requires Windows")
    }
}
#[derive(Clone, Debug)]
pub struct Receipt {
    pub schema: u32,
    pub id: String,
    pub package: String,
    pub version: String,
    pub arch: String,
    pub path: String,
    pub digest: String,
    pub command: String,
    pub elevated: bool,
    pub started: String,
    pub state: String,
    pub pid: u32,
    pub exit_code: Option<u32>,
}
crate::json_struct!(Receipt,{schema,id,package,version,arch,path,digest,command,elevated,started,state,pid,exit_code});
fn save_receipt(root: &Path, r: &Receipt, replace: bool) -> Result<()> {
    durable::write(
        &receipts_dir(root).join(format!("{}.json", r.id)),
        json::to_json_string_pretty(r).as_bytes(),
        replace,
    )?;
    Ok(())
}
trait Running {
    fn pid(&self) -> u32;
    fn wait(&self) -> Result<u32>;
}
static VENDOR_ACTIVE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
struct VendorGuard;
impl Drop for VendorGuard {fn drop(&mut self){VENDOR_ACTIVE.store(false,std::sync::atomic::Ordering::Release);}}
fn vendor_operation()->Result<VendorGuard>{
    ensure!(VENDOR_ACTIVE.compare_exchange(false,true,std::sync::atomic::Ordering::AcqRel,std::sync::atomic::Ordering::Acquire).is_ok(), "Another vendor operation is active; finish its UI before starting another");
    Ok(VendorGuard)
}
pub fn install(
    root: &Path,
    review: InstallReview,
    reviewed: bool,
    progress: &Progress,
) -> Result<String> {
    let _operation=vendor_operation()?;
    #[cfg(windows)]
    {
        let p = package(&review.id)?;
        install_with(
            root,
            review,
            reviewed,
            progress,
            p,
            signature,
            windows::launch,
        )
    }
    #[cfg(not(windows))]
    {
        let _ = (root, review, reviewed, progress);
        bail!("Installation requires Windows")
    }
}
fn install_with<P: Running>(
    root: &Path,
    mut review: InstallReview,
    reviewed: bool,
    progress: &Progress,
    p: Package,
    signature_check: impl Fn(&Package, &Path) -> Result<String>,
    launch: impl FnOnce(&Package, &Path, bool) -> Result<P>,
) -> Result<String> {
    ensure!(
        reviewed,
        "Review the exact installer and its system effects first"
    );
    checkpoint(progress)?;
    ensure!(p.id == review.id, "Review/package mismatch");
    check_bytes(&p, &mut review._held.file, progress)?;
    signature_check(&p, &review.path)?;
    directory(&receipts_dir(root))?;
    let mut receipt = Receipt {
        schema: 1,
        id: util::new_id(),
        package: p.name.clone(),
        version: p.version.clone(),
        arch: p.arch.clone(),
        path: review.path.display().to_string(),
        digest: p.digest.clone(),
        command: review.command.clone().unwrap_or(command_preview(&p, &review.path)?),
        elevated: review.elevated,
        started: util::now_string(),
        state: "prepared; launch not yet confirmed".into(),
        pid: 0,
        exit_code: None,
    };
    save_receipt(root, &receipt, false)?;
    checkpoint(progress)?;
    let launched = launch(&p, &review.path, review.elevated);
    let process = match launched {
        Ok(v) => v,
        Err(e) => {
            receipt.state =
                format!("Launch not confirmed: {e:#}; inspect vendor state before retrying");
            save_receipt(root, &receipt, true)?;
            return Err(e);
        }
    };
    {
        receipt.pid = process.pid();
        receipt.state = "Installer launched; completion not yet recorded".into();
        // Never abandon tracking/locks solely because an intermediate receipt write fails.
        let interim = save_receipt(root, &receipt, true).err();
        progress.set_phase(
            "Vendor installer active — complete or cancel its UI. SetupScope cannot cancel it.",
        );
        let code = process.wait();
        match code {
            Ok(code) => {
                receipt.exit_code = Some(code);
                receipt.state = match code {
                    0 => "Vendor reported success; readiness unverified",
                    3010 => "Vendor reported success; reboot required",
                    1641 => "Vendor reported reboot initiated",
                    1602 => "Vendor installation cancelled",
                    _ => "Vendor returned a failure/other code",
                }
                .into();
            }
            Err(e) => receipt.state = format!("Completion unknown: {e:#}"),
        };
        save_receipt(root,&receipt,true).context("Installer may have changed this PC, but final receipt could not be saved; inspect vendor state")?;
        if let Some(e) = interim {
            return Ok(format!(
                "{} (exit {:?}). Intermediate receipt write failed: {e:#}; final receipt saved.",
                receipt.state, receipt.exit_code
            ));
        }
    }
    Ok(format!(
        "{} (exit {:?}). Receipt {}. Reinspect toolchain readiness separately.",
        receipt.state, receipt.exit_code, receipt.id
    ))
}
pub fn recent_receipts(root: &Path) -> Result<Vec<Receipt>> {
    let dir = receipts_dir(root);
    local_path(&dir)?;
    if !dir.exists() {
        return Ok(Vec::new());
    }
    let entries: Vec<_> = fs::read_dir(dir)?
        .take(257)
        .collect::<std::io::Result<_>>()?;
    ensure!(
        entries.len() <= 256,
        "Receipt directory exceeds 256 entries; inspect it directly"
    );
    let mut paths = Vec::new();
    for e in entries {
        let p = e.path();
        if p.extension().is_some_and(|x| x == "json") {
            paths.push(p);
        }
    }
    ensure!(
        paths.len() <= 256,
        "More than 256 receipt entries; inspect the receipt folder directly"
    );
    paths.sort();
    paths.reverse();
    paths
        .into_iter()
        .take(20)
        .map(|p| {
            let mut f = lock(&p)?;
            ensure!(f.file.metadata()?.len() <= 32768, "Oversized receipt");
            let mut b = Vec::new();
            Read::by_ref(&mut f.file).take(32769).read_to_end(&mut b)?;
            ensure!(b.len() <= 32768, "Oversized receipt");
            let r: Receipt = json::from_json_slice(&b)?;
            ensure!(r.schema == 1, "Unsupported receipt");
            Ok(r)
        })
        .collect()
}
