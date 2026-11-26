//! One unnamed high-resolution Windows timer, with no system-wide timer
//! resolution request and no spinning. Owned and used on one waiting thread.
use anyhow::{Context, Result, ensure};
use std::time::{Duration, Instant};
use windows::{
    Win32::{
        Foundation::{CloseHandle, HANDLE, WAIT_OBJECT_0},
        System::Threading::{
            CREATE_WAITABLE_TIMER_HIGH_RESOLUTION, CreateWaitableTimerExW,
            SYNCHRONIZATION_SYNCHRONIZE, SetWaitableTimer, TIMER_MODIFY_STATE, WaitForSingleObject,
        },
    },
    core::PCWSTR,
};

pub struct PreciseTimer {
    handle: Option<HANDLE>,
}

impl PreciseTimer {
    pub fn new() -> Result<Self> {
        // SAFETY: no inheritable security attributes, no shared name, one
        // auto-reset timer handle owned by this object.
        let handle = unsafe {
            CreateWaitableTimerExW(
                None,
                PCWSTR::null(),
                CREATE_WAITABLE_TIMER_HIGH_RESOLUTION,
                (SYNCHRONIZATION_SYNCHRONIZE | TIMER_MODIFY_STATE).0,
            )
        }
        .context("Create high-resolution waitable timer")?;
        Ok(Self {
            handle: Some(handle),
        })
    }

    pub fn wait(&mut self, duration: Duration) -> Result<()> {
        ensure!(
            duration <= Duration::from_secs(1),
            "Timer delay exceeds session bound"
        );
        let handle = self.handle.context("Timer is closed")?;
        let started = Instant::now();
        // Check elapsed time after each signal; never run an early catch-up
        // burst. Bound retries if the OS does not deliver a usable signal.
        for _ in 0..3 {
            let remaining = duration.saturating_sub(started.elapsed());
            if remaining.is_zero() {
                return Ok(());
            }
            let ticks = i64::try_from(remaining.as_nanos().div_ceil(100))?;
            let due = -ticks;
            let timeout = u32::try_from(remaining.as_millis().saturating_add(1000))?;
            // SAFETY: the live private timer is armed for a relative one-shot
            // delay, no APC, no periodic wake and no system resume request.
            let result = unsafe {
                SetWaitableTimer(handle, &due, 0, None, None, false)?;
                WaitForSingleObject(handle, timeout)
            };
            ensure!(
                result == WAIT_OBJECT_0,
                "High-resolution timer wait failed: {:?}",
                result
            );
        }
        ensure!(started.elapsed() >= duration, "Timer returned early");
        Ok(())
    }

    pub fn close(&mut self) -> Result<()> {
        if let Some(handle) = self.handle.take() {
            // SAFETY: this exact private handle is released once, after waiting.
            unsafe { CloseHandle(handle)? };
        }
        Ok(())
    }
}

impl Drop for PreciseTimer {
    fn drop(&mut self) {
        if let Err(error) = self.close() {
            eprintln!("Close precise timer: {error:#}");
        }
    }
}
