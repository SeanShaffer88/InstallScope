use super::*;
use anyhow::bail;
use std::fs::File;
use std::{
    mem::{size_of, zeroed},
    os::windows::io::{AsRawHandle, FromRawHandle},
    ptr::{null, null_mut},
    thread,
    time::Instant,
};
use windows_sys::Win32::{
    Foundation::*,
    Security::SECURITY_ATTRIBUTES,
    System::{JobObjects::*, Pipes::CreatePipe, Threading::*},
};
struct Handle(HANDLE);
impl Drop for Handle {
    fn drop(&mut self) {
        unsafe {
            if !self.0.is_null() && self.0 != INVALID_HANDLE_VALUE {
                CloseHandle(self.0);
            }
        }
    }
}
fn check(ok: i32) -> Result<()> {
    if ok == 0 {
        Err(std::io::Error::last_os_error().into())
    } else {
        Ok(())
    }
}
fn wide(s: &std::ffi::OsStr) -> Vec<u16> {
    use std::os::windows::ffi::OsStrExt;
    s.encode_wide().chain(Some(0)).collect()
}
fn pipe() -> Result<(File, Handle)> {
    unsafe {
        let attributes = SECURITY_ATTRIBUTES {
            nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: null_mut(),
            bInheritHandle: 1,
        };
        let (mut read, mut write) = (null_mut(), null_mut());
        check(CreatePipe(&mut read, &mut write, &attributes, 0))?;
        let writer = Handle(write);
        let reader = File::from_raw_handle(read);
        check(SetHandleInformation(read, HANDLE_FLAG_INHERIT, 0))?;
        Ok((reader, writer))
    }
}
pub(super) fn execute(request: &Request, progress: &Progress) -> Result<Captured> {
    unsafe {
        let job = Handle(CreateJobObjectW(null(), null()));
        ensure!(
            !job.0.is_null(),
            "Cannot create execution containment: {}",
            std::io::Error::last_os_error()
        );
        let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = zeroed();
        limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        check(SetInformationJobObject(
            job.0,
            JobObjectExtendedLimitInformation,
            (&limits as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
            size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
        ))?;
        let (out_reader, out_writer) = pipe()?;
        let (err_reader, err_writer) = pipe()?;
        // Restrict inherited handles to these three, not unrelated application handles.
        let mut nul_options = OpenOptions::new();
        nul_options.read(true);
        let nul = nul_options.open("NUL")?;
        let mut stdin = null_mut();
        check(DuplicateHandle(
            GetCurrentProcess(),
            nul.as_raw_handle(),
            GetCurrentProcess(),
            &mut stdin,
            0,
            1,
            DUPLICATE_SAME_ACCESS,
        ))?;
        let stdin = Handle(stdin);
        let mut attribute_bytes = 0;
        InitializeProcThreadAttributeList(null_mut(), 1, 0, &mut attribute_bytes);
        let mut attributes = vec![0usize; attribute_bytes.div_ceil(size_of::<usize>())];
        let list = attributes.as_mut_ptr().cast();
        check(InitializeProcThreadAttributeList(
            list,
            1,
            0,
            &mut attribute_bytes,
        ))?;
        struct Attributes(LPPROC_THREAD_ATTRIBUTE_LIST);
        impl Drop for Attributes {
            fn drop(&mut self) {
                unsafe {
                    DeleteProcThreadAttributeList(self.0);
                }
            }
        }
        let list = Attributes(list);
        let handles = [stdin.0, out_writer.0, err_writer.0];
        check(UpdateProcThreadAttribute(
            list.0,
            0,
            PROC_THREAD_ATTRIBUTE_HANDLE_LIST as usize,
            handles.as_ptr().cast(),
            size_of_val(&handles),
            null_mut(),
            null(),
        ))?;
        let mut startup: STARTUPINFOEXW = zeroed();
        startup.StartupInfo.cb = size_of::<STARTUPINFOEXW>() as u32;
        startup.lpAttributeList = list.0;
        startup.StartupInfo.dwFlags = STARTF_USESTDHANDLES;
        startup.StartupInfo.hStdInput = stdin.0;
        startup.StartupInfo.hStdOutput = out_writer.0;
        startup.StartupInfo.hStdError = err_writer.0;
        let app = wide(request.executable.as_os_str());
        let cwd = wide(request.project.as_os_str());
        let command = std::iter::once(quote(&request.executable.to_string_lossy()))
            .chain(request.arguments.iter().map(|s| quote(s)))
            .collect::<Vec<_>>()
            .join(" ");
        let mut command = wide(std::ffi::OsStr::new(&command));
        let mut info: PROCESS_INFORMATION = zeroed();
        check(CreateProcessW(
            app.as_ptr(),
            command.as_mut_ptr(),
            null(),
            null(),
            1,
            CREATE_SUSPENDED | CREATE_NO_WINDOW | EXTENDED_STARTUPINFO_PRESENT,
            null(),
            cwd.as_ptr(),
            &startup.StartupInfo,
            &mut info,
        ))?;
        let process = Handle(info.hProcess);
        let main_thread = Handle(info.hThread);
        if AssignProcessToJobObject(job.0, process.0) == 0 {
            let error = std::io::Error::last_os_error();
            TerminateProcess(process.0, 1);
            bail!("Containment unavailable; suspended process was not allowed to run: {error}");
        }
        if progress.cancelled() {
            TerminateJobObject(job.0, 1);
            bail!("Cancelled before process resumed");
        }
        let stdout = thread::Builder::new()
            .name("build-stdout".into())
            .spawn(move || read_bounded(out_reader, MAX_LOG / 2))?;
        let stderr = thread::Builder::new()
            .name("build-stderr".into())
            .spawn(move || read_bounded(err_reader, MAX_LOG / 2))?;
        if ResumeThread(main_thread.0) == u32::MAX {
            TerminateJobObject(job.0, 1);
            bail!(
                "Could not resume contained process: {}",
                std::io::Error::last_os_error()
            );
        }
        drop(out_writer);
        drop(err_writer);
        drop(stdin);
        let started = Instant::now();
        progress.set_phase("Running reviewed project command");
        let state = loop {
            let wait = WaitForSingleObject(process.0, 100);
            if wait == WAIT_OBJECT_0 {
                break "Completed";
            }
            if wait == WAIT_FAILED {
                TerminateJobObject(job.0, 1);
                break "Wait failed; stop requested";
            }
            if progress.cancelled() {
                TerminateJobObject(job.0, 1);
                break "Cancelled; stop requested";
            }
            if started.elapsed() >= Duration::from_secs(request.timeout_seconds.into()) {
                TerminateJobObject(job.0, 1);
                break "Timed out; stop requested";
            }
        };
        // Even successful tools may leave watchers/background children; none are retained.
        check(TerminateJobObject(job.0, 1))?;
        WaitForSingleObject(process.0, 5000);
        let mut exit = 0;
        check(GetExitCodeProcess(process.0, &mut exit))?;
        let (out, ot) = stdout
            .join()
            .map_err(|_| anyhow::anyhow!("Stdout reader failed"))?;
        let (err, et) = stderr
            .join()
            .map_err(|_| anyhow::anyhow!("Stderr reader failed"))?;
        Ok((exit, state.into(), out, err, ot || et))
    }
}
