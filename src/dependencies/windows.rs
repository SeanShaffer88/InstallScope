use super::*;
use std::{
    ffi::c_void,
    mem::{size_of, zeroed},
    ptr::{null, null_mut},
    sync::atomic::Ordering,
    time::{Duration, Instant},
};
use windows_sys::Win32::{
    Foundation::{CloseHandle, HANDLE, WAIT_OBJECT_0},
    Networking::WinHttp::*,
    Security::Cryptography::*,
    System::Threading::{GetExitCodeProcess, GetProcessId, WaitForSingleObject},
    UI::Shell::{ShellExecuteExW, SEE_MASK_NOASYNC, SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW},
};
fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}
fn os(ok: i32) -> Result<()> {
    if ok == 0 {
        Err(std::io::Error::last_os_error().into())
    } else {
        Ok(())
    }
}
fn nt(status: i32) -> Result<()> {
    ensure!(
        status >= 0,
        "Windows cryptography error 0x{:08x}",
        status as u32
    );
    Ok(())
}
struct Hash {
    algorithm: BCRYPT_ALG_HANDLE,
    hash: BCRYPT_HASH_HANDLE,
    _object: Vec<u8>,
    len: usize,
}
impl Drop for Hash {
    fn drop(&mut self) {
        unsafe {
            if !self.hash.is_null() {
                BCryptDestroyHash(self.hash);
            }
            if !self.algorithm.is_null() {
                BCryptCloseAlgorithmProvider(self.algorithm, 0);
            }
        }
    }
}
impl Hash {
    fn new(name: &str) -> Result<Self> {
        let (algorithm, len) = match name {
            "SHA256" => (BCRYPT_SHA256_ALGORITHM, 32),
            "SHA512" => (BCRYPT_SHA512_ALGORITHM, 64),
            _ => bail!("Unsupported hash"),
        };
        let mut h = Self {
            algorithm: null_mut(),
            hash: null_mut(),
            _object: Vec::new(),
            len,
        };
        unsafe {
            nt(BCryptOpenAlgorithmProvider(
                &mut h.algorithm,
                algorithm,
                null(),
                0,
            ))?;
            let mut size = 0u32;
            let mut got = 0;
            nt(BCryptGetProperty(
                h.algorithm,
                BCRYPT_OBJECT_LENGTH,
                (&mut size as *mut u32).cast(),
                4,
                &mut got,
                0,
            ))?;
            ensure!(
                got == 4 && size < 1048576,
                "Invalid Windows hash object size"
            );
            h._object = vec![0u8; size as usize];
            nt(BCryptCreateHash(
                h.algorithm,
                &mut h.hash,
                h._object.as_mut_ptr(),
                size,
                null(),
                0,
                0,
            ))?;
        }
        Ok(h)
    }
    fn update(&self, b: &[u8]) -> Result<()> {
        unsafe { nt(BCryptHashData(self.hash, b.as_ptr(), b.len() as u32, 0)) }
    }
    fn finish(self) -> Result<String> {
        let mut out = vec![0u8; self.len];
        unsafe {
            nt(BCryptFinishHash(
                self.hash,
                out.as_mut_ptr(),
                out.len() as u32,
                0,
            ))?;
        }
        Ok(out.iter().map(|b| format!("{b:02x}")).collect())
    }
}
pub(super) fn hash(reader: &mut impl Read, algorithm: &str, p: &Progress) -> Result<String> {
    let h = Hash::new(algorithm)?;
    let mut buf = [0u8; 65536];
    let mut total = 0u64;
    loop {
        checkpoint(p)?;
        let n = reader.read(&mut buf)?;
        if n == 0 {
            break;
        }
        total += n as u64;
        ensure!(
            total <= 512 * 1024 * 1024,
            "Installer exceeds hashing limit"
        );
        h.update(&buf[..n])?;
        p.items.fetch_add(n as u64, Ordering::Relaxed);
    }
    h.finish()
}
struct Internet(*mut c_void);
impl Internet {
    fn new(h: *mut c_void) -> Result<Self> {
        ensure!(!h.is_null(), "WinHTTP: {}", std::io::Error::last_os_error());
        Ok(Self(h))
    }
}
impl Drop for Internet {
    fn drop(&mut self) {
        unsafe {
            WinHttpCloseHandle(self.0);
        }
    }
}
fn option(request: &Internet, key: u32, value: u32) -> Result<()> {
    unsafe {
        os(WinHttpSetOption(
            request.0,
            key,
            (&value as *const u32).cast(),
            4,
        ))
    }
}
fn header(request: &Internet, key: u32) -> Result<String> {
    let mut bytes = 0;
    unsafe {
        WinHttpQueryHeaders(request.0, key, null(), null_mut(), &mut bytes, null_mut());
    }
    ensure!(
        (2..=16384).contains(&bytes) && bytes % 2 == 0,
        "Missing or oversized HTTP header"
    );
    let mut buffer = vec![0u16; bytes as usize / 2];
    unsafe {
        os(WinHttpQueryHeaders(
            request.0,
            key,
            null(),
            buffer.as_mut_ptr().cast(),
            &mut bytes,
            null_mut(),
        ))?;
    }
    let end = buffer.iter().position(|c| *c == 0).unwrap_or(buffer.len());
    Ok(String::from_utf16(&buffer[..end])?)
}
pub(super) fn download(package: &Package, out: &mut File, p: &Progress) -> Result<()> {
    let agent = wide("SetupScope/0.1 Dependency Download");
    let session = Internet::new(unsafe {
        WinHttpOpen(
            agent.as_ptr(),
            WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY,
            null(),
            null(),
            0,
        )
    })?;
    unsafe {
        os(WinHttpSetTimeouts(session.0, 15000, 30000, 30000, 30000))?;
    }
    let mut url = package.url.clone();
    let started = Instant::now();
    for redirect in 0..=5 {
        checkpoint(p)?;
        ensure!(
            started.elapsed() < Duration::from_secs(1800),
            "Download timed out; retry later"
        );
        let (host, path) = allowed_url(&url)?;
        let host = wide(host);
        let path = wide(&format!("/{path}"));
        let verb = wide("GET");
        let connection =
            Internet::new(unsafe { WinHttpConnect(session.0, host.as_ptr(), 443, 0) })?;
        let request = Internet::new(unsafe {
            WinHttpOpenRequest(
                connection.0,
                verb.as_ptr(),
                path.as_ptr(),
                null(),
                null(),
                null(),
                WINHTTP_FLAG_SECURE,
            )
        })?;
        option(
            &request,
            WINHTTP_OPTION_REDIRECT_POLICY,
            WINHTTP_OPTION_REDIRECT_POLICY_NEVER,
        )?;
        option(
            &request,
            WINHTTP_OPTION_DISABLE_FEATURE,
            WINHTTP_DISABLE_COOKIES | WINHTTP_DISABLE_AUTHENTICATION,
        )?;
        unsafe {
            os(WinHttpSendRequest(request.0, null(), 0, null(), 0, 0, 0))?;
            os(WinHttpReceiveResponse(request.0, null_mut()))?;
        }
        let status = header(&request, WINHTTP_QUERY_STATUS_CODE)?.parse::<u32>()?;
        if matches!(status, 301 | 302 | 303 | 307 | 308) {
            ensure!(redirect < 5, "Too many redirects");
            url = header(&request, WINHTTP_QUERY_LOCATION)?;
            allowed_url(&url)?;
            continue;
        }
        ensure!(
            status == 200,
            "Official download returned HTTP {status}; retry later or update the catalog"
        );
        if let Ok(length) = header(&request, WINHTTP_QUERY_CONTENT_LENGTH) {
            ensure!(
                length.parse::<u64>()? == package.bytes,
                "Official download has an unexpected length"
            );
        }
        let hasher = Hash::new(&package.algorithm)?;
        let mut total = 0;
        let mut buffer = [0u8; 65536];
        loop {
            checkpoint(p)?;
            ensure!(
                started.elapsed() < Duration::from_secs(1800),
                "Download exceeded 30-minute limit"
            );
            let mut got = 0;
            unsafe {
                os(WinHttpReadData(
                    request.0,
                    buffer.as_mut_ptr().cast(),
                    buffer.len() as u32,
                    &mut got,
                ))?;
            }
            if got == 0 {
                break;
            }
            total += got as u64;
            ensure!(
                total <= package.bytes,
                "Official response exceeds pinned size"
            );
            hasher.update(&buffer[..got as usize])?;
            out.write_all(&buffer[..got as usize])?;
            p.items.fetch_add(got as u64, Ordering::Relaxed);
        }
        ensure!(
            total == package.bytes,
            "Download ended early; no partial installer published"
        );
        ensure!(
            hasher.finish()?.eq_ignore_ascii_case(&package.digest),
            "Downloaded installer failed its pinned digest; nothing published"
        );
        checkpoint(p)?;
        return Ok(());
    }
    bail!("Download redirect limit reached")
}
pub(super) fn native_arm64() -> bool {
    unsafe {
        let mut info = zeroed();
        windows_sys::Win32::System::SystemInformation::GetNativeSystemInfo(&mut info);
        info.Anonymous.Anonymous.wProcessorArchitecture == 12
    }
}
pub(super) fn msiexec() -> Result<PathBuf> {
    let mut buffer = [0u16; 32768];
    let len = unsafe {
        windows_sys::Win32::System::SystemInformation::GetSystemDirectoryW(
            buffer.as_mut_ptr(),
            buffer.len() as u32,
        )
    };
    ensure!(
        len > 0 && (len as usize) < buffer.len(),
        "Cannot resolve trusted Windows system directory"
    );
    Ok(PathBuf::from(String::from_utf16(&buffer[..len as usize])?).join("msiexec.exe"))
}
pub(super) struct Process(HANDLE);
impl Drop for Process {
    fn drop(&mut self) {
        unsafe {
            CloseHandle(self.0);
        }
    }
}
impl super::Running for Process {
    fn pid(&self) -> u32 {
        unsafe { GetProcessId(self.0) }
    }
    fn wait(&self) -> Result<u32> {
        ensure!(
            unsafe { WaitForSingleObject(self.0, u32::MAX) } == WAIT_OBJECT_0,
            "Could not wait for vendor installer; completion unknown"
        );
        let mut code = 0;
        unsafe {
            os(GetExitCodeProcess(self.0, &mut code))?;
        }
        Ok(code)
    }
}
pub(super) fn launch(p: &Package, path: &Path, elevated: bool) -> Result<Process> {
    // Use only fixed arguments and an absolute system executable for MSI.
    let exe = if p.kind == "msi" {
        msiexec()?
    } else {
        path.to_path_buf()
    };
    let args = if p.kind == "msi" {
        format!("/i \"{}\" /norestart", path.display())
    } else {
        "/install /norestart".into()
    };
    launch_command(
        &exe,
        &args,
        elevated,
        path.parent().context("Installer parent missing")?,
    )
}
pub(super) fn launch_command(
    exe: &Path,
    arguments: &str,
    elevated: bool,
    parent: &Path,
) -> Result<Process> {
    let exe = wide(&exe.to_string_lossy());
    let args = wide(arguments);
    let verb = wide(if elevated { "runas" } else { "open" });
    let parent = wide(&parent.to_string_lossy());
    let mut info: SHELLEXECUTEINFOW = unsafe { zeroed() };
    info.cbSize = size_of::<SHELLEXECUTEINFOW>() as u32;
    info.fMask = SEE_MASK_NOCLOSEPROCESS | SEE_MASK_NOASYNC;
    info.lpVerb = verb.as_ptr();
    info.lpFile = exe.as_ptr();
    info.lpParameters = args.as_ptr();
    info.lpDirectory = parent.as_ptr();
    info.nShow = 1;
    unsafe {
        os(ShellExecuteExW(&mut info))?;
    }
    ensure!(
        !info.hProcess.is_null(),
        "Vendor activation returned no process handle; check vendor state before retrying"
    );
    Ok(Process(info.hProcess))
}
