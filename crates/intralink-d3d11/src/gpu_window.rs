//! Dedicated local diagnostic HWND. No input hooks or real desktop capture.
use anyhow::{Result, ensure};
use std::{marker::PhantomData, rc::Rc};
use windows::{
    Win32::{
        Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, RECT, WPARAM},
        System::LibraryLoader::GetModuleHandleW,
        UI::WindowsAndMessaging::*,
    },
    core::{PCWSTR, w},
};

const CLASS: PCWSTR = w!("IntraLinkGpuFixtureWindow");
const TITLE: PCWSTR = w!("IntraLink - local GPU decoded test");

unsafe extern "system" fn window_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if msg == WM_ERASEBKGND {
        return LRESULT(1);
    }
    // SAFETY: forward the exact parameters received for this window to the OS.
    unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
}

pub(super) struct PreviewWindow {
    handle: Option<HWND>,
    instance: HINSTANCE,
    registered: bool,
    _owner: PhantomData<Rc<()>>,
}
impl PreviewWindow {
    pub(super) fn new(size: (u32, u32)) -> Result<Self> {
        // SAFETY: obtain this process's module and register a private class.
        let module = unsafe { GetModuleHandleW(PCWSTR::null())? };
        let instance = HINSTANCE(module.0);
        let class = WNDCLASSW {
            lpfnWndProc: Some(window_proc),
            hInstance: instance,
            lpszClassName: CLASS,
            ..Default::default()
        };
        // SAFETY: class descriptor lives through registration, static class
        // name and callback remain valid until class unregistration.
        let atom = unsafe { RegisterClassW(&class) };
        ensure!(
            atom != 0,
            "Register local GPU preview window: {}",
            windows::core::Error::from_thread()
        );
        let mut window = Self {
            handle: None,
            instance,
            registered: true,
            _owner: PhantomData,
        };
        let style = WS_CAPTION | WS_SYSMENU | WS_MINIMIZEBOX | WS_VISIBLE;
        // Use a smaller visible client for large frames. DXGI stretches
        // the full-resolution GPU back buffer, not a CPU-rescaled image.
        let mut rect = RECT {
            left: 0,
            top: 0,
            right: (size.0.min(1280)) as i32,
            bottom: (size.1.min(720)) as i32,
        };
        // SAFETY: initialized client rect, valid class and module; no parent,
        // menu, external window handle or user-controlled callback.
        let handle = unsafe {
            AdjustWindowRectEx(&mut rect, style, false, WINDOW_EX_STYLE(0))?;
            CreateWindowExW(
                WINDOW_EX_STYLE(0),
                CLASS,
                TITLE,
                style,
                CW_USEDEFAULT,
                CW_USEDEFAULT,
                rect.right - rect.left,
                rect.bottom - rect.top,
                None,
                None,
                Some(instance),
                None,
            )?
        };
        window.handle = Some(handle);
        Ok(window)
    }
    pub(super) fn hwnd(&self) -> Result<HWND> {
        self.handle
            .ok_or_else(|| anyhow::anyhow!("Local preview window closed"))
    }

    pub(super) fn pump(&mut self) -> Result<()> {
        let hwnd = self.hwnd()?;
        // SAFETY: pump messages for this exact owned HWND on the owner thread;
        // Escape is handled only while the local preview window has focus.
        unsafe {
            let mut msg = MSG::default();
            for _ in 0..128 {
                if !PeekMessageW(&mut msg, Some(hwnd), 0, 0, PM_REMOVE).as_bool() {
                    break;
                }
                ensure!(
                    msg.message != WM_QUIT && !(msg.message == WM_KEYDOWN && msg.wParam.0 == 27),
                    "Local GPU preview stopped"
                );
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
            ensure!(IsWindow(Some(hwnd)).as_bool(), "Local GPU preview closed");
        }
        Ok(())
    }

    pub(super) fn close(&mut self) -> Result<()> {
        if let Some(hwnd) = self.handle.take() {
            // SAFETY: this HWND was created by this owner thread only.
            unsafe {
                if IsWindow(Some(hwnd)).as_bool() {
                    DestroyWindow(hwnd)?;
                }
            }
        }
        if self.registered {
            // SAFETY: no live window remains for our private class.
            unsafe {
                UnregisterClassW(CLASS, Some(self.instance))?;
            }
            self.registered = false;
        }
        Ok(())
    }
}
impl Drop for PreviewWindow {
    fn drop(&mut self) {
        if let Err(error) = self.close() {
            eprintln!("GPU preview cleanup: {error:#}");
        }
    }
}
