//! Foreground connection review for geometry mode. No input is sent here.
use super::{Slot, topology_canvas as ui, topology_editor, topology_store};
use anyhow::{Context, Result, ensure};
use intralink_core::{
    display_geometry::Display,
    overview::{HEIGHT, WIDTH},
};
use minifb::{Key, Window};
use std::time::{Duration, Instant};
use tokio::time::sleep;

const CONNECT_LIMIT: Duration = Duration::from_secs(25);

/// Every selected host must be live and report its own pinned metadata.
/// Never infer geometry from the advertisement or a saved configuration.
fn collect(slots: &[Slot]) -> Option<Vec<Display>> {
    slots.iter().map(Slot::display).collect()
}

pub(super) async fn await_displays(slots: &[Slot], window: &mut Window) -> Result<Vec<Display>> {
    let deadline = Instant::now() + CONNECT_LIMIT;
    let mut canvas = vec![0; WIDTH * HEIGHT];
    let mut painted = None;
    loop {
        window.update();
        ensure!(
            window.is_open() && !window.is_key_down(Key::Escape) && !window.is_key_down(Key::F12),
            "Authenticated display review canceled"
        );
        ensure!(
            Instant::now() < deadline,
            "Timed out waiting for all selected input/2 CLIENTs"
        );
        for slot in slots {
            ensure!(
                !slot.task.is_finished(),
                "{} disconnected or does not support input/2; no automatic downgrade",
                slot.name
            );
        }
        if let Some(displays) = collect(slots) {
            return Ok(displays);
        }
        let mask = slots.iter().enumerate().fold(0u8, |mask, (i, s)| {
            mask | if s.display().is_some() { 1 << i } else { 0 }
        });
        if painted != Some(mask) {
            canvas.fill(ui::BG);
            ui::label(
                &mut canvas,
                32,
                24,
                "INTRALINK / DETECT CLIENT DISPLAYS",
                2,
                ui::ACCENT,
            );
            ui::label(
                &mut canvas,
                32,
                66,
                "WAITING FOR AUTHENTICATED CLIENTS",
                3,
                ui::INK,
            );
            ui::label(
                &mut canvas,
                32,
                105,
                "INPUT SAFE / NO REMOTE DEVICE IS ACTIVE",
                2,
                ui::MUTED,
            );
            for (i, slot) in slots.iter().enumerate() {
                let y = 150 + i * 42;
                ui::label(&mut canvas, 32, y, &slot.name, 2, ui::INK);
                ui::label(
                    &mut canvas,
                    650,
                    y,
                    if mask & (1 << i) != 0 {
                        "GEOMETRY RECEIVED"
                    } else {
                        "WAITING"
                    },
                    1,
                    if mask & (1 << i) != 0 {
                        ui::ACCENT
                    } else {
                        ui::MUTED
                    },
                );
            }
            ui::label(
                &mut canvas,
                32,
                512,
                "ESC OR F12 CANCELS / BOTH DEVICES MUST APPROVE",
                1,
                ui::MUTED,
            );
            window
                .update_with_buffer(&canvas, WIDTH, HEIGHT)
                .map_err(|e| anyhow::anyhow!("Display review: {e}"))?;
            painted = Some(mask);
        }
        sleep(Duration::from_millis(20)).await;
    }
}

pub(super) fn review(
    store: topology_store::Store,
    displays: Vec<Display>,
    slots: &[Slot],
) -> Result<topology_store::Config> {
    let store = store.with_displays(displays.clone())?;
    let mut names = vec!["BASE"];
    names.extend(slots.iter().map(|s| s.name.as_str()));
    topology_editor::edit_checked(&store, &names, || {
        ensure!(
            collect(slots).as_deref() == Some(displays.as_slice()),
            "Authenticated CLIENT geometry session ended; reconnect and review"
        );
        Ok(())
    })
    .context("Authenticated topology review")
}
