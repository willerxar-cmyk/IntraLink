//! Irreversible process-local hardening, applied only inside a diagnostic
//! decoder child. No system policy or parent process token is changed.
use std::mem::size_of;

use anyhow::{Result, ensure};
use windows::Win32::System::{
    SystemServices::{
        PROCESS_MITIGATION_CHILD_PROCESS_POLICY, PROCESS_MITIGATION_EXTENSION_POINT_DISABLE_POLICY,
        PROCESS_MITIGATION_IMAGE_LOAD_POLICY,
    },
    Threading::{
        GetCurrentProcess, GetProcessMitigationPolicy, PROCESS_MITIGATION_POLICY,
        ProcessChildProcessPolicy, ProcessExtensionPointDisablePolicy, ProcessImageLoadPolicy,
        SetProcessMitigationPolicy,
    },
};

fn require_flags(policy: PROCESS_MITIGATION_POLICY, flags: u32) -> Result<()> {
    let mut current = 0u32;
    // SAFETY: each caller uses a Windows policy with an exact 32-bit Flags
    // union. Query the current process only, never an external handle.
    unsafe {
        GetProcessMitigationPolicy(
            GetCurrentProcess(),
            policy,
            (&mut current as *mut u32).cast(),
            size_of::<u32>(),
        )?;
    }
    if current & flags != flags {
        let requested = current | flags;
        // SAFETY: the selected policies are exact 32-bit bitfield structures;
        // retain existing settings, adding only the fixed restrictive bits.
        unsafe {
            SetProcessMitigationPolicy(
                policy,
                (&requested as *const u32).cast(),
                size_of::<u32>(),
            )?;
            GetProcessMitigationPolicy(
                GetCurrentProcess(),
                policy,
                (&mut current as *mut u32).cast(),
                size_of::<u32>(),
            )?;
        }
    }
    ensure!(
        current & flags == flags,
        "Decoder mitigation readback mismatch: {policy:?}"
    );
    Ok(())
}

/// Apply before any native GPU initialization or packet parsing. This
/// remains defense in depth: Win32k and dynamic code are intentionally not
/// blocked because the existing DXGI window and driver may require them.
pub fn harden_decoder_worker(restricted: bool) -> Result<()> {
    crate::decoder_job::require_decoder_job()?;
    if restricted {
        crate::decoder_token::verify_current()?;
    }
    const _: () = assert!(size_of::<PROCESS_MITIGATION_IMAGE_LOAD_POLICY>() == 4);
    const _: () = assert!(size_of::<PROCESS_MITIGATION_EXTENSION_POINT_DISABLE_POLICY>() == 4);
    const _: () = assert!(size_of::<PROCESS_MITIGATION_CHILD_PROCESS_POLICY>() == 4);
    // IMAGE_LOAD: NoRemoteImages, NoLowMandatoryLabelImages,
    // PreferSystem32Images. The process loads only its system driver DLLs.
    require_flags(ProcessImageLoadPolicy, 0b111)?;
    require_flags(ProcessExtensionPointDisablePolicy, 1)?;
    // NoChildProcessCreation. The job's active-process limit also applies.
    require_flags(ProcessChildProcessPolicy, 1)?;
    Ok(())
}
