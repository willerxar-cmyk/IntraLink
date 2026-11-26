//! Explicit-adapter WGC for an owned generated test window only.
//! The frame pool uses the selected D3D11 device directly. Native event
//! callbacks merely coalesce notifications; GPU access stays on the caller.
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver},
    },
    time::{Duration, Instant},
};

use crate::{LatestSurface, readback::texture_desc};
use anyhow::{Context, Result, ensure};
use windows::{
    Foundation::TypedEventHandler,
    Graphics::{
        Capture::{Direct3D11CaptureFramePool, GraphicsCaptureItem, GraphicsCaptureSession},
        DirectX::DirectXPixelFormat,
    },
    Win32::{
        Foundation::E_POINTER,
        Graphics::Direct3D11::{ID3D11Device, ID3D11DeviceContext, ID3D11Texture2D},
        System::WinRT::{
            Direct3D11::IDirect3DDxgiInterfaceAccess, RO_INIT_MULTITHREADED, RoInitialize,
            RoUninitialize,
        },
    },
    core::{IInspectable, Interface},
};
use windows_capture::{
    d3d11::create_direct3d_device, settings::GraphicsCaptureItemType, window::Window,
};

pub(crate) struct Apartment(std::marker::PhantomData<std::rc::Rc<()>>);
impl Apartment {
    pub(crate) fn new() -> Result<Self> {
        // SAFETY: initialize WinRT for this calling thread, balanced by Drop.
        unsafe { RoInitialize(RO_INIT_MULTITHREADED)? };
        Ok(Self(std::marker::PhantomData))
    }
}
impl Drop for Apartment {
    fn drop(&mut self) {
        // SAFETY: this owner was initialized on the same thread.
        unsafe { RoUninitialize() };
    }
}

pub(crate) struct SelectedCapture<'a> {
    item: GraphicsCaptureItem,
    pool: Direct3D11CaptureFramePool,
    session: GraphicsCaptureSession,
    frame_token: Option<i64>,
    close_token: Option<i64>,
    wake: Receiver<()>,
    closed: Arc<AtomicBool>,
    size: (u32, u32),
    latest: Option<LatestSurface>,
    timestamp: Option<i64>,
    device: ID3D11Device,
    context: ID3D11DeviceContext,
    stopped: bool,
    // The apartment must outlive every WinRT capture object.
    _apartment: &'a Apartment,
}

impl<'a> SelectedCapture<'a> {
    pub(crate) fn start(
        apartment: &'a Apartment,
        pid: u32,
        title: &str,
        device: &ID3D11Device,
        context: &ID3D11DeviceContext,
    ) -> Result<Self> {
        ensure!(GraphicsCaptureSession::IsSupported()?, "WGC unavailable");
        let mut candidates = Window::enumerate()?.into_iter().filter(|window| {
            window.process_id().ok() == Some(pid) && window.title().ok().as_deref() == Some(title)
        });
        let target = candidates
            .next()
            .context("Owned diagnostic window unavailable")?;
        ensure!(
            candidates.next().is_none() && target.is_valid(),
            "Ambiguous or invalid diagnostic window"
        );
        let item = match target.try_into()? {
            GraphicsCaptureItemType::Window((item, _)) => item,
            _ => anyhow::bail!("Diagnostic requires a window capture item"),
        };
        let size = item.Size()?;
        ensure!(
            size.Width > 0 && size.Height > 0,
            "Invalid WGC item geometry"
        );
        let dimensions = (size.Width as u32, size.Height as u32);
        crate::readback::layout(dimensions.0, dimensions.1)?;
        // SAFETY: check device/context identity before constructing the pool.
        let owner = unsafe { context.GetDevice()? };
        ensure!(&owner == device, "Selected WGC device/context mismatch");
        let direct = create_direct3d_device(device)?;
        let pool = Direct3D11CaptureFramePool::CreateFreeThreaded(
            &direct,
            DirectXPixelFormat::R8G8B8A8UIntNormalized,
            1,
            size,
        )?;
        let session = pool.CreateCaptureSession(&item)?;
        let (notify, wake) = mpsc::sync_channel(1);
        let closed = Arc::new(AtomicBool::new(false));
        let mut capture = Self {
            item,
            pool,
            session,
            frame_token: None,
            close_token: None,
            wake,
            closed,
            size: dimensions,
            latest: None,
            timestamp: None,
            device: device.clone(),
            context: context.clone(),
            stopped: false,
            _apartment: apartment,
        };
        let event_notify = notify.clone();
        capture.frame_token = Some(capture.pool.FrameArrived(&TypedEventHandler::<
            Direct3D11CaptureFramePool,
            IInspectable,
        >::new(move |_, _| {
            let _ = event_notify.try_send(());
            Ok(())
        }))?);
        let flag = capture.closed.clone();
        capture.close_token = Some(capture.item.Closed(&TypedEventHandler::<
            GraphicsCaptureItem,
            IInspectable,
        >::new(move |_, _| {
            flag.store(true, Ordering::Release);
            let _ = notify.try_send(());
            Ok(())
        }))?);
        // Preserve OS capture border and cursor defaults. No border bypass.
        capture.session.StartCapture()?;
        Ok(capture)
    }

