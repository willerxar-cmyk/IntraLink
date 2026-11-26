//! Explicit D3D11 adapter selection for native video diagnostics.
use anyhow::{Context, Result, ensure};
use windows::{
    Win32::{
        Foundation::HMODULE,
        Graphics::{
            Direct3D::{D3D_DRIVER_TYPE_UNKNOWN, D3D_FEATURE_LEVEL_11_0, D3D_FEATURE_LEVEL_11_1},
            Direct3D11::*,
            Dxgi::{
                CreateDXGIFactory1, DXGI_ADAPTER_FLAG_SOFTWARE, DXGI_ERROR_NOT_FOUND, IDXGIAdapter,
                IDXGIAdapter1, IDXGIFactory1,
            },
        },
    },
    core::Interface,
};

fn enumerate_adapters() -> Result<Vec<(u32, IDXGIAdapter1)>> {
    // SAFETY: DXGI returns a live COM factory of the requested interface.
    let factory: IDXGIFactory1 = unsafe { CreateDXGIFactory1()? };
    let mut adapters = Vec::new();
    for index in 0..64 {
        // SAFETY: the factory is live; index is bounded.
        match unsafe { factory.EnumAdapters1(index) } {
            Ok(adapter) => adapters.push((index, adapter)),
            Err(error) if error.code() == DXGI_ERROR_NOT_FOUND => return Ok(adapters),
            Err(error) => return Err(error.into()),
        }
    }
    anyhow::bail!("DXGI adapter enumeration exceeded limit")
}

/// Diagnostic indices belong to the current DXGI enumeration, not a persistent
/// device identity. Print the full descriptor so ambiguous vendors are visible.
pub fn list_video_adapters() -> Result<()> {
    for (index, adapter) in enumerate_adapters()? {
        // SAFETY: live adapter interface; the returned descriptor owns its data.
        let desc = unsafe { adapter.GetDesc1()? };
        let name = String::from_utf16_lossy(&desc.Description);
        let name = name.trim_end_matches(char::from(0));
        println!(
            "adapter_index={index}, name={name}, vendor_id={:#06x}, device_id={:#06x}, luid={:08x}:{:08x}, flags={:#x}, dedicated_bytes={}",
            desc.VendorId,
            desc.DeviceId,
            desc.AdapterLuid.HighPart as u32,
            desc.AdapterLuid.LowPart,
            desc.Flags,
            desc.DedicatedVideoMemory
        );
    }
    Ok(())
}

/// Select the requested non-software DXGI adapter explicitly. A vendor-only
/// selection must be unique; an index must match the optional requested vendor.
pub(crate) fn create_video_device(
    vendor: &str,
    adapter_index: Option<u32>,
) -> Result<(ID3D11Device, ID3D11DeviceContext)> {
    let vendor_id = match vendor {
        "default" if adapter_index.is_none() => {
            return Ok(windows_capture::d3d11::create_d3d_device()?);
        }
        "default" => None,
        "intel" => Some(0x8086),
        "nvidia" => Some(0x10de),
        "amd" => Some(0x1002),
        _ => anyhow::bail!("Unknown adapter vendor: {vendor}"),
    };
    let mut selected: Option<IDXGIAdapter1> = None;
    for (index, adapter) in enumerate_adapters()? {
        if adapter_index.is_some_and(|requested| requested != index) {
            continue;
        }
        // SAFETY: the adapter interface is live and returns a descriptor.
        let desc = unsafe { adapter.GetDesc1()? };
        if vendor_id.is_none_or(|requested| requested == desc.VendorId)
            && desc.Flags & DXGI_ADAPTER_FLAG_SOFTWARE.0 as u32 == 0
        {
            ensure!(
                selected.is_none(),
                "Multiple DXGI adapters match {vendor}; use desktop-video-adapters and --adapter-index"
            );
            selected = Some(adapter);
        }
    }
    let selected: IDXGIAdapter = selected
        .context("Requested non-software adapter unavailable or vendor mismatch")?
        .cast()?;
    let mut device = None;
    let mut context = None;
    // SAFETY: explicit adapter, UNKNOWN driver type, valid feature levels
    // and live output storage. No software fallback or other device is used.
    unsafe {
        D3D11CreateDevice(
            &selected,
            D3D_DRIVER_TYPE_UNKNOWN,
            HMODULE::default(),
            D3D11_CREATE_DEVICE_BGRA_SUPPORT | D3D11_CREATE_DEVICE_VIDEO_SUPPORT,
            Some(&[D3D_FEATURE_LEVEL_11_1, D3D_FEATURE_LEVEL_11_0]),
            D3D11_SDK_VERSION,
            Some(&mut device),
            None,
            Some(&mut context),
        )?;
    }
    Ok((
        device.context("Missing selected D3D11 device")?,
        context.context("Missing selected immediate context")?,
    ))
}
