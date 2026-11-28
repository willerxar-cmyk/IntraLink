//! Read-only physical cursor and primary monitor metadata, using a scoped
//! per-monitor DPI context. No hooks, clipping, key state, or cursor warps.
use anyhow::{Result, ensure};
use intralink_core::physical_edge::PhysicalScreen;
use windows::Win32::{
    Foundation::POINT,
    Graphics::Gdi::{GetMonitorInfoW, MONITOR_DEFAULTTOPRIMARY, MONITORINFO, MonitorFromPoint},
    UI::{
        HiDpi::{
            DPI_AWARENESS_CONTEXT, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
            SetThreadDpiAwarenessContext,
        },
        WindowsAndMessaging::{
            GetPhysicalCursorPos, GetSystemMetrics, MONITORINFOF_PRIMARY, SM_CMONITORS,
        },
    },
};

/// A DPI change is strictly scoped to a synchronous call on this thread.
/// Do not keep this guard across an await or transfer it to another thread.
pub(crate) struct PhysicalDpi(DPI_AWARENESS_CONTEXT);
impl PhysicalDpi {
    pub(crate) fn enter() -> Result<Self> {
        // SAFETY: changes only the calling thread's DPI context, restored by Drop.
        let previous =
            unsafe { SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) };
        // The DPI_UNAWARE pseudohandle is -1 and is valid here; only NULL
        // means failure for SetThreadDpiAwarenessContext.
        ensure!(!previous.0.is_null(), "Physical DPI context unavailable");
        Ok(Self(previous))
    }
}
impl Drop for PhysicalDpi {
    fn drop(&mut self) {
        // SAFETY: restore the valid context returned by the same thread's call.
        let restored = unsafe { SetThreadDpiAwarenessContext(self.0) };
        debug_assert!(
            !restored.0.is_null(),
            "Failed to restore thread DPI context"
        );
    }
}

/// Create or query native resources synchronously using physical DPI.
pub fn with_physical_dpi<T>(operation: impl FnOnce() -> Result<T>) -> Result<T> {
    let _dpi = PhysicalDpi::enter()?;
    operation()
}

/// For now refuse multi-monitor local desktops: an OS neighbor must never
/// be confused with a remote target at the same physical edge.
pub fn physical_primary() -> Result<PhysicalScreen> {
    let _dpi = PhysicalDpi::enter()?;
    // SAFETY: read-only system metrics, no window handles or borrowed pointers.
    ensure!(
        // SAFETY: read-only count of monitors on the current desktop.
        unsafe { GetSystemMetrics(SM_CMONITORS) } == 1,
        "Physical-edge preview requires exactly one local monitor"
    );
    // SAFETY: a value POINT and documented primary-monitor selection flag.
    let monitor = unsafe { MonitorFromPoint(POINT { x: 0, y: 0 }, MONITOR_DEFAULTTOPRIMARY) };
    ensure!(!monitor.is_invalid(), "No physical primary monitor");
    let mut info = MONITORINFO {
        cbSize: std::mem::size_of::<MONITORINFO>() as u32,
        ..Default::default()
    };
    // SAFETY: initialized MONITORINFO of exactly the declared size, live through the call.
    ensure!(
        // SAFETY: monitor handle and initialized MONITORINFO are valid for this call.
        unsafe { GetMonitorInfoW(monitor, &mut info) }.as_bool(),
        "Physical monitor query failed"
    );
    ensure!(
        info.dwFlags & MONITORINFOF_PRIMARY != 0,
        "Expected primary monitor"
    );
    let rect = info.rcMonitor;
    let width = rect
        .right
        .checked_sub(rect.left)
        .ok_or_else(|| anyhow::anyhow!("Invalid physical width"))?
        .try_into()?;
    let height = rect
        .bottom
        .checked_sub(rect.top)
        .ok_or_else(|| anyhow::anyhow!("Invalid physical height"))?
        .try_into()?;
    PhysicalScreen::new(rect.left, rect.top, width, height)
        .ok_or_else(|| anyhow::anyhow!("Unsupported physical monitor geometry"))
}

/// Read-only visibility check for an existing, caller-owned UI indicator.
/// A minimized or destroyed indicator must stop the local diagnostic.
pub fn indicator_visible(handle: usize) -> bool {
    use windows::Win32::{
        Foundation::HWND,
        UI::WindowsAndMessaging::{IsIconic, IsWindow, IsWindowVisible},
    };
    if handle == 0 {
        return false;
    }
    let hwnd = HWND(handle as *mut std::ffi::c_void);
    // SAFETY: these read-only APIs accept a window handle value; IsWindow
    // rejects an invalid handle before the other queries.
    unsafe {
        IsWindow(Some(hwnd)).as_bool()
            && IsWindowVisible(hwnd).as_bool()
            && !IsIconic(hwnd).as_bool()
    }
}

pub fn physical_cursor() -> Result<(i32, i32)> {
    let mut point = POINT::default();
    // SAFETY: a writable initialized POINT, valid for the synchronous call.
    unsafe { GetPhysicalCursorPos(&mut point) }?;
    Ok((point.x, point.y))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[ignore = "requires the local Windows interactive desktop"]
    fn native_physical_monitor_and_cursor_can_be_read_without_input() -> Result<()> {
        use windows::Win32::UI::HiDpi::{
            AreDpiAwarenessContextsEqual, GetThreadDpiAwarenessContext,
        };
        // SAFETY: reads the calling thread's DPI context only.
        let before = unsafe { GetThreadDpiAwarenessContext() };
        let screen = physical_primary()?;
        let cursor = physical_cursor()?;
        // SAFETY: comparing two DPI contexts returned by the same thread.
        assert!(
            // SAFETY: query and compare DPI contexts on the same thread.
            unsafe { AreDpiAwarenessContextsEqual(before, GetThreadDpiAwarenessContext()) }
                .as_bool()
        );
        ensure!(
            screen.offset(cursor).is_some(),
            "Pointer not on the single interactive monitor"
        );
        println!(
            "NATIVE_PHYSICAL_READ_PASS size={:?}; DPI context restored; no pointer data logged",
            screen.size()
        );
        Ok(())
    }
}
