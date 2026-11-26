//! Vendor-neutral local D3D11 capability inventory. Driver configuration
//! advertisement is separate from verified conversion and native codec safety.
use crate::adapter::{create_video_device_on, enumerate_adapters};
use anyhow::{Result, ensure};
use windows::{
    Win32::Graphics::{
        Direct3D11::*,
        Dxgi::{Common::DXGI_FORMAT_NV12, DXGI_ADAPTER_FLAG_SOFTWARE, IDXGIDevice},
    },
    core::Interface,
};

fn decoder_profiles(device: &ID3D11Device) -> Result<()> {
    let video: ID3D11VideoDevice = device.cast()?;
    // SAFETY: retained live video interface; only query immutable capabilities.
    let count = unsafe { video.GetVideoDecoderProfileCount() };
    ensure!(
        count <= 256,
        "Decoder profile count exceeds diagnostic bound"
    );
    let mut profiles = Vec::with_capacity(count as usize);
    for index in 0..count {
        // SAFETY: index is within the driver's bounded advertised count.
        profiles.push(unsafe { video.GetVideoDecoderProfile(index)? });
    }
    for (name, guid) in [
        ("H264", D3D11_DECODER_PROFILE_H264_VLD_NOFGT),
        ("HEVC-main", D3D11_DECODER_PROFILE_HEVC_VLD_MAIN),
        ("VP9-profile0", D3D11_DECODER_PROFILE_VP9_VLD_PROFILE0),
        ("AV1-profile0", D3D11_DECODER_PROFILE_AV1_VLD_PROFILE0),
    ] {
        if !profiles.contains(&guid) {
            println!("decoder={name}, advertised=false, remote_enabled=false");
            continue;
        }
        // SAFETY: known profile GUID and constant NV12 format, no allocations.
        let format = unsafe { video.CheckVideoDecoderFormat(&guid, DXGI_FORMAT_NV12)? };
        if !format.as_bool() {
            println!("decoder={name}, advertised=true, nv12=false, remote_enabled=false");
            continue;
        }
        for height in [540u32, 720, 1080] {
            let desc = D3D11_VIDEO_DECODER_DESC {
                Guid: guid,
                SampleWidth: height * 16 / 9,
                SampleHeight: height,
                OutputFormat: DXGI_FORMAT_NV12,
            };
            // SAFETY: initialized descriptor and live video device. No
            // decoder, surface or bitstream is created by this query.
            match unsafe { video.GetVideoDecoderConfigCount(&desc) } {
                Ok(configs) => println!(
                    "decoder={name}, height={height}, nv12=true, driver_configs={configs}, frames_verified=0, remote_enabled=false"
                ),
                Err(error) => println!(
                    "decoder={name}, height={height}, config_query_failed={error:?}, remote_enabled=false"
                ),
            }
        }
    }
    Ok(())
}

pub fn probe_video_capabilities() -> Result<()> {
    let mut hardware = 0u32;
    let mut created = 0u32;
    let mut conversion_verified = 0u32;
    for (index, adapter) in enumerate_adapters()? {
        // SAFETY: the current enumerated adapter is alive.
        let desc = unsafe { adapter.GetDesc1()? };
        let name = String::from_utf16_lossy(&desc.Description)
            .trim_end_matches('\0')
            .to_owned();
        let software = desc.Flags & DXGI_ADAPTER_FLAG_SOFTWARE.0 as u32 != 0;
        println!(
            "adapter_index={index}, name={name:?}, vendor={:#06x}, device={:#06x}, luid={:08x}:{:08x}, software={software}",
            desc.VendorId,
            desc.DeviceId,
            desc.AdapterLuid.HighPart as u32,
            desc.AdapterLuid.LowPart
        );
        if software {
            continue;
        }
        hardware += 1;
        let (device, context) = match create_video_device_on(&adapter) {
            Ok(pair) => pair,
            Err(error) => {
                println!("device_creation_failed={error:#}");
                continue;
            }
        };
        let dxgi: IDXGIDevice = device.cast()?;
        // SAFETY: query only the actual created device's adapter descriptor.
        let actual = unsafe { dxgi.GetAdapter()?.GetDesc()? };
        ensure!(
            actual.AdapterLuid == desc.AdapterLuid,
            "Created device adapter LUID differs from inventory"
        );
        created += 1;
        if let Err(error) = decoder_profiles(&device) {
            println!("decoder_inventory_incomplete={error:#}");
        }
        match crate::nv12::verify_nv12_on(1080, 6, device, context) {
            Ok(()) => {
                conversion_verified += 1;
                println!("conversion_1080p=verified");
            }
            Err(error) => println!("conversion_1080p=failed, reason={error:#}"),
        }
    }
    println!(
        "hardware_adapters={hardware}, devices_created={created}, verified_nv12_conversion={conversion_verified}, remote_hardware_codecs_enabled=0"
    );
    // Missing or unsupported GPU is a reportable capability outcome, not a
    // requirement for the independent software desktop implementation.
    Ok(())
}
