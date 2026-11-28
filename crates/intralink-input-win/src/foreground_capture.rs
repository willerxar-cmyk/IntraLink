//! Explicit, temporary foreground capture surface for the desktop KVM preview.
//! No global hooks, key logging, cursor clipping or foreground-lock bypass.
use crate::{
    physical::{PhysicalDpi, indicator_visible},
    physical_cursor,
};
use anyhow::{Result, ensure};
use intralink_core::physical_edge::PhysicalScreen;
use windows::Win32::{
    Foundation::{HWND, POINT, RECT},
    Graphics::Gdi::ClientToScreen,
    UI::{
        Input::KeyboardAndMouse::GetAsyncKeyState,
        WindowsAndMessaging::{
            GetClientRect, GetForegroundWindow, GetWindowThreadProcessId, HWND_TOPMOST,
            SWP_NOACTIVATE, SetForegroundWindow, SetPhysicalCursorPos, SetWindowPos,
            WindowFromPoint,
        },
    },
};

/// A boolean transition guard only; no key identity or text is returned,
/// retained or transmitted. Called only around explicitly approved handoffs.
pub fn neutral_for_handoff() -> bool {
    // SAFETY: synchronous read of the high (currently down) bit, ignoring
    // the unreliable historical low bit. No global hook is installed.
    (1..=254).all(|key| {
        // SAFETY: read a bounded virtual-key current-state bit only.
        let state = unsafe { GetAsyncKeyState(key) };
        state >= 0
    })
}

fn owned(handle: usize) -> Option<HWND> {
    if handle == 0 {
        return None;
    }
    let hwnd = HWND(handle as *mut std::ffi::c_void);
    let mut pid = 0;
    // SAFETY: read-only window-owner query with a live writable process ID.
    let thread = unsafe { GetWindowThreadProcessId(hwnd, Some(&mut pid)) };
    (thread != 0 && pid == std::process::id()).then_some(hwnd)
}

pub struct ForegroundCapture {
    handle: usize,
    screen: PhysicalScreen,
    region: PhysicalScreen,
    anchor: (i32, i32),
    thread: std::thread::ThreadId,
}
impl ForegroundCapture {
    /// The caller owns the visible window and has already obtained explicit
    /// local consent. Normal OS foreground activation may be refused.
    pub fn prepare(handle: usize, screen: PhysicalScreen) -> Result<Self> {
        Self::prepare_region(handle, screen, screen)
    }

    pub fn prepare_compact(
        handle: usize,
        screen: PhysicalScreen,
        point: (i32, i32),
    ) -> Result<Self> {
        let region = screen
            .panel(point, 640, 360)
            .ok_or_else(|| anyhow::anyhow!("Compact panel cannot fit approved primary monitor"))?;
        Self::prepare_region(handle, screen, region)
    }

    fn prepare_region(
        handle: usize,
        screen: PhysicalScreen,
        region: PhysicalScreen,
    ) -> Result<Self> {
        let hwnd = owned(handle)
            .ok_or_else(|| anyhow::anyhow!("Capture window is not owned by IntraLink"))?;
        let _dpi = PhysicalDpi::enter()?;
        let (x, y) = region.origin();
        let (w, h) = region.size();
        // SAFETY: only resize this process's explicitly created borderless
        // window. No other process's window is manipulated.
        unsafe {
            SetWindowPos(
                hwnd,
                Some(HWND_TOPMOST),
                x,
                y,
                i32::from(w),
                i32::from(h),
                SWP_NOACTIVATE,
            )
        }?;
        let result = Self {
            handle,
            screen,
            region,
            anchor: (x + i32::from(w) / 2, y + i32::from(h) / 2),
            thread: std::thread::current().id(),
        };
        if !result.geometry_matches() {
            let mut rect = RECT::default();
            let mut origin = POINT::default();
            // SAFETY: diagnostic queries for this owned window only.
            unsafe { GetClientRect(hwnd, &mut rect) }?;
            // SAFETY: initialized output point for the owned window.
            ensure!(
                // SAFETY: initialized client origin output for the owned window.
                unsafe { ClientToScreen(hwnd, &mut origin) }.as_bool(),
                "Owned capture client origin unavailable"
            );
            anyhow::bail!(
                "Owned capture client rectangle {}x{} at ({},{}), expected {}x{} at ({},{})",
                rect.right - rect.left,
                rect.bottom - rect.top,
                origin.x,
                origin.y,
                w,
                h,
                x,
                y
            );
        }
        Ok(result)
    }

    pub(crate) fn handle(&self) -> usize {
        self.handle
    }

