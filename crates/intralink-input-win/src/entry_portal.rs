//! Tiny native, click-through destination cue. The premultiplied ARGB
//! surface is generated locally; no capture, hooks, cursor movement or focus.
use crate::physical::PhysicalDpi;
use anyhow::{Result, ensure};
use intralink_core::physical_edge::PhysicalScreen;
use std::{
    ffi::c_void,
    ptr::NonNull,
    time::{Duration, Instant},
};
use windows::{
    Win32::{
        Foundation::{COLORREF, HWND, POINT, SIZE},
        Graphics::Gdi::{
            AC_SRC_ALPHA, AC_SRC_OVER, BI_RGB, BITMAPINFO, BITMAPINFOHEADER, BLENDFUNCTION,
            CreateCompatibleDC, CreateDIBSection, DIB_RGB_COLORS, DeleteDC, DeleteObject, HBITMAP,
            HDC, HGDIOBJ, SelectObject,
        },
        UI::WindowsAndMessaging::{
            CreateWindowExW, DestroyWindow, HWND_TOPMOST, SW_HIDE, SWP_NOACTIVATE, SWP_SHOWWINDOW,
            SetWindowPos, ShowWindow, ULW_ALPHA, UpdateLayeredWindow, WS_EX_LAYERED,
            WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_EX_TRANSPARENT, WS_POPUP,
        },
    },
    core::w,
};

const DURATION: Duration = Duration::from_millis(220);
const FRAME: Duration = Duration::from_millis(16);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PortalStyle {
    Full,
    Reduced,
}
impl PortalStyle {
    fn radius(self) -> usize {
        match self {
            Self::Full => 44,
            Self::Reduced => 26,
        }
    }
    fn intensity(self) -> f32 {
        match self {
            Self::Full => 1.0,
            Self::Reduced => 0.5,
        }
    }
}

/// Draw an independent fading swirl/ring, not a screenshot distortion.
/// Transparent center leaves the system pointer unobstructed.
fn raster(pixels: &mut [u32], style: PortalStyle, progress: f32) {
    let radius = style.radius();
    let side = radius * 2 + 1;
    assert_eq!(pixels.len(), side * side);
    pixels.fill(0);
    if !progress.is_finite() || !(0.0..1.0).contains(&progress) {
        return;
    }
    let fade = (std::f32::consts::PI * progress).sin() * style.intensity();
    let r0 = 0.44 + 0.23 * progress;
    for y in 0..side {
        for x in 0..side {
            let dx = (x as f32 - radius as f32) / radius as f32;
            let dy = (y as f32 - radius as f32) / radius as f32;
            let r = (dx * dx + dy * dy).sqrt();
            if !(0.17..1.0).contains(&r) {
                continue;
            }
            let a = dy.atan2(dx);
            let ripple = (a * 2.0 - r * 13.0 + progress * 7.0).sin();
            let ring = (1.0 - ((r - r0 - ripple * 0.055) / 0.13).abs()).max(0.0);
            let halo = (1.0 - ((r - r0) / 0.30).abs()).max(0.0) * 0.25;
            let alpha = ((ring * 0.75 + halo) * fade * 220.0).clamp(0.0, 220.0) as u32;
            let channel = |v: u32| v * alpha / 255;
            let red = if ripple > 0.0 { 99 } else { 45 };
            let green = if ripple > 0.0 { 221 } else { 155 };
            let blue = if ripple > 0.0 { 194 } else { 245 };
            pixels[y * side + x] =
                (alpha << 24) | (channel(red) << 16) | (channel(green) << 8) | channel(blue);
        }
    }
}

