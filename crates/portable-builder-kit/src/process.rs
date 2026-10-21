//! Bounded, shell-free compiler supervision. Call on a worker thread.
//!
//! On Windows a kill-on-close Job Object owns the full compiler tree; a child
//! starts suspended and is assigned before its first instruction executes.
//! Unix uses direct-child cleanup only. This runner is for trusted build tools,
//! not for sandboxing arbitrary applications.
use std::ffi::OsString;
use std::io;
use std::path::PathBuf;
use std::time::{Duration, Instant};

/// An explicit executable and argument vector. No shell or UAC elevation.
#[derive(Debug, Clone)]
pub struct ToolRequest {
    pub executable: PathBuf,
    pub arguments: Vec<OsString>,
    pub working_directory: Option<PathBuf>,
    /// Must be positive. Includes time waiting for Windows descendants.
    pub timeout: Duration,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ToolOutcome {
    pub exit_code: u32,
    pub elapsed: Duration,
}

#[derive(Debug)]
pub enum ToolError {
    Cancelled,
    TimedOut,
    Failed(u32),
    Io(io::Error),
    Cleanup(io::Error),
}
impl std::fmt::Display for ToolError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Cancelled => f.write_str("Build cancelled"),
            Self::TimedOut => f.write_str("Build timed out. Check the tool configuration before retrying"),
            Self::Failed(code) => write!(f,"Build tool failed with exit code {code}"),
            Self::Io(error) => write!(f,"Could not run the build tool: {error}"),
            Self::Cleanup(error) => write!(f,"Compiler cleanup failed: {error}. Inspect running compiler processes before retrying"),
        }
    }
}
impl std::error::Error for ToolError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(error) | Self::Cleanup(error) => Some(error),
            _ => None,
        }
    }
}
impl From<io::Error> for ToolError {
    fn from(e: io::Error) -> Self {
        Self::Io(e)
    }
}

/// Polls cancellation at most every 50 ms between operating-system calls.
/// A cancellation already requested prevents process creation. On success,
/// Windows descendants have also finished; failed jobs terminate descendants.
/// Output is not captured in memory. Tool-generated build logs remain available.
pub fn run_tool(
    request: &ToolRequest,
    cancel: &dyn Fn() -> bool,
) -> Result<ToolOutcome, ToolError> {
    if request.timeout.is_zero() {
        return Err(
            io::Error::new(io::ErrorKind::InvalidInput, "tool timeout must be positive").into(),
        );
    }
    if cancel() {
        return Err(ToolError::Cancelled);
    }
    let mut request = request.clone();
    request.executable = std::fs::canonicalize(&request.executable)?;
    super::plain_file(&request.executable)?;
    if let Some(dir) = &request.working_directory {
        let dir = std::fs::canonicalize(dir)?;
        if !dir.is_dir() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "tool working directory must be a directory",
            )
            .into());
        }
        request.working_directory = Some(dir);
    }
    let start = Instant::now();
    if cancel() {
        return Err(ToolError::Cancelled);
    }
    let mut child = native::Process::spawn(&request)?;
    let result = (|| -> Result<ToolOutcome, ToolError> {
        loop {
            if cancel() {
                return Err(ToolError::Cancelled);
            }
            if start.elapsed() >= request.timeout {
                return Err(ToolError::TimedOut);
            }
            if let Some(code) = child.poll()? {
                if code != 0 {
                    return Err(ToolError::Failed(code));
                }
                return Ok(ToolOutcome {
                    exit_code: code,
                    elapsed: start.elapsed(),
                });
            }
            std::thread::sleep(
                Duration::from_millis(50).min(request.timeout.saturating_sub(start.elapsed())),
            );
        }
    })();
    child.stop().map_err(ToolError::Cleanup)?;
    result
}

