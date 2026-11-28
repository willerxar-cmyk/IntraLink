use super::*;
use intralink_core::presentation::PresentationSurface;
use minifb::{ScaleMode, Window, WindowOptions};

fn display(w: u16, h: u16) -> Display {
    Display {
        width: w,
        height: h,
        scale: 100,
        rotation: 0,
    }
}
fn state() -> Chrome {
    Chrome {
        screen: Lane::Live,
        input: Lane::Live,
        state: InputState::Safe,
        verified: false,
        verify_pending: false,
        verify_allowed: true,
        arm_allowed: false,
        stale: false,
        display: Some(display(1080, 1920)),
    }
}

#[test]
fn displayed_video_content_excludes_both_letterboxes_and_maps_endpoints() {
    for (frame, d) in [
        ((960, 540), display(1080, 1920)),
        ((1920, 1080), display(1920, 1080)),
        ((1280, 720), display(1281, 1024)),
        ((960, 540), display(3440, 1440)),
    ] {
        let image = image_rect(frame).unwrap();
        let region = video_region(frame, d).unwrap();
        assert_eq!(normalized(region, (region.x, region.y)), Some((0, 0)));
        assert_eq!(
            normalized(region, (region.x + region.w - 1, region.y + region.h - 1)),
            Some((65535, 65535))
        );
        assert!(normalized(region, (region.x - 1, region.y)).is_none());
        assert!(normalized(region, (region.x, region.y - 1)).is_none());
        assert!(normalized(region, (region.x + region.w, region.y)).is_none());
        assert!(normalized(region, (region.x, region.y + region.h)).is_none());
        let (w, h) = fitted(
            (u32::from(d.width), u32::from(d.height)),
            (frame.0 as u32, frame.1 as u32),
        );
        let left = (frame.0 - w as usize) / 2;
        let top = (frame.1 - h as usize) / 2;
        for x in image.x..image.x + image.w {
            let sx = (x - image.x) * frame.0 / image.w;
            assert_eq!(
                (region.x..region.x + region.w).contains(&x),
                (left..left + w as usize).contains(&sx)
            );
        }
        for y in image.y..image.y + image.h {
            let sy = (y - image.y) * frame.1 / image.h;
            assert_eq!(
                (region.y..region.y + region.h).contains(&y),
                (top..top + h as usize).contains(&sy)
            );
        }
        assert!(region.y >= BODY_TOP * 2 && region.y + region.h <= BODY_BOTTOM * 2);
    }
    assert!(map_window((1280, 1000), (640.0, 100.0)).is_none());
    assert_eq!(map_window((1280, 720), (0.0, 0.0)), Some((0, 0)));
    assert_eq!(map_window((1280, 1000), (640.0, 500.0)), Some((960, 540)));
}

#[test]
fn local_controls_require_fresh_complete_clicks_and_arming_is_separate() {
    let mut chrome = state();
    let arm = Some((ARM.x * 2, ARM.y * 2));
    let verify = Some((VERIFY.x * 2, VERIFY.y * 2));
    assert_eq!(chrome.hit(arm), None);
    assert_eq!(chrome.hit(verify), Some(Action::Verify));
    chrome.verify_pending = true;
    assert_eq!(chrome.hit(arm), None);
    chrome.verified = true;
    chrome.arm_allowed = true;
    assert_eq!(chrome.hit(arm), Some(Action::Arm));
    assert_eq!(chrome.hit(Some((BODY.x, BODY.y))), None);
    let mut click = Click::default();
    assert_eq!(click.step(true, true, Some(Action::Arm)), None);
    assert_eq!(click.step(true, false, Some(Action::Arm)), None);
    assert_eq!(click.step(true, true, Some(Action::Arm)), None);
    assert_eq!(click.step(false, false, Some(Action::Arm)), None);
    assert_eq!(click.step(true, false, Some(Action::Arm)), None);
    assert_eq!(click.step(true, true, Some(Action::Arm)), None);
    assert_eq!(click.step(true, false, Some(Action::Verify)), None);
    assert_eq!(click.step(true, true, Some(Action::Arm)), None);
    assert_eq!(
        click.step(true, false, Some(Action::Arm)),
        Some(Action::Arm)
    );
}