struct Surface {
    dc: HDC,
    bitmap: HBITMAP,
    old: HGDIOBJ,
    bits: NonNull<u32>,
    side: usize,
}
impl Surface {
    fn new(side: usize) -> Result<Self> {
        // SAFETY: creates a private memory DC; no desktop pixel access.
        let dc = unsafe { CreateCompatibleDC(None) };
        ensure!(!dc.is_invalid(), "Could not allocate portal memory DC");
        let info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: side as i32,
                biHeight: -(side as i32),
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut bits: *mut c_void = std::ptr::null_mut();
        // SAFETY: initialized 32-bit top-down DIB description; writable pointer
        // receives the private allocation, valid until the bitmap is deleted.
        let bitmap = match unsafe {
            CreateDIBSection(Some(dc), &info, DIB_RGB_COLORS, &mut bits, None, 0)
        } {
            Ok(bitmap) => bitmap,
            Err(error) => {
                // SAFETY: DC is owned and has no selected custom object.
                let _ = unsafe { DeleteDC(dc) };
                return Err(error.into());
            }
        };
        let Some(bits) = NonNull::new(bits.cast::<u32>()) else {
            // SAFETY: both handles are owned and bitmap is not selected.
            unsafe {
                let _ = DeleteObject(HGDIOBJ(bitmap.0));
                let _ = DeleteDC(dc);
            }
            anyhow::bail!("Missing portal bitmap memory");
        };
        // SAFETY: select the owned bitmap into the owned memory DC.
        let old = unsafe { SelectObject(dc, HGDIOBJ(bitmap.0)) };
        if old.is_invalid() {
            // SAFETY: failed selection; no owned bitmap remains selected.
            unsafe {
                let _ = DeleteObject(HGDIOBJ(bitmap.0));
                let _ = DeleteDC(dc);
            }
            anyhow::bail!("Could not select portal bitmap");
        }
        Ok(Self {
            dc,
            bitmap,
            old,
            bits,
            side,
        })
    }
    fn pixels(&mut self) -> &mut [u32] {
        // SAFETY: the owned 32-bit DIB contains exactly side*side pixels,
        // with no row padding. Exclusive borrow prevents concurrent writes.
        unsafe { std::slice::from_raw_parts_mut(self.bits.as_ptr(), self.side * self.side) }
    }
}
impl Drop for Surface {
    fn drop(&mut self) {
        // SAFETY: restore original selection before deleting the owned
        // bitmap, then destroy the owned memory DC. No borrowed handles freed.
        unsafe {
            let _ = SelectObject(self.dc, self.old);
            let _ = DeleteObject(HGDIOBJ(self.bitmap.0));
            let _ = DeleteDC(self.dc);
        }
    }
}

pub struct EntryPortal {
    hwnd: HWND,
    surface: Surface,
    style: PortalStyle,
    start: Option<(Instant, (i32, i32))>,
    last_frame: Option<Instant>,
}
impl EntryPortal {
    pub fn new(style: PortalStyle) -> Result<Self> {
        let _dpi = PhysicalDpi::enter()?;
        let surface = Surface::new(style.radius() * 2 + 1)?;
        // SAFETY: built-in STATIC class, owned popup; deliberately hidden,
        // layered, click-through, toolwindow, no-activate. No hooks or owner changes.
        let hwnd = unsafe {
            CreateWindowExW(
                WS_EX_LAYERED
                    | WS_EX_TRANSPARENT
                    | WS_EX_NOACTIVATE
                    | WS_EX_TOOLWINDOW
                    | WS_EX_TOPMOST,
                w!("STATIC"),
                w!("IntraLink arrival cue"),
                WS_POPUP,
                0,
                0,
                surface.side as i32,
                surface.side as i32,
                None,
                None,
                None,
                None,
            )
        }?;
        Ok(Self {
            hwnd,
            surface,
            style,
            start: None,
            last_frame: None,
        })
    }

    /// Coordinates are the approved destination move projected into physical
    /// pixels. Never warp or hide the actual Windows cursor.
    pub fn begin(&mut self, screen: PhysicalScreen, x: u16, y: u16, now: Instant) -> Result<()> {
        self.begin_at(screen, screen.from_normalized(x, y), now)
    }

    pub fn begin_at(
        &mut self,
        screen: PhysicalScreen,
        center: (i32, i32),
        now: Instant,
    ) -> Result<()> {
        ensure!(
            screen.offset(center).is_some(),
            "Portal center outside approved physical monitor"
        );
        self.hide();
        self.start = Some((now, center));
        self.last_frame = None;
        Ok(())
    }