    pub fn request_focus(&self) -> bool {
        if !self.geometry_matches() {
            return false;
        }
        let Some(hwnd) = owned(self.handle) else {
            return false;
        };
        // SAFETY: ordinary foreground request for the owned window. No bypass.
        let _ = unsafe { SetForegroundWindow(hwnd) };
        self.ready()
    }

    pub fn ready(&self) -> bool {
        if !self.geometry_matches() {
            return false;
        }
        let Some(hwnd) = owned(self.handle) else {
            return false;
        };
        // SAFETY: only compare the actual foreground HWND with our own.
        let foreground = unsafe { GetForegroundWindow() };
        foreground == hwnd
    }

    pub fn geometry_matches(&self) -> bool {
        if std::thread::current().id() != self.thread || !indicator_visible(self.handle) {
            return false;
        }
        let Some(hwnd) = owned(self.handle) else {
            return false;
        };
        let Ok(_dpi) = PhysicalDpi::enter() else {
            return false;
        };
        let mut rect = RECT::default();
        let mut origin = POINT::default();
        // SAFETY: both outputs are initialized, writable and valid
        // throughout these synchronous queries on the owned window.
        if unsafe { GetClientRect(hwnd, &mut rect) }.is_err() {
            return false;
        }
        // SAFETY: translates one initialized client point into screen coordinates.
        if !unsafe { ClientToScreen(hwnd, &mut origin) }.as_bool() {
            return false;
        }
        let (w, h) = self.region.size();
        rect.left == 0
            && rect.top == 0
            && rect.right == i32::from(w)
            && rect.bottom == i32::from(h)
            && (origin.x, origin.y) == self.region.origin()
    }

    fn owns_pointer(&self, point: (i32, i32)) -> bool {
        let Some(hwnd) = owned(self.handle) else {
            return false;
        };
        let Ok(_dpi) = PhysicalDpi::enter() else {
            return false;
        };
        if self.region.offset(point).is_none() {
            return false;
        }
        // SAFETY: read-only hit test; don't deliver remote input if another
        // window (for example an OS notification) covers the actual cursor.
        let hit = unsafe {
            WindowFromPoint(POINT {
                x: point.0,
                y: point.1,
            })
        };
        hit == hwnd
    }

    fn place(&self, point: (i32, i32)) -> Result<()> {
        ensure!(
            self.ready() && self.owns_pointer(point),
            "Foreground capture no longer owns the pointer destination"
        );
        // SAFETY: a bounded point inside the explicitly authorized, owned
        // foreground surface. This never changes clipping or global hooks.
        unsafe { SetPhysicalCursorPos(point.0, point.1) }?;
        ensure!(
            self.ready(),
            "Foreground capture lost during pointer placement"
        );
        Ok(())
    }

    pub fn center(&self) -> Result<()> {
        self.place(self.anchor)
    }

    pub fn owns_current_pointer(&self) -> bool {
        self.ready() && physical_cursor().is_ok_and(|point| self.owns_pointer(point))
    }

    /// Poll accelerated physical motion relative to the center, then recenter.
    /// This is a preview, not a lossless raw-input device reader.
    pub fn motion(&self) -> Result<(i32, i32)> {
        ensure!(self.ready(), "Foreground capture lost");
        let point = physical_cursor()?;
        ensure!(
            self.owns_pointer(point),
            "Pointer left the owned capture surface"
        );
        let delta = (point.0 - self.anchor.0, point.1 - self.anchor.1);
        if delta != (0, 0) {
            self.center()?;
        }
        Ok(delta)
    }

    /// Only after remote Deactivate has been acknowledged. The caller
    /// destroys its own capture window immediately after this placement.
    pub fn return_at(&self, x: u16, y: u16) -> Result<()> {
        ensure!(neutral_for_handoff(), "Local input is still held");
        let point = self.screen.from_normalized(x, y);
        if self.region == self.screen {
            return self.place(point);
        }
        // Compact mode may return outside the panel only after the caller
        // has released the old remote owner and closed Raw Input.
        ensure!(
            self.owns_current_pointer() && crate::physical_primary()? == self.screen,
            "Cannot return cursor: approved primary geometry or foreground changed"
        );
        // SAFETY: normalized point inside the same approved physical BASE;
        // this is cursor placement only, no click or keyboard injection.
        unsafe { SetPhysicalCursorPos(point.0, point.1) }?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn invalid_handles_cannot_become_capture_windows() {
        let screen = PhysicalScreen::new(0, 0, 1920, 1080).unwrap();
        assert!(ForegroundCapture::prepare(0, screen).is_err());
        assert!(ForegroundCapture::prepare(1, screen).is_err());
    }
}
