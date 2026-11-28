use super::*;
use iroh::SecretKey;
#[test]
#[ignore = "requires local native graphical desktop; generated peer only"]
fn native_topology_editor_observes_base_and_presents_generated_layout() -> Result<()> {
    let observed = topology_store::observe_base()?;
    let dir = tempfile::tempdir()?;
    let id = |n: u8| SecretKey::from_bytes(&[n; 32]).public();
    let store = Store::new(&dir.path().join("test"), id(1), vec![id(2)], observed)?;
    let mut editor = Editor::fresh(&store);
    let mut canvas = vec![0; WIDTH * HEIGHT];
    let mut window = super::super::window(
        "IntraLink | Generated geometry test",
        WIDTH,
        HEIGHT,
        false,
        ui::BG,
    )?;
    window.set_target_fps(30);
    for confirmed in [false, true] {
        if confirmed {
            // Generated portrait CLIENT fixture; only BASE is observed.
            editor.fields[0] = "1080".into();
            editor.fields[1] = "1920".into();
            editor.fields[2] = "125".into();
            editor.fields[3] = "90".into();
            editor.fields[4] = "-864".into();
            editor.fields[5] = "0".into();
            editor.apply()?;
            assert!(editor.save_ready());
            store.save(&editor.config)?;
            assert!(store.load()?.is_some());
        }
        let topology =
            display_geometry::topology(&editor.config.placements).context("Test topology")?;
        let projection = Projection::new(&topology, EDIT_AREA).context("Test projection")?;
        editor.render(
            &mut canvas,
            &["BASE", "GENERATED CLIENT"],
            &topology,
            projection,
        );
        for _ in 0..8 {
            window
                .update_with_buffer(&canvas, WIDTH, HEIGHT)
                .map_err(|e| anyhow::anyhow!("{e}"))?;
            assert!(window.is_open());
        }
    }
    println!(
        "NATIVE_TOPOLOGY_PASS base={}x{} scale={} rotation={} submissions=16; generated CLIENT only",
        observed.display.width,
        observed.display.height,
        observed.display.scale,
        observed.display.rotation
    );
    Ok(())
}

#[test]
fn placeholders_cannot_be_saved_and_edits_are_atomic() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let id = |n: u8| SecretKey::from_bytes(&[n; 32]).public();
    let store = Store::new(
        &dir.path().join("test"),
        id(1),
        vec![id(2)],
        topology_store::Observed {
            monitor: 1,
            display: Display {
                width: 1920,
                height: 1080,
                scale: 100,
                rotation: 0,
            },
        },
    )?;
    let mut e = Editor::fresh(&store);
    assert!(!e.save_ready());
    e.fields[0] = "1080".into();
    e.fields[1] = "1920".into();
    e.fields[2] = "125".into();
    e.fields[3] = "90".into();
    e.fields[4] = "-864".into();
    e.fields[5] = "100".into();
    e.apply()?;
    assert!(e.save_ready());
    let previous = e.config.placements[1];
    e.fields[4] = "0".into();
    assert!(e.apply().is_err());
    assert_eq!(e.config.placements[1], previous);
    e.select(1);
    e.editing = Some(0);
    e.replace = true;
    e.key(Key::Key1);
    assert!(!e.save_ready());
    e.select(0);
    assert!(e.apply().is_err());
    assert!(store.load()?.is_none());
    Ok(())
}

#[test]
fn authenticated_dimensions_are_read_only_and_positions_remain_editable() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let id = |n: u8| SecretKey::from_bytes(&[n; 32]).public();
    let observed = topology_store::Observed {
        monitor: 1,
        display: Display {
            width: 1920,
            height: 1080,
            scale: 100,
            rotation: 0,
        },
    };
    let client = Display {
        width: 1080,
        height: 1920,
        scale: 125,
        rotation: 90,
    };
    let store = Store::new(&dir.path().join("test"), id(1), vec![id(2)], observed)?
        .with_displays(vec![client])?;
    let mut editor = Editor::fresh(&store);
    assert!(editor.authenticated && editor.save_ready());
    let t = display_geometry::topology(&editor.config.placements).context("Test topology")?;
    let view = Projection::new(&t, EDIT_AREA).context("Test projection")?;
    assert_eq!(editor.hit(view, &t, (field(0).x, field(0).y)), None);
    assert_eq!(
        editor.hit(view, &t, (field(4).x, field(4).y)),
        Some(Action::Field(4))
    );
    editor.fields[0] = "720".into();
    assert!(editor.apply().is_err());
    assert_eq!(editor.config.placements[1].display, client);
    editor.select(1);
    editor.fields[4] = "-864".into();
    editor.fields[5] = "100".into();
    editor.apply()?;
    assert_eq!(editor.config.placements[1].display, client);
    assert!(editor.save_ready());
    editor.editing = Some(5);
    editor.key(Key::Tab);
    assert_eq!(editor.editing, Some(4));
    assert!(!editor.save_ready());
    assert!(store.load()?.is_none());
    Ok(())
}

#[test]
#[ignore = "requires local graphical desktop; generated authenticated-display fixture"]
fn native_authenticated_geometry_review_presents_read_only_dimensions() -> Result<()> {
    let observed = topology_store::observe_base()?;
    let dir = tempfile::tempdir()?;
    let id = |n: u8| SecretKey::from_bytes(&[n; 32]).public();
    let remote = Display {
        width: 1080,
        height: 1920,
        scale: 125,
        rotation: 90,
    };
    let store = Store::new(&dir.path().join("test"), id(1), vec![id(2)], observed)?
        .with_displays(vec![remote])?;
    let editor = Editor::fresh(&store);
    assert!(editor.authenticated && editor.save_ready());
    let t = display_geometry::topology(&editor.config.placements).context("Test topology")?;
    let projection = Projection::new(&t, EDIT_AREA).context("Test projection")?;
    let mut canvas = vec![0; WIDTH * HEIGHT];
    let mut window = super::super::window(
        "IntraLink | Generated display review test",
        WIDTH,
        HEIGHT,
        false,
        ui::BG,
    )?;
    window.set_target_fps(30);
    editor.render(&mut canvas, &["BASE", "GENERATED CLIENT"], &t, projection);
    let rect = field(0);
    assert_eq!(canvas[rect.y * WIDTH + rect.x], ui::DISABLED);
    for _ in 0..8 {
        window
            .update_with_buffer(&canvas, WIDTH, HEIGHT)
            .map_err(|e| anyhow::anyhow!("{e}"))?;
        assert!(window.is_open());
    }
    assert!(store.load()?.is_none());
    println!(
        "AUTHENTICATED_DISPLAY_NATIVE_PASS submissions=8 base={}x{} scale={} generated CLIENT fixture",
        observed.display.width, observed.display.height, observed.display.scale
    );
    Ok(())
}
