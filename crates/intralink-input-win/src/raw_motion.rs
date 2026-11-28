//! Foreground-only Raw Input mouse motion. No keyboard registration, global
//! hook, background input sink, persistent device identifier or cursor clip.
//! The existing visible controller surface must own the foreground.
use crate::foreground_capture::ForegroundCapture;
use anyhow::{Result, ensure};
use std::{
    cell::{Cell, RefCell},
    marker::PhantomData,
    rc::Rc,
};
use windows::Win32::{
    Foundation::{HWND, LPARAM, LRESULT, WPARAM},
    UI::{
        Input::{
            GetRawInputData, GetRegisteredRawInputDevices, HRAWINPUT, MOUSE_MOVE_NOCOALESCE,
            RAWINPUT, RAWINPUTDEVICE, RAWINPUTDEVICE_FLAGS, RAWINPUTHEADER, RAWMOUSE, RID_INPUT,
            RIDEV_DEVNOTIFY, RIDEV_REMOVE, RIM_TYPEMOUSE, RegisterRawInputDevices,
        },
        Shell::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass},
        WindowsAndMessaging::{
            GetForegroundWindow, IsWindow, WM_ACTIVATEAPP, WM_INPUT, WM_INPUT_DEVICE_CHANGE,
            WM_KILLFOCUS, WM_NCDESTROY,
        },
    },
};

const CAPACITY: usize = 128;
const MAX_COUNT: i32 = 4096;
const SUBCLASS: usize = 0x494c4d;
const MOUSE_PAGE: u16 = 1;
const MOUSE_USAGE: u16 = 2;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Delta {
    pub x: i32,
    pub y: i32,
}

pub struct Batch {
    samples: [Delta; CAPACITY],
    len: usize,
    button_activity: bool,
}
impl Default for Batch {
    fn default() -> Self {
        Self {
            samples: [Delta { x: 0, y: 0 }; CAPACITY],
            len: 0,
            button_activity: false,
        }
    }
}
impl Batch {
    pub fn samples(&self) -> &[Delta] {
        &self.samples[..self.len]
    }
    pub fn button_activity(&self) -> bool {
        self.button_activity
    }
    fn append(&mut self, mouse: RAWMOUSE) -> bool {
        // No absolute/virtual-desktop, changed attributes or unknown flags.
        if mouse.usFlags.0 & !MOUSE_MOVE_NOCOALESCE.0 != 0
            || mouse.lLastX.unsigned_abs() > MAX_COUNT as u32
            || mouse.lLastY.unsigned_abs() > MAX_COUNT as u32
        {
            return false;
        }
        // SAFETY: RAWMOUSE is fully initialized by Windows (or a test fixture).
        // The union alternatives overlap initialized u32 storage.
        let buttons = unsafe { mouse.Anonymous.Anonymous.usButtonFlags };
        self.button_activity |= buttons != 0;
        if mouse.lLastX == 0 && mouse.lLastY == 0 {
            return true;
        }
        if self.len == CAPACITY {
            return false;
        }
        self.samples[self.len] = Delta {
            x: mouse.lLastX,
            y: mouse.lLastY,
        };
        self.len += 1;
        true
    }
}

struct Receiver {
    hwnd: usize,
    batch: Batch,
}

thread_local! {
    static RECEIVER:RefCell<Option<Receiver>>=const{RefCell::new(None)};
    static FAULT:Cell<bool>=const{Cell::new(false)};
}

fn fault() {
    let _ = FAULT.try_with(|f| f.set(true));
}

fn read_mouse(lparam: LPARAM) -> Option<RAWMOUSE> {
    let handle = HRAWINPUT(lparam.0 as *mut core::ffi::c_void);
    if handle.is_invalid() {
        return None;
    }
    let mut packet = RAWINPUT::default();
    let mut bytes = std::mem::size_of::<RAWINPUT>() as u32;
    // SAFETY: Windows writes at most the declared aligned stack buffer.
    // We accept only the exact fixed-size mouse record; no HID allocation.
    let copied = unsafe {
        GetRawInputData(
            handle,
            RID_INPUT,
            Some((&mut packet as *mut RAWINPUT).cast()),
            &mut bytes,
            std::mem::size_of::<RAWINPUTHEADER>() as u32,
        )
    };
    if copied != std::mem::size_of::<RAWINPUT>() as u32
        || bytes != copied
        || packet.header.dwSize != copied
        || packet.header.dwType != RIM_TYPEMOUSE.0
        || packet.header.hDevice.is_invalid()
        || packet.header.wParam.0 & 0xff != 0
    {
        return None;
    }
    // SAFETY: the complete record was read and the union tag is mouse.
    Some(unsafe { packet.data.mouse })
}