#[test]
fn rendering_preserves_chrome_and_reuses_mapping_without_stale_screen_pixels() -> Result<()> {
    let mut surface = PresentationSurface::default();
    surface.update::<anyhow::Error>(1, (960, 540), |pixels| {
        pixels.resize(960 * 540, 0xff1234);
        Ok(())
    })?;
    let first = surface.snapshot_after(0).unwrap();
    let mut canvas = Canvas::new();
    canvas.chrome("Generated CLIENT", state());
    let toolbar = canvas.pixels[..BODY_TOP * 2 * WIDTH].to_vec();
    canvas.frame(&first)?;
    assert_eq!(canvas.pixels[..BODY_TOP * 2 * WIDTH], toolbar);
    let rect = image_rect(first.dimensions()).unwrap();
    assert_eq!(canvas.pixels[rect.y * WIDTH + rect.x], 0xff1234);
    let mapping = canvas.xmap.as_ptr();
    drop(first);
    surface.update::<anyhow::Error>(2, (960, 540), |pixels| {
        pixels.fill(0x3412ff);
        Ok(())
    })?;
    canvas.frame(&surface.snapshot_after(1).unwrap())?;
    assert_eq!(canvas.xmap.as_ptr(), mapping);
    canvas.placeholder(Lane::Ended, Some(display(1080, 1920)));
    assert!(
        canvas.pixels[BODY_TOP * 2 * WIDTH..BODY_BOTTOM * 2 * WIDTH]
            .iter()
            .all(|p| *p != 0xff1234 && *p != 0x3412ff)
    );
    assert!(canvas.image.is_none());
    Ok(())
}

#[test]
#[ignore = "requires local native desktop; generated media and UI only, no network or input"]
fn native_unified_window_presents_screen_and_input_only_states() -> Result<()> {
    let mut window = Window::new(
        "IntraLink | Generated unified preview",
        1280,
        720,
        WindowOptions {
            resize: true,
            scale_mode: ScaleMode::AspectRatioStretch,
            ..WindowOptions::default()
        },
    )
    .map_err(|e| anyhow::anyhow!("{e}"))?;
    window.set_target_fps(30);
    let mut canvas = Canvas::new();
    let mut chrome = state();
    let mut surface = PresentationSurface::default();
    surface.update::<anyhow::Error>(1, (960, 540), |pixels| {
        pixels.resize(960 * 540, 0);
        for (y, row) in pixels.chunks_exact_mut(960).enumerate() {
            for (x, p) in row.iter_mut().enumerate() {
                *p = ((x as u32 & 255) << 16) | ((y as u32 & 255) << 8) | 0x55;
            }
        }
        Ok(())
    })?;
    canvas.chrome("GENERATED CLIENT", chrome);
    canvas.frame(&surface.snapshot_after(0).unwrap())?;
    for _ in 0..8 {
        window
            .update_with_buffer(&canvas.pixels, WIDTH, HEIGHT)
            .map_err(|e| anyhow::anyhow!("{e}"))?;
        assert!(window.is_open());
    }
    chrome.screen = Lane::Ended;
    chrome.verified = false;
    chrome.verify_allowed = false;
    chrome.arm_allowed = true;
    canvas.chrome("GENERATED CLIENT", chrome);
    canvas.placeholder(Lane::Ended, chrome.display);
    for _ in 0..8 {
        window
            .update_with_buffer(&canvas.pixels, WIDTH, HEIGHT)
            .map_err(|e| anyhow::anyhow!("{e}"))?;
        assert!(window.is_open());
    }
    println!(
        "NATIVE_UNIFIED_CONTROL_PASS 16 generated 1920x1080 submissions; no capture, connection or input"
    );
    Ok(())
}