    pub fn poll(&mut self, now: Instant) -> Result<()> {
        let Some((start, center)) = self.start else {
            return Ok(());
        };
        let elapsed = now.saturating_duration_since(start);
        if elapsed >= DURATION {
            self.hide();
            return Ok(());
        }
        if self
            .last_frame
            .is_some_and(|last| now.saturating_duration_since(last) < FRAME)
        {
            return Ok(());
        }
        let show = self.last_frame.is_none();
        let _dpi = PhysicalDpi::enter()?;
        let progress = elapsed.as_secs_f32() / DURATION.as_secs_f32();
        raster(self.surface.pixels(), self.style, progress);
        let radius = self.style.radius() as i32;
        let position = POINT {
            x: center.0.saturating_sub(radius),
            y: center.1.saturating_sub(radius),
        };
        let size = SIZE {
            cx: self.surface.side as i32,
            cy: self.surface.side as i32,
        };
        let zero = POINT::default();
        let blend = BLENDFUNCTION {
            BlendOp: AC_SRC_OVER as u8,
            BlendFlags: 0,
            SourceConstantAlpha: 255,
            AlphaFormat: AC_SRC_ALPHA as u8,
        };
        // SAFETY: all arguments describe the live owned memory DIB and popup.
        // UpdateLayeredWindow copies premultiplied pixels, never reads desktop content.
        unsafe {
            UpdateLayeredWindow(
                self.hwnd,
                None,
                Some(&position),
                Some(&size),
                Some(self.surface.dc),
                Some(&zero),
                COLORREF(0),
                Some(&blend),
                ULW_ALPHA,
            )
        }?;
        if show {
            // SAFETY: reveal only this owned small popup without activation.
            unsafe {
                SetWindowPos(
                    self.hwnd,
                    Some(HWND_TOPMOST),
                    position.x,
                    position.y,
                    size.cx,
                    size.cy,
                    SWP_NOACTIVATE | SWP_SHOWWINDOW,
                )
            }?;
        }
        self.last_frame = Some(now);
        Ok(())
    }

    pub fn hide(&mut self) {
        if self.start.take().is_some() {
            // SAFETY: hides this owned overlay; never changes foreground ownership.
            let _ = unsafe { ShowWindow(self.hwnd, SW_HIDE) };
        }
        self.last_frame = None;
    }
}
impl Drop for EntryPortal {
    fn drop(&mut self) {
        self.hide();
        // SAFETY: destroy the owned popup on the same UI thread. DIB is
        // released separately after the popup has been destroyed.
        if let Err(error) = unsafe { DestroyWindow(self.hwnd) } {
            eprintln!("Arrival cue cleanup: {error}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn raster_is_small_premultiplied_transparent_and_fades() {
        for style in [PortalStyle::Full, PortalStyle::Reduced] {
            let side = style.radius() * 2 + 1;
            let mut pixels = vec![0; side * side];
            for p in [0.0, 0.25, 0.5, 0.75, 1.0] {
                raster(&mut pixels, style, p);
                assert_eq!(pixels[0], 0);
                assert_eq!(pixels[side * style.radius() + style.radius()], 0);
                assert!(pixels.iter().all(|v| {
                    let a = v >> 24;
                    [v & 255, (v >> 8) & 255, (v >> 16) & 255]
                        .into_iter()
                        .all(|c| c <= a)
                }));
                assert_eq!(pixels.iter().any(|v| *v != 0), p > 0.0 && p < 1.0);
            }
        }
    }
    #[test]
    #[ignore = "requires local graphical Windows desktop; generated cue only"]
    fn native_portal_updates_without_activation_and_hides_at_expiry() -> Result<()> {
        use windows::Win32::UI::WindowsAndMessaging::{
            GWL_EXSTYLE, GetForegroundWindow, GetWindowLongW, IsWindow, IsWindowVisible,
        };
        let screen = crate::physical_primary()?;
        for style in [PortalStyle::Full, PortalStyle::Reduced] {
            let mut portal = EntryPortal::new(style)?;
            let hwnd = portal.hwnd;
            // SAFETY: read foreground and styles of our own live window.
            let foreground = unsafe { GetForegroundWindow() };
            // SAFETY: read extended styles of the owned live popup.
            let flags = unsafe { GetWindowLongW(hwnd, GWL_EXSTYLE) } as u32;
            assert_eq!(
                flags & (WS_EX_LAYERED | WS_EX_TRANSPARENT | WS_EX_NOACTIVATE).0,
                (WS_EX_LAYERED | WS_EX_TRANSPARENT | WS_EX_NOACTIVATE).0
            );
            let now = Instant::now();
            portal.begin(screen, 32768, 32768, now)?;
            portal.poll(now + Duration::from_millis(50))?;
            // SAFETY: read only visibility/foreground of live handles.
            assert!(unsafe { IsWindowVisible(hwnd) }.as_bool());
            // SAFETY: query foreground window handle without mutating it.
            assert_eq!(unsafe { GetForegroundWindow() }, foreground);
            portal.poll(now + DURATION)?;
            // SAFETY: query visibility of the live owned popup.
            assert!(!unsafe { IsWindowVisible(hwnd) }.as_bool());
            drop(portal);
            // SAFETY: IsWindow accepts stale handle values and reports validity.
            assert!(!unsafe { IsWindow(Some(hwnd)) }.as_bool());
        }
        println!("NATIVE_ARRIVAL_PORTAL_PASS full/reduced; no focus or cursor changes");
        Ok(())
    }
}