unsafe extern "system" fn subclass(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    id: usize,
    _reference: usize,
) -> LRESULT {
    if id == SUBCLASS {
        let result = RECEIVER.try_with(|cell| {
            let Ok(mut receiver) = cell.try_borrow_mut() else {
                fault();
                return;
            };
            let Some(receiver) = receiver.as_mut().filter(|r| r.hwnd == hwnd.0 as usize) else {
                return;
            };
            if matches!(msg, WM_KILLFOCUS | WM_NCDESTROY | WM_INPUT_DEVICE_CHANGE)
                || (msg == WM_ACTIVATEAPP && wparam.0 == 0)
            {
                fault();
                return;
            }
            if msg == WM_INPUT {
                // SAFETY: reads current foreground HWND; no focus change.
                let foreground = unsafe { GetForegroundWindow() };
                if foreground != hwnd
                    || wparam.0 & 0xff != 0
                    || FAULT.try_with(Cell::get).unwrap_or(true)
                {
                    fault();
                    return;
                }
                match read_mouse(lparam) {
                    Some(mouse) if receiver.batch.append(mouse) => {}
                    _ => fault(),
                }
            }
        });
        if result.is_err() {
            fault();
        }
    }
    // SAFETY: continue the owned window's subclass chain for every message,
    // including foreground WM_INPUT cleanup. No raw data is read afterwards.
    unsafe { DefSubclassProc(hwnd, msg, wparam, lparam) }
}

fn registrations() -> Result<([RAWINPUTDEVICE; 16], usize)> {
    let mut devices = [RAWINPUTDEVICE::default(); 16];
    let mut count = devices.len() as u32;
    // SAFETY: bounded aligned output array with exact element count/size.
    let written = unsafe {
        GetRegisteredRawInputDevices(
            Some(devices.as_mut_ptr()),
            &mut count,
            std::mem::size_of::<RAWINPUTDEVICE>() as u32,
        )
    };
    ensure!(
        written != u32::MAX && written <= devices.len() as u32,
        "Raw Input registration query failed or exceeded bound"
    );
    Ok((devices, written as usize))
}
fn register(hwnd: HWND, flags: RAWINPUTDEVICE_FLAGS) -> Result<()> {
    let request = RAWINPUTDEVICE {
        usUsagePage: MOUSE_PAGE,
        usUsage: MOUSE_USAGE,
        dwFlags: flags,
        hwndTarget: hwnd,
    };
    // SAFETY: one initialized mouse registration, no keyboard or sink flags.
    unsafe { RegisterRawInputDevices(&[request], std::mem::size_of::<RAWINPUTDEVICE>() as u32) }?;
    Ok(())
}

