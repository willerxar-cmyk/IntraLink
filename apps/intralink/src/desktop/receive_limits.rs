//! Local receiver resource limits and per-workspace slot overrides.
use anyhow::{Context, Result, ensure};
use intralink_core::{
    adaptive::ViewProtocol, desktop::CodecMode, profile_limits::ReceiverLimits,
    workspace::MAX_DEVICES,
};
use intralink_transport::desktop::ViewMode;

#[derive(clap::Args, Clone, Copy)]
pub struct ReceiveProfileArgs {
    /// Negotiate software codec, receiver resolution and FPS ceilings (v12).
    #[arg(long, conflicts_with_all = ["adaptive_resolution", "negotiate_codec"])]
    negotiate_profile: bool,
    /// Receiver resolution ceiling: 540, 720 or 1080 (default 1080).
    #[arg(long, requires = "negotiate_profile")]
    max_height: Option<u16>,
    /// Receiver FPS ceiling, 1-60 (default 60).
    #[arg(long, requires = "negotiate_profile", value_parser = clap::value_parser!(u32).range(1..=60))]
    max_fps: Option<u32>,
    /// Measure local software decoding and reduce the requested profile if needed.
    #[arg(long, requires = "negotiate_profile")]
    auto_profile: bool,
}

impl ReceiveProfileArgs {
    pub fn single_mode(
        self,
        codec: CodecMode,
        adaptive: bool,
        negotiated: bool,
    ) -> Result<ViewMode> {
        let mode = self.mode(codec, adaptive, negotiated)?;
        if !self.auto_profile {
            return Ok(mode);
        }
        let calibrated = super::calibration::measure(codec, &[mode.limits])?
            .into_iter()
            .next()
            .context("Missing measured receiver profile")?;
        println!(
            "Measured receiver ceiling: {}p/{} FPS",
            calibrated.height(),
            calibrated.fps()
        );
        Ok(mode.with_limits(calibrated))
    }

    pub fn workspace_modes(
        self,
        codec: CodecMode,
        adaptive: bool,
        negotiated: bool,
        names: &[String],
        overrides: &[String],
    ) -> Result<Vec<ViewMode>> {
        let base = self.mode(codec, adaptive, negotiated)?;
        let mut modes = slot_modes(names, base, overrides)?;
        if self.auto_profile {
            let ceilings: Vec<_> = modes.iter().map(|mode| mode.limits).collect();
            let calibrated = super::calibration::measure(codec, &ceilings)?;
            ensure!(
                calibrated.len() == modes.len(),
                "Incomplete calibrated workspace"
            );
            for ((name, mode), selected) in names.iter().zip(&mut modes).zip(calibrated) {
                println!(
                    "Measured receiver ceiling for {name}: {}p/{} FPS",
                    selected.height(),
                    selected.fps()
                );
                *mode = mode.with_limits(selected);
            }
        }
        Ok(modes)
    }

    fn mode(self, codec: CodecMode, adaptive: bool, negotiated: bool) -> Result<ViewMode> {
        let limits = ReceiverLimits::new(
            self.max_height.unwrap_or(ReceiverLimits::MAX.height()),
            self.max_fps.unwrap_or(ReceiverLimits::MAX.fps()),
        )
        .context("Receiver limits require height 540/720/1080 and FPS 1-60")?;
        Ok(ViewMode::new(
            codec,
            super::view_protocol(adaptive, negotiated, self.negotiate_profile),
        )
        .with_limits(limits))
    }
}

/// Override exact selected paired-device names only, never increase the
/// controller's global limit, and never silently accept duplicate overrides.
pub(super) fn slot_modes(
    names: &[String],
    base: ViewMode,
    overrides: &[String],
) -> Result<Vec<ViewMode>> {
    ensure!(
        (1..=MAX_DEVICES).contains(&names.len()),
        "Select 1-8 distinct hosts"
    );
    ensure!(
        overrides.len() <= names.len(),
        "Too many device limit overrides"
    );
    ensure!(
        overrides.is_empty() || base.protocol == ViewProtocol::Bounded,
        "Device limits require v12"
    );
    let mut modes = vec![base; names.len()];
    let mut applied = vec![false; names.len()];
    for entry in overrides {
        let (name, values) = entry
            .rsplit_once('=')
            .context("Use --device-limit NAME=HEIGHT:FPS")?;
        let (height, fps) = values
            .split_once(':')
            .context("Use --device-limit NAME=HEIGHT:FPS")?;
        let limits = ReceiverLimits::new(height.parse()?, fps.parse()?)
            .context("Invalid device resolution or FPS ceiling")?;
        let index = names
            .iter()
            .position(|selected| selected == name)
            .context("Device limit names an unselected host")?;
        ensure!(!applied[index], "Duplicate device limit override");
        ensure!(
            limits.height() <= base.limits.height() && limits.fps() <= base.limits.fps(),
            "Device limit exceeds controller ceiling"
        );
        modes[index] = base.with_limits(limits);
        applied[index] = true;
    }
    Ok(modes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn per_device_limits_are_independent_and_bounded() {
        let names = ["first".to_owned(), "second".to_owned(), "third".to_owned()];
        let base = ViewMode::new(CodecMode::Compact, ViewProtocol::Bounded);
        let modes = slot_modes(
            &names,
            base,
            &["second=720:30".to_owned(), "third=540:15".to_owned()],
        )
        .unwrap();
        assert_eq!(modes[0], base);
        assert_eq!(modes[1].limits, ReceiverLimits::new(720, 30).unwrap());
        assert_eq!(modes[2].limits, ReceiverLimits::new(540, 15).unwrap());
        for bad in [
            "missing=540:15",
            "first=1440:30",
            "first=540:61",
            "first=720",
            "first=540:15:1",
        ] {
            assert!(slot_modes(&names, base, &[bad.to_owned()]).is_err());
        }
        assert!(
            slot_modes(
                &names,
                base,
                &["first=540:15".to_owned(), "first=720:30".to_owned()]
            )
            .is_err()
        );
        assert!(
            slot_modes(
                &names,
                base.with_limits(ReceiverLimits::new(720, 30).unwrap()),
                &["first=1080:60".to_owned()]
            )
            .is_err()
        );
        assert!(
            slot_modes(
                &names,
                ViewMode::new(CodecMode::Compact, ViewProtocol::Negotiated),
                &["first=540:15".to_owned()]
            )
            .is_err()
        );
    }
}
