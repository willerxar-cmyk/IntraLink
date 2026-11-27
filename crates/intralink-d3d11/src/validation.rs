//! Local hardware diagnostic using synthetic texels and independent devices.
use anyhow::{Context, Result, ensure};
use windows::Win32::Graphics::{
    Direct3D11::{
        D3D11_SUBRESOURCE_DATA, D3D11_TEXTURE2D_DESC, D3D11_USAGE_DEFAULT, ID3D11Device,
        ID3D11Texture2D,
    },
    Dxgi::Common::{DXGI_FORMAT_R8G8B8A8_UNORM, DXGI_SAMPLE_DESC},
};
use windows_capture::d3d11::create_d3d_device;

use crate::Readback;

fn texture(device: &ID3D11Device, width: u32, bytes: &[u8]) -> Result<ID3D11Texture2D> {
    ensure!(
        bytes.len() == width as usize * 4 * 4,
        "Invalid fixture pixels"
    );
    let desc = D3D11_TEXTURE2D_DESC {
        Width: width,
        Height: 4,
        MipLevels: 1,
        ArraySize: 1,
        Format: DXGI_FORMAT_R8G8B8A8_UNORM,
        SampleDesc: DXGI_SAMPLE_DESC {
            Count: 1,
            Quality: 0,
        },
        Usage: D3D11_USAGE_DEFAULT,
        ..Default::default()
    };
    let data = D3D11_SUBRESOURCE_DATA {
        pSysMem: bytes.as_ptr().cast(),
        SysMemPitch: width * 4,
        SysMemSlicePitch: 0,
    };
    let mut texture = None;
    // SAFETY: the validated input contains every texel in one initialized
    // subresource; COM copies the data during the call.
    unsafe { device.CreateTexture2D(&desc, Some(&data), Some(&mut texture))? };
    texture.context("Missing diagnostic texture")
}

/// Run on actual hardware, not as a mandatory headless CI test. This checks
/// device identity rejection, not physical device removal or driver reset.
pub fn verify_native_boundary() -> Result<()> {
    let (device, context) = create_d3d_device()?;
    let (other_device, other_context) = create_d3d_device()?;
    ensure!(device != other_device, "Expected independent D3D11 devices");
    let bytes = [17u8; 64];
    let source = texture(&device, 4, &bytes)?;
    let foreign = texture(&other_device, 4, &bytes)?;
    let resized = texture(&device, 8, &[99u8; 128])?;
    let mut output = Vec::new();
    let mut readback = Readback::default();
    readback.copy_texture(&device, &context, &source, &mut output)?;
    ensure!(output == bytes, "Native readback mismatch");
    ensure!(
        readback
            .copy_texture(&device, &other_context, &source, &mut output)
            .is_err(),
        "Foreign context accepted"
    );
    ensure!(
        readback
            .copy_texture(&device, &context, &foreign, &mut output)
            .is_err(),
        "Foreign texture accepted"
    );
    ensure!(
        readback
            .copy_texture(&other_device, &other_context, &foreign, &mut output)
            .is_err(),
        "Staging device changed"
    );
    ensure!(
        readback
            .copy_texture(&device, &context, &resized, &mut output)
            .is_err(),
        "Staging geometry changed"
    );
    ensure!(
        output == bytes && readback.allocations() == 1,
        "Invalid source mutated readback state"
    );
    readback.copy_texture(&device, &context, &source, &mut output)?;
    ensure!(
        output == bytes && readback.allocations() == 1,
        "Valid source did not recover"
    );
    let distinct: [u8; 64] = std::array::from_fn(|index| (index * 37 + 11) as u8);
    let second = texture(&device, 4, &distinct)?;
    readback.begin_texture(&device, &context, &second)?;
    ensure!(readback.is_pending(), "Staged copy not retained");
    ensure!(
        readback.begin_texture(&device, &context, &source).is_err(),
        "Second pending copy accepted"
    );
    ensure!(
        readback.reset_output_staging().is_err(),
        "Pending staging reset accepted"
    );
    ensure!(
        readback
            .copy_texture(&device, &context, &source, &mut output)
            .is_err(),
        "Blocking copy replaced pending generation"
    );
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(1);
    loop {
        if readback.try_finish(&mut output)? {
            break;
        }
        ensure!(output == bytes, "Pending poll mutated output");
        ensure!(
            std::time::Instant::now() < deadline,
            "Staged native validation timed out"
        );
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    ensure!(
        !readback.is_pending() && output == distinct && readback.allocations() == 1,
        "Staged native pixels or allocation mismatch"
    );
    ensure!(
        readback.try_finish(&mut output).is_err(),
        "Completed staging consumed twice"
    );
    readback.copy_texture(&device, &context, &source, &mut output)?;
    ensure!(
        output == bytes && readback.allocations() == 1,
        "Blocking readback did not recover after staged completion"
    );
    readback.begin_texture(&device, &context, &second)?;
    drop(readback);
    let mut replacement = Readback::default();
    replacement.copy_texture(&device, &context, &source, &mut output)?;
    ensure!(
        output == bytes,
        "New readback after pending cancellation returned incorrect pixels"
    );
    println!(
        "Native staged readback: exact RGBA, single pending generation, reset rejection, reuse and cancellation passed"
    );
    Ok(())
}