pub struct RawMotion {
    hwnd: HWND,
    installed: bool,
    registered: bool,
    _thread: PhantomData<Rc<()>>,
}
impl RawMotion {
    /// Called only after local foreground consent; never during LOCAL mode.
    pub fn open(capture: &ForegroundCapture) -> Result<Self> {
        ensure!(
            capture.owns_current_pointer(),
            "Raw Input requires the owned foreground surface"
        );
        let hwnd = HWND(capture.handle() as *mut core::ffi::c_void);
        ensure!(
            registrations()?.1 == 0,
            "Existing process Raw Input registration; refusing to override it"
        );
        RECEIVER.with(|cell| -> Result<()> {
            let mut receiver = cell.try_borrow_mut()?;
            ensure!(receiver.is_none(), "Raw Input receiver already active");
            *receiver = Some(Receiver {
                hwnd: hwnd.0 as usize,
                batch: Batch::default(),
            });
            Ok(())
        })?;
        FAULT.with(|f| f.set(false));
        let mut owner = Self {
            hwnd,
            installed: false,
            registered: false,
            _thread: PhantomData,
        };
        // SAFETY: capture proves this thread owns the foreground HWND; the
        // callback is static and its TLS stores no borrowed pointers.
        ensure!(
            // SAFETY: install static callback on the proven same-thread owned HWND.
            unsafe { SetWindowSubclass(hwnd, Some(subclass), SUBCLASS, 0) }.as_bool(),
            "Could not install foreground Raw Input handler"
        );
        owner.installed = true;
        // Keep legacy mouse messages for the existing focused button/wheel path.
        register(hwnd, RIDEV_DEVNOTIFY)?;
        owner.registered = true;
        ensure!(
            owner.owns_registration()? && capture.owns_current_pointer(),
            "Raw Input admission changed"
        );
        Ok(owner)
    }
    fn owns_registration(&self) -> Result<bool> {
        let (devices, count) = registrations()?;
        Ok(devices[..count].iter().any(|r| {
            r.usUsagePage == MOUSE_PAGE
                && r.usUsage == MOUSE_USAGE
                && r.hwndTarget == self.hwnd
                && r.dwFlags.0 & !RIDEV_DEVNOTIFY.0 == 0
        }))
    }
    pub fn healthy(&self) -> bool {
        self.registered && !FAULT.with(Cell::get)
    }
    pub fn take(&self) -> Result<Batch> {
        ensure!(
            self.healthy() && self.owns_registration()?,
            "Raw Input ownership lost, invalid packet, device change or queue overflow"
        );
        // SAFETY: a read-only foreground query.
        let foreground = unsafe { GetForegroundWindow() };
        ensure!(foreground == self.hwnd, "Raw Input foreground lost");
        RECEIVER.with(|cell| {
            let mut receiver = cell.try_borrow_mut()?;
            let receiver = receiver
                .as_mut()
                .filter(|r| r.hwnd == self.hwnd.0 as usize)
                .ok_or_else(|| anyhow::anyhow!("Missing Raw Input receiver"))?;
            Ok(std::mem::take(&mut receiver.batch))
        })
    }
    pub fn discard(&self) -> Result<()> {
        self.take().map(drop)
    }

