//! Native operations exclusively for the process-owned diagnostic window.
use anyhow::{Result, ensure};
use windows::Win32::{
    Foundation::{HWND, RECT},
    UI::WindowsAndMessaging::{
        GetWindowRect, GetWindowThreadProcessId, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOZORDER,
        SetWindowPos,
    },
};

/// Resize only a window belonging to the calling process. The input is a
/// native handle, not a memory address to dereference.
pub fn shrink_owned_window(handle: usize) -> Result<()> {
    ensure!(handle != 0, "Missing diagnostic window");
    let hwnd = HWND(handle as *mut core::ffi::c_void);
    let mut pid = 0;
    let mut rect = RECT::default();
    // SAFETY: the OS validates HWND; output pointers refer to live locals.
    // Only the current process's own window may be changed.
    unsafe {
        ensure!(
            GetWindowThreadProcessId(hwnd, Some(&mut pid)) != 0 && pid == std::process::id(),
            "Diagnostic window is not owned by this process"
        );
        GetWindowRect(hwnd, &mut rect)?;
        let width = rect.right.checked_sub(rect.left).filter(|n| *n >= 128);
        let height = rect.bottom.checked_sub(rect.top).filter(|n| *n >= 128);
        ensure!(
            width.is_some() && height.is_some(),
            "Invalid diagnostic window geometry"
        );
        SetWindowPos(
            hwnd,
            None,
            0,
            0,
            width.unwrap() / 2,
            height.unwrap() / 2,
            SWP_NOACTIVATE | SWP_NOMOVE | SWP_NOZORDER,
        )?;
    }
    Ok(())
}
