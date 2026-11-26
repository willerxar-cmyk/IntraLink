//! Launch the restricted decoder using an explicit primary token and exactly
//! three inherited standard handles. The child starts suspended, joins its
//! private job, and is then resumed. No arbitrary command line is accepted.
use std::{
    fs::File,
    io,
    mem::size_of,
    os::windows::{
        ffi::OsStrExt,
        io::{AsRawHandle, OwnedHandle},
        process::ExitStatusExt,
    },
    process::ExitStatus,
};

use anyhow::{Context, Result, ensure};
use windows::{
    Win32::{
        Foundation::{
            DUPLICATE_SAME_ACCESS, DuplicateHandle, HANDLE, HANDLE_FLAG_INHERIT, HANDLE_FLAGS,
            SetHandleInformation, WAIT_OBJECT_0, WAIT_TIMEOUT,
        },
        Security::SECURITY_ATTRIBUTES,
        System::{Pipes::CreatePipe, SystemInformation::GetWindowsDirectoryW, Threading::*},
    },
    core::{BOOL, PCWSTR, PWSTR},
};

use crate::{
    DecoderJob,
    decoder_token::{create, handle, owned},
};

struct Attributes {
    _storage: Vec<usize>,
    list: LPPROC_THREAD_ATTRIBUTE_LIST,
}

impl Attributes {
    fn new(handles: &[HANDLE; 3]) -> Result<Self> {
        let mut size = 0usize;
        // SAFETY: documented size query with null attribute list.
        unsafe {
            let _ = InitializeProcThreadAttributeList(None, 1, None, &mut size);
        }
        ensure!(
            (1..=65536).contains(&size),
            "Invalid startup attribute size"
        );
        let mut storage = vec![0usize; size.div_ceil(size_of::<usize>())];
        let list = LPPROC_THREAD_ATTRIBUTE_LIST(storage.as_mut_ptr().cast());
        // SAFETY: native-aligned stable storage and one requested attribute.
        unsafe {
            InitializeProcThreadAttributeList(Some(list), 1, None, &mut size)?;
        }
        let attributes = Self {
            _storage: storage,
            list,
        };
        // SAFETY: exact three live inheritable handles; the caller retains the
        // array until after CreateProcessAsUserW and attribute-list teardown.
        unsafe {
            UpdateProcThreadAttribute(
                attributes.list,
                0,
                PROC_THREAD_ATTRIBUTE_HANDLE_LIST as usize,
                Some(handles.as_ptr().cast()),
                size_of::<[HANDLE; 3]>(),
                None,
                None,
            )?;
        }
        Ok(attributes)
    }
}
impl Drop for Attributes {
    fn drop(&mut self) {
        // SAFETY: this initialized list is deleted before its storage drops.
        unsafe {
            DeleteProcThreadAttributeList(self.list);
        }
    }
}

fn pipe(parent_reads: bool) -> Result<(OwnedHandle, OwnedHandle)> {
    let mut read = HANDLE::default();
    let mut write = HANDLE::default();
    let attributes = SECURITY_ATTRIBUTES {
        nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
        bInheritHandle: BOOL(1),
        ..Default::default()
    };
    // SAFETY: two distinct initialized outputs, private default DACL and
    // inheritable child handle. The parent end is immediately made private.
    unsafe {
        CreatePipe(&mut read, &mut write, Some(&attributes), 0)?;
    }
    let read = owned(read)?;
    let write = owned(write)?;
    let (parent, child) = if parent_reads {
        (read, write)
    } else {
        (write, read)
    };
    // SAFETY: modify inheritance only on this newly owned parent pipe end.
    unsafe {
        SetHandleInformation(handle(&parent), HANDLE_FLAG_INHERIT.0, HANDLE_FLAGS(0))?;
    }
    Ok((parent, child))
}

fn diagnostic_stderr() -> Result<OwnedHandle> {
    let stderr = io::stderr();
    let mut raw = HANDLE::default();
    // SAFETY: duplicate only the parent's existing standard error handle;
    // the process pseudo-handle is never inherited. Same-access duplication
    // does not grant access to another resource.
    unsafe {
        let process = GetCurrentProcess();
        DuplicateHandle(
            process,
            HANDLE(stderr.as_raw_handle()),
            process,
            &mut raw,
            0,
            true,
            DUPLICATE_SAME_ACCESS,
        )?;
    }
    owned(raw)
}

fn windows_directory() -> Result<Vec<u16>> {
    let mut path = vec![0u16; 32768];
    // SAFETY: initialized writable UTF-16 buffer.
    let count = unsafe { GetWindowsDirectoryW(Some(&mut path)) } as usize;
    ensure!(
        count > 0 && count < path.len(),
        "Windows directory unavailable"
    );
    path.truncate(count);
    Ok(path)
}