#[cfg(not(windows))]
mod native {
    use super::*;
    use std::process::{Child, Command, Stdio};
    pub(super) struct Process(Child);
    impl Process {
        pub(super) fn spawn(request: &ToolRequest) -> io::Result<Self> {
            let mut cmd = Command::new(&request.executable);
            cmd.args(&request.arguments).stdin(Stdio::null());
            if let Some(dir) = &request.working_directory {
                cmd.current_dir(dir);
            }
            cmd.spawn().map(Self)
        }
        pub(super) fn stop(&mut self) -> io::Result<()> {
            if self.0.try_wait()?.is_none() {
                self.0.kill()?;
            }
            self.0.wait()?;
            Ok(())
        }
        pub(super) fn poll(&mut self) -> io::Result<Option<u32>> {
            self.0
                .try_wait()?
                .map(|status| {
                    status
                        .code()
                        .map(|code| code as u32)
                        .ok_or_else(|| io::Error::other("build tool terminated by a signal"))
                })
                .transpose()
        }
    }
    impl Drop for Process {
        fn drop(&mut self) {
            // Child::kill is harmless after exit; always reap to avoid zombies.
            if !matches!(self.0.try_wait(), Ok(Some(_))) {
                let _ = self.0.kill();
            }
            let _ = self.0.wait();
        }
    }
}

#[cfg(windows)]
mod native {
    use super::*;
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Foundation::{CloseHandle, HANDLE, WAIT_FAILED, WAIT_OBJECT_0};
    use windows_sys::Win32::System::JobObjects::*;
    use windows_sys::Win32::System::Threading::*;

    struct Handle(HANDLE);
    impl Drop for Handle {
        fn drop(&mut self) {
            unsafe {
                CloseHandle(self.0);
            }
        }
    }
    pub(super) struct Process {
        job: Handle,
        process: Handle,
        stopped: bool,
    }

