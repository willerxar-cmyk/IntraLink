//! Lifetime containment for the owned diagnostic decoder child.
//! This job is not a sandbox or a restricted access token.
use std::{os::windows::io::AsRawHandle, process::Child};

use anyhow::{Result, ensure};
use windows::{
    Win32::{
        Foundation::{CloseHandle, HANDLE},
        System::{
            JobObjects::{
                AssignProcessToJobObject, CreateJobObjectW, IsProcessInJob,
                JOB_OBJECT_LIMIT_ACTIVE_PROCESS, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
                JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation,
                QueryInformationJobObject, SetInformationJobObject, TerminateJobObject,
            },
            Threading::GetCurrentProcess,
        },
    },
    core::{BOOL, PCWSTR},
};

/// An unnamed, non-inheritable job owning only the explicitly spawned child.
pub struct DecoderJob {
    handle: Option<HANDLE>,
}

impl DecoderJob {
    pub fn attach(child: &Child) -> Result<Self> {
        // SAFETY: null security attributes make the unnamed job private and
        // non-inheritable. The child process handle is owned by Child.
        let handle = unsafe { CreateJobObjectW(None, PCWSTR::null())? };
        let job = Self {
            handle: Some(handle),
        };
        let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
        limits.BasicLimitInformation.LimitFlags =
            JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE | JOB_OBJECT_LIMIT_ACTIVE_PROCESS;
        limits.BasicLimitInformation.ActiveProcessLimit = 1;
        // SAFETY: live job and borrowed child handles; the initialized limits
        // buffer is valid for the entire call. No process ID lookup or reuse.
        unsafe {
            SetInformationJobObject(
                handle,
                JobObjectExtendedLimitInformation,
                (&limits as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
                std::mem::size_of_val(&limits) as u32,
            )?;
            AssignProcessToJobObject(handle, HANDLE(child.as_raw_handle()))?;
        }
        Ok(job)
    }

    pub fn terminate(&self) -> Result<()> {
        if let Some(handle) = self.handle {
            // SAFETY: this is our private job, never a caller-supplied job.
            unsafe { TerminateJobObject(handle, 1)? };
        }
        Ok(())
    }

    pub fn close(&mut self) -> Result<()> {
        if let Some(handle) = self.handle.take() {
            // SAFETY: release the one owned job handle exactly once. The OS
            // terminates any remaining job member, including on parent exit.
            unsafe { CloseHandle(handle)? };
        }
        Ok(())
    }
}

impl Drop for DecoderJob {
    fn drop(&mut self) {
        if let Err(error) = self.close() {
            eprintln!("Decoder job close: {error:#}");
        }
    }
}

/// Reject standalone worker invocations unless the current process belongs
/// to a job with the expected lifetime/child-count policy. This checks local
/// process containment, not the identity or trustworthiness of the parent.
pub(crate) fn require_decoder_job() -> Result<()> {
    let mut member = BOOL(0);
    let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
    // SAFETY: query the current process's enclosing job and write only the
    // initialized local output structures of the exact declared sizes.
    unsafe {
        IsProcessInJob(GetCurrentProcess(), None, &mut member)?;
        ensure!(member.as_bool(), "Decoder worker requires a private job");
        QueryInformationJobObject(
            None,
            JobObjectExtendedLimitInformation,
            (&mut limits as *mut JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
            std::mem::size_of_val(&limits) as u32,
            None,
        )?;
    }
    let basic = limits.BasicLimitInformation;
    ensure!(
        basic
            .LimitFlags
            .contains(JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE)
            && basic.LimitFlags.contains(JOB_OBJECT_LIMIT_ACTIVE_PROCESS)
            && basic.ActiveProcessLimit == 1,
        "Decoder worker job policy mismatch"
    );
    Ok(())
}