fn minimal_environment(windows: &[u16]) -> Vec<u16> {
    // Explicit environment: do not inherit arbitrary credentials, user
    // configuration, PATH or application secrets into the reduced child.
    let mut env = Vec::new();
    for key in ["SystemRoot=", "WINDIR="] {
        env.extend(key.encode_utf16());
        env.extend_from_slice(windows);
        env.push(0);
    }
    env.push(0);
    env
}

pub struct RestrictedDecoderChild {
    process: OwnedHandle,
    pid: u32,
}

impl RestrictedDecoderChild {
    pub fn spawn() -> Result<(Self, DecoderJob, File, File)> {
        let token = create()?;
        let job = DecoderJob::new()?;
        let (input, child_input) = pipe(false)?;
        let (output, child_output) = pipe(true)?;
        let error = diagnostic_stderr()?;
        let handles = [handle(&child_input), handle(&child_output), handle(&error)];
        let attributes = Attributes::new(&handles)?;

        let executable = std::env::current_exe()?;
        let path: Vec<u16> = executable.as_os_str().encode_wide().collect();
        ensure!(
            !path.contains(&0) && !path.contains(&34),
            "Invalid executable path"
        );
        let mut application = path.clone();
        application.push(0);
        let mut command = vec![34u16];
        command.extend(path);
        command.extend("\" desktop-restricted-decoder-worker".encode_utf16());
        command.push(0);
        let mut directory = windows_directory()?;
        let env = minimal_environment(&directory);
        directory.push(0);
        let startup = STARTUPINFOEXW {
            StartupInfo: STARTUPINFOW {
                cb: size_of::<STARTUPINFOEXW>() as u32,
                dwFlags: STARTF_USESTDHANDLES,
                hStdInput: handles[0],
                hStdOutput: handles[1],
                hStdError: handles[2],
                ..Default::default()
            },
            lpAttributeList: attributes.list,
        };
        let mut info = PROCESS_INFORMATION::default();
        // SAFETY: explicit own executable, mutable NUL-terminated command,
        // reduced primary token, explicit minimal Unicode environment and
        // handle allow-list. No external command/path, credentials or process
        // handle is accepted. The initial thread remains suspended.
        unsafe {
            CreateProcessAsUserW(
                Some(handle(&token)),
                PCWSTR(application.as_ptr()),
                Some(PWSTR(command.as_mut_ptr())),
                None,
                None,
                true,
                CREATE_SUSPENDED
                    | CREATE_UNICODE_ENVIRONMENT
                    | EXTENDED_STARTUPINFO_PRESENT
                    | CREATE_NO_WINDOW,
                Some(env.as_ptr().cast()),
                PCWSTR(directory.as_ptr()),
                &startup.StartupInfo,
                &mut info,
            )
        }
        .context("Launch restricted decoder process")?;
        let process = owned(info.hProcess)?;
        let child = Self {
            process,
            pid: info.dwProcessId,
        };
        let thread = owned(info.hThread)?;
        job.assign(handle(&child.process))
            .context("Attach suspended restricted decoder")?;
        // SAFETY: only this newly created primary thread; resume once after
        // the private job is attached. Failure destroys our owned child.
        let previous = unsafe { ResumeThread(handle(&thread)) };
        ensure!(previous == 1, "Unexpected restricted decoder suspend count");
        drop(thread);
        drop(child_input);
        drop(child_output);
        drop(error);
        Ok((child, job, File::from(input), File::from(output)))
    }

    pub fn id(&self) -> u32 {
        self.pid
    }

    pub fn try_wait(&mut self) -> Result<Option<ExitStatus>> {
        // SAFETY: poll only this owned process handle, never a PID lookup.
        match unsafe { WaitForSingleObject(handle(&self.process), 0) } {
            WAIT_TIMEOUT => Ok(None),
            WAIT_OBJECT_0 => {
                let mut code = 0;
                // SAFETY: signaled process handle, initialized exit code output.
                unsafe {
                    GetExitCodeProcess(handle(&self.process), &mut code)?;
                }
                Ok(Some(ExitStatus::from_raw(code)))
            }
            _ => Err(windows::core::Error::from_thread().into()),
        }
    }
}

impl Drop for RestrictedDecoderChild {
    fn drop(&mut self) {
        if self.try_wait().is_ok_and(|status| status.is_none()) {
            // SAFETY: only the exact owned child can be stopped. This covers
            // startup errors before job assignment as well as normal RAII.
            if let Err(error) = unsafe { TerminateProcess(handle(&self.process), 1) } {
                eprintln!("Restricted decoder cleanup: {error}");
            }
        }
    }
}