    fn wide(value: &std::ffi::OsStr) -> io::Result<Vec<u16>> {
        let mut units: Vec<_> = value.encode_wide().collect();
        if units.contains(&0) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "NUL in tool argument/path",
            ));
        }
        units.push(0);
        Ok(units)
    }

    // Windows CRT quoting: double trailing backslashes and escape embedded quotes.
    fn quoted(value: &std::ffi::OsStr, out: &mut Vec<u16>) -> io::Result<()> {
        let units = wide(value)?;
        let argument = &units[..units.len() - 1];
        // Match conventional Windows argv encoding. Quoting simple switches
        // unnecessarily can confuse tools with their own command-line parser.
        if !argument.is_empty()
            && !argument
                .iter()
                .any(|u| matches!(*u, 9 | 10 | 11 | 13 | 32 | 34))
        {
            out.extend_from_slice(argument);
            return Ok(());
        }
        out.push(b'"' as u16);
        let mut slashes = 0;
        for &unit in &units[..units.len() - 1] {
            if unit == b'\\' as u16 {
                slashes += 1;
                continue;
            }
            if unit == b'"' as u16 {
                out.extend(std::iter::repeat_n(b'\\' as u16, slashes * 2 + 1));
            } else {
                out.extend(std::iter::repeat_n(b'\\' as u16, slashes));
            }
            slashes = 0;
            out.push(unit);
        }
        out.extend(std::iter::repeat_n(b'\\' as u16, slashes * 2));
        out.push(b'"' as u16);
        Ok(())
    }

    impl Process {
        pub(super) fn spawn(request: &ToolRequest) -> io::Result<Self> {
            let executable = wide(request.executable.as_os_str())?;
            let mut line = Vec::new();
            quoted(request.executable.as_os_str(), &mut line)?;
            for arg in &request.arguments {
                line.push(b' ' as u16);
                quoted(arg, &mut line)?;
            }
            line.push(0);
            if line.len() > 32767 {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "tool command line exceeds Windows limit",
                ));
            }
            let cwd = request
                .working_directory
                .as_ref()
                .map(|p| wide(p.as_os_str()))
                .transpose()?;
            unsafe {
                let job = CreateJobObjectW(std::ptr::null(), std::ptr::null());
                if job.is_null() {
                    return Err(io::Error::last_os_error());
                }
                let job = Handle(job);
                let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
                limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
                if SetInformationJobObject(
                    job.0,
                    JobObjectExtendedLimitInformation,
                    &limits as *const _ as *const _,
                    std::mem::size_of_val(&limits) as u32,
                ) == 0
                {
                    return Err(io::Error::last_os_error());
                }
                let mut startup: STARTUPINFOW = std::mem::zeroed();
                startup.cb = std::mem::size_of_val(&startup) as u32;
                let mut info: PROCESS_INFORMATION = std::mem::zeroed();
                if CreateProcessW(
                    executable.as_ptr(),
                    line.as_mut_ptr(),
                    std::ptr::null(),
                    std::ptr::null(),
                    0,
                    CREATE_SUSPENDED | CREATE_NO_WINDOW,
                    std::ptr::null(),
                    cwd.as_ref().map_or(std::ptr::null(), |v| v.as_ptr()),
                    &startup,
                    &mut info,
                ) == 0
                {
                    return Err(io::Error::last_os_error());
                }
                let process = Handle(info.hProcess);
                let thread = Handle(info.hThread);
                if AssignProcessToJobObject(job.0, process.0) == 0 {
                    let error = io::Error::last_os_error();
                    TerminateProcess(process.0, 1);
                    WaitForSingleObject(process.0, 5000);
                    return Err(error);
                }
                let child = Self {
                    job,
                    process,
                    stopped: false,
                };
                if ResumeThread(thread.0) == u32::MAX {
                    return Err(io::Error::last_os_error());
                }
                Ok(child)
            }
        }
        pub(super) fn stop(&mut self) -> io::Result<()> {
            if self.stopped {
                return Ok(());
            }
            unsafe {
                if TerminateJobObject(self.job.0, 1) == 0 {
                    return Err(io::Error::last_os_error());
                }
                let deadline = Instant::now() + Duration::from_secs(5);
                loop {
                    let mut accounting: JOBOBJECT_BASIC_ACCOUNTING_INFORMATION = std::mem::zeroed();
                    if QueryInformationJobObject(
                        self.job.0,
                        JobObjectBasicAccountingInformation,
                        &mut accounting as *mut _ as *mut _,
                        std::mem::size_of_val(&accounting) as u32,
                        std::ptr::null_mut(),
                    ) == 0
                    {
                        return Err(io::Error::last_os_error());
                    }
                    if accounting.ActiveProcesses == 0 {
                        self.stopped = true;
                        return Ok(());
                    }
                    if Instant::now() >= deadline {
                        return Err(io::Error::new(
                            io::ErrorKind::TimedOut,
                            "Windows compiler tree did not stop within five seconds",
                        ));
                    }
                    std::thread::sleep(Duration::from_millis(10));
                }
            }
        }
        pub(super) fn poll(&mut self) -> io::Result<Option<u32>> {
            unsafe {
                match WaitForSingleObject(self.process.0, 0) {
                    WAIT_FAILED => return Err(io::Error::last_os_error()),
                    WAIT_OBJECT_0 => {}
                    _ => return Ok(None),
                }
                let mut code = 0;
                if GetExitCodeProcess(self.process.0, &mut code) == 0 {
                    return Err(io::Error::last_os_error());
                }
                if code != 0 {
                    return Ok(Some(code));
                }
                let mut accounting: JOBOBJECT_BASIC_ACCOUNTING_INFORMATION = std::mem::zeroed();
                if QueryInformationJobObject(
                    self.job.0,
                    JobObjectBasicAccountingInformation,
                    &mut accounting as *mut _ as *mut _,
                    std::mem::size_of_val(&accounting) as u32,
                    std::ptr::null_mut(),
                ) == 0
                {
                    return Err(io::Error::last_os_error());
                }
                Ok((accounting.ActiveProcesses == 0).then_some(code))
            }
        }
    }
    impl Drop for Process {
        fn drop(&mut self) {
            unsafe {
                // Covers cancellation, timeout, polling error, failed tool and unwinding.
                if !self.stopped {
                    TerminateJobObject(self.job.0, 1);
                }
            }
        }
    }
}
