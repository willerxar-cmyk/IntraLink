use super::*;
#[test]
fn desktop_key_policy_does_not_forward_system_or_stop_keys() {
    for key in [
        Key::F12,
        Key::Escape,
        Key::LeftAlt,
        Key::RightAlt,
        Key::LeftSuper,
        Key::RightSuper,
        Key::CapsLock,
        Key::NumLock,
        Key::ScrollLock,
    ] {
        assert!(keys::desktop_key(key).is_none());
    }
    assert_eq!(keys::desktop_key(Key::F1).unwrap().usage(), 58);
    assert_eq!(keys::desktop_key(Key::F9).unwrap().usage(), 66);
    assert_eq!(keys::desktop_key(Key::A).unwrap().usage(), 4);
}

#[test]
#[ignore = "requires local native desktop; small generated window, no input or warp"]
fn native_foreground_source_verifies_only_its_owned_window_geometry() -> Result<()> {
    let primary = physical_primary()?;
    let (width, height) = primary.size();
    ensure!(
        width >= 800 && height >= 500,
        "Native test needs a sufficiently large monitor"
    );
    let (x, y) = primary.origin();
    let mut canvas = vec![0; WIDTH * HEIGHT];
    let mut w = with_physical_dpi(|| {
        Window::new(
            "IntraLink | Native geometry test - NO INPUT",
            640,
            360,
            WindowOptions {
                borderless: true,
                none: true,
                title: false,
                topmost: true,
                resize: false,
                scale_mode: ScaleMode::AspectRatioStretch,
                ..WindowOptions::default()
            },
        )
        .map_err(|e| anyhow::anyhow!("{e}"))
    })?;
    w.set_target_fps(0);
    paint_capture(&mut canvas, "GENERATED WINDOW", false);
    w.update_with_buffer(&canvas, WIDTH, HEIGHT)
        .map_err(|e| anyhow::anyhow!("{e}"))?;
    let guard = ForegroundCapture::prepare_compact(
        w.get_window_handle() as usize,
        primary,
        (x + 360, y + 220),
    )?;
    w.update();
    assert!(guard.geometry_matches());
    let granted = guard.request_focus();
    w.update();
    if granted {
        ensure!(guard.ready(), "Granted test foreground disappeared");
    } else {
        assert!(!guard.ready());
    }
    drop(w);
    assert!(!guard.ready() && !guard.geometry_matches());
    // No calls to motion(), center(), return_at(), or input workers.
    println!(
        "NATIVE_COMPACT_GEOMETRY_PASS 640x360 owned window, foreground={granted}; no cursor movement or input"
    );
    Ok(())
}

#[test]
#[ignore = "requires native local graphical desktop; no capture or input"]
fn native_desktop_edge_surfaces_present_generated_status() -> Result<()> {
    let mut canvas = vec![0; WIDTH * HEIGHT];
    let mut w = indicator()?;
    paint_indicator(&mut canvas, false, 300, SourceMode::CompactRaw);
    w.update_with_buffer(&canvas, WIDTH, HEIGHT)
        .map_err(|e| anyhow::anyhow!("{e}"))?;
    ensure!(
        indicator_visible(w.get_window_handle() as usize),
        "Missing local status window"
    );
    paint_capture(&mut canvas, "GENERATED CLIENT", true);
    for _ in 0..8 {
        w.update_with_buffer(&canvas, WIDTH, HEIGHT)
            .map_err(|e| anyhow::anyhow!("{e}"))?;
        ensure!(w.is_open(), "Status window closed");
    }
    // Deliberately never call ForegroundCapture::prepare or cursor placement.
    drop(w);
    println!(
        "NATIVE_DESKTOP_EDGE_STATUS_PASS generated UI only; no capture, warp, listener or connection"
    );
    Ok(())
}