    fn cleanup(&mut self) -> Result<()> {
        // Remove memory state first: even a late/reentrant callback is inert.
        RECEIVER.with(|cell| {
            if let Ok(mut receiver) = cell.try_borrow_mut()
                && receiver
                    .as_ref()
                    .is_some_and(|r| r.hwnd == self.hwnd.0 as usize)
            {
                *receiver = None;
            }
        });
        fault();
        let removal = if self.registered {
            // Never remove a registration replaced by another component.
            match self.owns_registration() {
                Ok(true) => register(HWND::default(), RIDEV_REMOVE),
                Ok(false) => Ok(()),
                Err(e) => Err(e),
            }
        } else {
            Ok(())
        };
        if removal.is_ok() {
            self.registered = false;
        }
        let subclass_removed = if self.installed {
            // SAFETY: read validity of our handle; remove exactly our own
            // static callback and ID on the creating thread.
            unsafe {
                !IsWindow(Some(self.hwnd)).as_bool()
                    || RemoveWindowSubclass(self.hwnd, Some(subclass), SUBCLASS).as_bool()
            }
        } else {
            true
        };
        if subclass_removed {
            self.installed = false;
        }
        removal?;
        ensure!(subclass_removed, "Raw Input handler cleanup failed");
        Ok(())
    }
    pub fn close(mut self) -> Result<()> {
        self.cleanup()
    }
}
impl Drop for RawMotion {
    fn drop(&mut self) {
        if let Err(error) = self.cleanup() {
            eprintln!("Foreground Raw Input cleanup failed: {error:#}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows::Win32::UI::Input::{MOUSE_MOVE_ABSOLUTE, MOUSE_STATE, RAWMOUSE_0, RAWMOUSE_0_0};
    fn packet(x: i32, y: i32, flags: MOUSE_STATE, buttons: u16) -> RAWMOUSE {
        RAWMOUSE {
            usFlags: flags,
            lLastX: x,
            lLastY: y,
            Anonymous: RAWMOUSE_0 {
                Anonymous: RAWMOUSE_0_0 {
                    usButtonFlags: buttons,
                    usButtonData: 0,
                },
            },
            ..Default::default()
        }
    }
    #[test]
    fn ordered_bounded_motion_and_button_activity() {
        let mut batch = Batch::default();
        assert!(batch.append(packet(3, -5, MOUSE_STATE(0), 0)));
        assert!(batch.append(packet(-3, 5, MOUSE_MOVE_NOCOALESCE, 0)));
        assert_eq!(
            batch.samples(),
            &[Delta { x: 3, y: -5 }, Delta { x: -3, y: 5 }]
        );
        assert!(!batch.button_activity());
        assert!(batch.append(packet(0, 0, MOUSE_STATE(0), 1)));
        assert!(batch.button_activity());
        for _ in 2..CAPACITY {
            assert!(batch.append(packet(1, 0, MOUSE_STATE(0), 0)));
        }
        assert!(!batch.append(packet(1, 0, MOUSE_STATE(0), 0)));
        assert_eq!(batch.samples().len(), CAPACITY);
    }
    #[test]
    fn unsupported_absolute_unknown_and_large_counts_are_rejected() {
        let mut batch = Batch::default();
        for mouse in [
            packet(1, 2, MOUSE_MOVE_ABSOLUTE, 0),
            packet(1, 2, MOUSE_STATE(0x8000), 0),
            packet(i32::MIN, 0, MOUSE_STATE(0), 0),
            packet(4097, 0, MOUSE_STATE(0), 0),
        ] {
            assert!(!batch.append(mouse));
        }
        assert!(batch.samples().is_empty());
    }
    #[test]
    #[ignore = "native foreground-only registration lifecycle on a hidden generated window; no input read"]
    fn native_raw_registration_lifecycle_is_scoped_and_removed() -> Result<()> {
        use windows::{
            Win32::UI::WindowsAndMessaging::{
                CreateWindowExW, DestroyWindow, WINDOW_EX_STYLE, WS_POPUP,
            },
            core::w,
        };
        struct Hidden(HWND);
        impl Drop for Hidden {
            fn drop(&mut self) {
                // SAFETY: test owns this hidden window on the creating thread.
                let _ = unsafe { DestroyWindow(self.0) };
            }
        }
        assert_eq!(registrations()?.1, 0);
        // SAFETY: built-in hidden STATIC popup, no WS_VISIBLE, no focus request.
        let window = Hidden(unsafe {
            CreateWindowExW(
                WINDOW_EX_STYLE(0),
                w!("STATIC"),
                w!("IntraLink registration fixture"),
                WS_POPUP,
                0,
                0,
                1,
                1,
                None,
                None,
                None,
                None,
            )
        }?);
        // SAFETY: read-only foreground check; the hidden window is never active.
        assert_ne!(unsafe { GetForegroundWindow() }, window.0);
        let mut owner = RawMotion {
            hwnd: window.0,
            installed: false,
            registered: false,
            _thread: PhantomData,
        };
        // SAFETY: static callback on our own same-thread hidden HWND; TLS
        // receiver is absent, so even a dispatched message cannot read input.
        let installed =
            unsafe { SetWindowSubclass(window.0, Some(subclass), SUBCLASS, 0) }.as_bool();
        ensure!(installed, "Test subclass could not be installed");
        owner.installed = true;
        register(window.0, RIDEV_DEVNOTIFY)?;
        owner.registered = true;
        let (devices, count) = registrations()?;
        assert_eq!(count, 1);
        assert_eq!(devices[0].usUsagePage, MOUSE_PAGE);
        assert_eq!(devices[0].usUsage, MOUSE_USAGE);
        assert_eq!(devices[0].hwndTarget, window.0);
        assert_eq!(devices[0].dwFlags.0 & !RIDEV_DEVNOTIFY.0, 0);
        assert!(owner.owns_registration()?);
        owner.close()?;
        assert_eq!(registrations()?.1, 0);
        // SAFETY: the fixture never became a foreground input receiver.
        assert_ne!(unsafe { GetForegroundWindow() }, window.0);
        println!(
            "NATIVE_RAW_LIFECYCLE_PASS mouse-only foreground registration removed; no input read"
        );
        Ok(())
    }

    #[test]
    #[ignore = "native read-only registration query; does not register or collect input"]
    fn native_raw_registration_query_is_empty_without_installing_reader() -> Result<()> {
        assert_eq!(registrations()?.1, 0);
        println!("NATIVE_RAW_REGISTRY_PASS no registration installed");
        Ok(())
    }
}
