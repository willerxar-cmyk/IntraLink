//! Foreground workspace lifecycle, including authenticated display review.
use super::*;

pub(crate) async fn configure_topology(path: &Path, names: &[String]) -> Result<()> {
    let (_vault, secret, targets) = load_targets(path, names, false, false, false).await?;
    let observed = topology_store::observe_base()?;
    let peers = targets.iter().map(|(_, id, _, _)| *id).collect();
    let store = topology_store::Store::new(path, secret.public(), peers, observed)?;
    let mut names = vec!["BASE"];
    names.extend(targets.iter().map(|(name, _, _, _)| name.as_str()));
    topology_editor::edit(&store, &names)?;
    println!("Topology saved locally; no connection or input session opened.");
    Ok(())
}

#[derive(Clone, Copy)]
pub(crate) enum WorkspaceMode {
    Manual,
    Spatial,
    Geometry { detect_displays: bool },
    DesktopEdges { source: SourceMode },
}

pub(crate) async fn workspace(
    path: &Path,
    names: &[String],
    choose: bool,
    lan: bool,
    mode: WorkspaceMode,
) -> Result<()> {
    let spatial = matches!(mode, WorkspaceMode::Spatial);
    let geometry = matches!(
        mode,
        WorkspaceMode::Geometry { .. } | WorkspaceMode::DesktopEdges { .. }
    );
    let desktop_source = match mode {
        WorkspaceMode::DesktopEdges { source } => Some(source),
        _ => None,
    };
    let desktop_edges = desktop_source.is_some();
    let detect_displays = desktop_edges
        || matches!(
            mode,
            WorkspaceMode::Geometry {
                detect_displays: true
            }
        );
    ensure!(
        !detect_displays || geometry,
        "Authenticated displays require geometry mode"
    );
    let (_vault, secret, targets) = load_targets(path, names, choose, detect_displays, lan).await?;
    let store = if geometry {
        let observed = topology_store::observe_base()?;
        let peers = targets.iter().map(|(_, id, _, _)| *id).collect();
        Some(topology_store::Store::new(
            path,
            secret.public(),
            peers,
            observed,
        )?)
    } else {
        None
    };
    let mut geometry = if !detect_displays {
        if let Some(store) = store.as_ref() {
            let mut names = vec!["BASE"];
            names.extend(targets.iter().map(|(name, _, _, _)| name.as_str()));
            let config = topology_editor::edit(store, &names)?;
            Some(topology_pad::Pad::new(config, store.observed(), false)?)
        } else {
            None
        }
    } else {
        None
    };
    approve(if matches!(desktop_source, Some(SourceMode::CompactRaw)) {
        "Open experimental compact Raw Input KVM? The BASE stays visible. The shared edge opens a small, temporary foreground panel; only relative mouse motion from the focused panel is read, and its pointer is recentered after acknowledged activation. Keyboard/buttons use the focused window. Fast motion leaving the panel, device change, overflow, focus loss or stop cancels the session. No global hook, cursor clipping, background keyboard capture, or elevation. Each CLIENT must independently approve."
    } else if matches!(desktop_source, Some(SourceMode::Raw)) {
        "Open experimental foreground Raw Input KVM? The shared edge opens a temporary full-screen foreground surface. Read relative mouse motion only while it owns foreground; keyboard/buttons remain focused-window input. Unsupported mouse reports, overflow or focus loss stop the session. No global hook, cursor clipping, background keyboard capture, or elevation. Each CLIENT must independently approve."
    } else if desktop_edges {
        "Open the explicitly armed physical-edge KVM preview? While LOCAL, only cursor position is sampled and a neutral-state check occurs at handoff. When a shared edge is reached, IntraLink requests a temporary visible full-screen foreground surface and recenters its pointer while relaying input to the approved CLIENT. F12 or Escape stops. System shortcuts and unsupported keys are not globally intercepted. No hooks, clipping, screen stream or elevation. Each CLIENT must approve independently."
    } else if detect_displays {
        "Connect to the selected input/2 CLIENTs and receive their current primary-display metadata? Each CLIENT must independently approve this session. Input remains SAFE during review and requires a separate local arm action. No downgrade, global hook or video stream."
    } else if geometry.is_some() {
        "Open the foreground geometry-aware input pad? The configured monitor geometry applies only inside this window. Each CLIENT must approve independently. Click BASE or F9 to arm, F12 releases. No global hook or screen stream."
    } else if spatial {
        "Open the visible spatial input pad? Review the arrangement and start from the local BASE card. START or F9 arms switching; F12 disarms and releases input. No global hook or screen stream."
    } else {
        "Open the visible input-only pad for these paired hosts? F1-F8 explicitly activate a ready host; F12 returns to local. No global hook, screen stream or unattended control."
    })?;
    let mut spatial = if spatial && geometry.is_none() && !detect_displays {
        let peers = targets
            .iter()
            .map(|(_, identity, _, _)| *identity)
            .collect();
        let store = layout_store::Store::new(path, secret.public(), peers)?;
        Some(Spatial::new(store))
    } else {
        None
    };
    let mut window = window(
        "IntraLink INPUT | LOCAL | connecting",
        960,
        540,
        false,
        0x00212c3b,
    )?;
    let endpoint = bind_workspace_controller(secret).await?;
    let mut slots: Vec<_> = targets
        .into_iter()
        .map(|(name, identity, route, hints)| {
            start_slot(&endpoint, name, identity, route, hints, detect_displays)
        })
        .collect();
    for (index, slot) in slots.iter().enumerate() {
        println!("F{}: {}", index + 1, slot.name);
    }
    println!(
        "Focus the input pad. Its pointer maps to the active host's primary display. F12 releases the selected host; Escape closes. Do not use two profiles on the same computer."
    );
    let mut focus = Focus::default();
    let mut previous = Snapshot::default();
    let mut last_title = String::new();
    if geometry.is_some() {
        println!(
            "Geometry mode: pointer transfers only over exact shared logical edges. CLIENT dimensions are user-confirmed, not remotely measured. F12 releases. Window only."
        );
    }
    if spatial.is_some() {
        println!(
            "Spatial pad: drag named devices while SAFE. Changes save locally for this BASE and exact CLIENT set. Review the layout, click START or press F9 in the BASE card, cross with keys/buttons released. F12 disarms. F10 or the portal button changes intensity; RESET restores defaults."
        );
    }
    let local = async {
        if detect_displays {
            let displays=super::display_review::await_displays(&slots,&mut window).await?;
            drop(window);
            let store=store.context("Missing geometry preference store")?;
            let observed=store.observed();
            let config=super::display_review::review(store,displays,&slots)?;
            if let Some(source)=desktop_source {
                return super::desktop_edges::run(config,observed,&slots,source).await;
            }
            geometry=Some(topology_pad::Pad::new(config,observed,true)?);
            window=super::window("IntraLink INPUT | AUTHENTICATED GEOMETRY SAFE",960,540,false,0x00212c3b)?;
            println!("Authenticated primary display geometry reviewed. Click BASE or F9 to arm. F12 releases.");
        }
        loop {
            window.update();
            if !window.is_open() || window.is_key_down(LocalKey::Escape) {
                break;
            }
            if focus.active().is_some() && !window.is_active() {
                anyhow::bail!(
                    "Input pad lost focus; disconnecting rather than forwarding stale input"
                );
            }
            if let Some(geometry) = &mut geometry {
                geometry
                    .step(&mut focus, &mut previous, &slots, &mut window)
                    .await?;
                geometry.paint(&slots, &focus, &mut window)?;
            } else if let Some(spatial) = &mut spatial {
                spatial
                    .step(&mut focus, &mut previous, &slots, &mut window)
                    .await?;
                spatial.paint(&slots, &focus, &mut window)?;
            } else if window.is_active() {
                let select = window
                    .get_keys_pressed(KeyRepeat::No)
                    .into_iter()
                    .find_map(|key| SELECT_KEYS.iter().position(|candidate| *candidate == key));
                let return_local = window.is_key_pressed(LocalKey::F12, KeyRepeat::No)
                    || (focus.active().is_some() && normalized(&window).is_none());
                if return_local {
                    switch_focus(&mut focus, None, &slots, &mut window).await?;
                    previous = Snapshot::default();
                } else if let Some(index) = select {
                    if slots.get(index).is_some_and(Slot::connected)
                        && local_keys(&window).is_empty()
                        && local_buttons(&window).is_empty()
                        && normalized(&window).is_some()
                    {
                        if focus.active() != Some(index) {
                            switch_focus(&mut focus, Some(index), &slots, &mut window).await?;
                            previous = Snapshot::default();
                        }
                    } else {
                        println!(
                            "Target unavailable or local input still held; select again when ready."
                        );
                    }
                }
                if let Some(index) = focus.active() {
                    ensure!(window.is_active(), "Input pad lost focus");
                    let position = normalized(&window).context("Pointer left the input pad")?;
                    previous.forward(&window, &slots[index], position)?;
                }
            }
            let next = title(&slots, &focus, spatial.as_ref(), geometry.as_ref());
            if next != last_title {
                window.set_title(&next);
                last_title = next;
            }
            sleep(POLL).await;
        }
        Ok::<(), anyhow::Error>(())
    }
    .await;
    // Abort every owned worker on close/focus loss. Dropping the connection
    // closes its QUIC session, causing the host to release its owned input.
    focus.cancel();
    for slot in &slots {
        slot.task.abort();
    }
    for slot in &mut slots {
        let _ = (&mut slot.task).await;
    }
    endpoint.close().await;
    local
}