    pub(crate) fn size(&self) -> (u32, u32) {
        self.size
    }

    pub(crate) fn next(&mut self, timeout: Duration) -> Result<&LatestSurface> {
        ensure!(
            !self.stopped && timeout <= Duration::from_secs(3),
            "Invalid WGC wait"
        );
        let started = Instant::now();
        loop {
            ensure!(!self.closed.load(Ordering::Acquire), "WGC item closed");
            let remaining = timeout.saturating_sub(started.elapsed());
            ensure!(!remaining.is_zero(), "No new WGC frame before deadline");
            self.wake
                .recv_timeout(remaining)
                .context("WGC frame wait failed")?;
            ensure!(!self.closed.load(Ordering::Acquire), "WGC item closed");
            let frame = match self.pool.TryGetNextFrame() {
                Ok(frame) => frame,
                // A coalesced notification may outlive the corresponding
                // frame. E_POINTER is the WinRT null-return case only.
                Err(error) if error.code() == E_POINTER => continue,
                Err(error) => return Err(error.into()),
            };
            let result = (|| {
                let content = frame.ContentSize()?;
                ensure!(
                    content.Width > 0
                        && content.Height > 0
                        && (content.Width as u32, content.Height as u32) == self.size,
                    "WGC geometry changed"
                );
                let time = frame.SystemRelativeTime()?.Duration;
                ensure!(
                    self.timestamp.is_none_or(|previous| time > previous),
                    "WGC timestamp not advancing"
                );
                let access: IDirect3DDxgiInterfaceAccess = frame.Surface()?.cast()?;
                // SAFETY: access refers to the live frame surface. The
                // returned texture must belong to the selected D3D11 device.
                let texture = unsafe { access.GetInterface::<ID3D11Texture2D>()? };
                let desc = texture_desc(&texture, &self.device)?;
                ensure!(
                    (desc.Width, desc.Height) == self.size,
                    "WGC frame texture geometry changed"
                );
                if let Some(latest) = &mut self.latest {
                    latest.update_texture(&self.device, &self.context, &texture)?;
                } else {
                    self.latest = Some(LatestSurface::from_texture(
                        &self.device,
                        &self.context,
                        &texture,
                    )?);
                }
                self.timestamp = Some(time);
                Ok::<(), anyhow::Error>(())
            })();
            let cleanup = frame.Close();
            result?;
            cleanup?;
            ensure!(!self.closed.load(Ordering::Acquire), "WGC item closed");
            return self.latest.as_ref().context("Missing retained WGC frame");
        }
    }

    pub(crate) fn close(&mut self) -> Result<()> {
        if self.stopped {
            return Ok(());
        }
        self.stopped = true;
        self.closed.store(true, Ordering::Release);
        let mut errors = Vec::new();
        if let Some(token) = self.frame_token.take()
            && let Err(error) = self.pool.RemoveFrameArrived(token)
        {
            errors.push(error.to_string());
        }
        if let Some(token) = self.close_token.take()
            && let Err(error) = self.item.RemoveClosed(token)
        {
            errors.push(error.to_string());
        }
        if let Err(error) = self.session.Close() {
            errors.push(error.to_string());
        }
        if let Err(error) = self.pool.Close() {
            errors.push(error.to_string());
        }
        self.latest.take();
        ensure!(errors.is_empty(), "WGC cleanup: {}", errors.join("; "));
        Ok(())
    }
}
impl Drop for SelectedCapture<'_> {
    fn drop(&mut self) {
        if let Err(error) = self.close() {
            eprintln!("{error:#}");
        }
    }
}
