//! Local software decoder calibration. Only generated pixels and the real
//! production decoder are used; no screen capture, network or vault access.
use std::{
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};

use anyhow::{Context, Result, ensure};
use intralink_core::{
    calibration::{DecodeCohort, DecodeTier, recommend},
    desktop::CodecMode,
    profile_limits::ReceiverLimits,
    workspace::MAX_DEVICES,
};

use super::codec::Decoder;
#[path = "calibration_fixtures.rs"]
mod fixtures;
use fixtures::{Fixture, FixtureSet};

const SAMPLES: usize = 6;

#[derive(clap::Args)]
pub struct ProfileCheckArgs {
    /// Number of simultaneously selected workspace slots.
    #[arg(long, default_value_t = 1, value_parser = clap::value_parser!(u8).range(1..=8))]
    slots: u8,
    /// Probe optional native Windows adapters separately from software decode.
    #[arg(long)]
    hardware: bool,
    /// Measure Host capture backends and software encoding of generated pixels only.
    #[arg(long, conflicts_with_all = ["hardware", "slots"])]
    host: bool,
    /// Compare the current and historical software JPEG encoders on generated pixels.
    #[arg(long, conflicts_with_all = ["host", "hardware", "slots"])]
    encoder: bool,
    /// Compare ordered serial/overlapped capture on an owned window and QUIC loopback.
    #[arg(long, conflicts_with_all = ["host", "encoder", "hardware", "slots"])]
    pipeline: bool,
    /// Compare every decoded pixel against an owned local captured reference (not a benchmark).
    #[arg(long, conflicts_with_all = ["host", "encoder", "hardware", "slots", "pipeline"])]
    visual: bool,
    /// Compare bounded WGC callback locking against the existing wake policy.
    #[arg(long, conflicts_with_all = ["host", "encoder", "hardware", "slots", "pipeline", "visual"])]
    capture_lock: bool,
    /// Compare nonblocking staged D3D11 readback with synchronous capture.
    #[arg(long, conflicts_with_all = ["host", "encoder", "hardware", "slots", "pipeline", "visual", "capture_lock"])]
    staged_readback: bool,
}

impl ProfileCheckArgs {
    pub fn run(self) -> Result<()> {
        if self.pipeline || self.visual || self.capture_lock || self.staged_readback {
            #[cfg(all(target_os = "windows", feature = "wgc-preview"))]
            return crate::run_async(|| {
                super::pipeline_check::run(self.visual, self.capture_lock, self.staged_readback)
            });
            #[cfg(not(all(target_os = "windows", feature = "wgc-preview")))]
            anyhow::bail!("Native pipeline diagnostic requires Windows wgc-preview");
        }
        if self.encoder {
            return super::jpeg_benchmark::run();
        }
        if self.host {
            let ceiling = intralink_core::desktop::VideoConfig::new(1080, 60, 75, 30)
                .context("Invalid Host calibration profile")?
                .with_mode(CodecMode::Compact);
            super::host_calibration::select(ceiling)?;
            return Ok(());
        }
        if self.hardware {
            #[cfg(not(all(target_os = "windows", feature = "wgc-preview")))]
            anyhow::bail!("Native adapter probe requires Windows wgc-preview");
        }
        let ceilings = vec![ReceiverLimits::MAX; usize::from(self.slots)];
        let limits = measure(CodecMode::Compact, &ceilings)?;
        for (index, limit) in limits.iter().enumerate() {
            println!(
                "recommended_slot={}, recommended_height={}, recommended_fps={}, slots={}, software_only=true",
                index + 1,
                limit.height(),
                limit.fps(),
                self.slots
            );
        }
        #[cfg(all(target_os = "windows", feature = "wgc-preview"))]
        if self.hardware {
            intralink_d3d11::probe_video_capabilities()?;
        }
        println!(
            "Generated local fixtures only; no remote performance or hardware codec qualification."
        );
        Ok(())
    }
}

#[derive(Clone, Copy)]
struct Work<'a> {
    fixture: &'a Fixture,
    base: Option<&'a [u8]>,
}

struct WorkerResult {
    worst: Duration,
    finished: Instant,
    samples: usize,
}

fn run_worker(
    work: Work<'_>,
    ready: mpsc::Sender<()>,
    start: mpsc::Receiver<()>,
) -> Result<WorkerResult> {
    // Each worker owns its decoder and output canvas. Only the generated
    // immutable compressed bytes are shared; no shared decoder lock.
    let mut decoder = Decoder::default();
    let mut pixels = Vec::new();
    let config = work.fixture.config;
    let expected = (config.width() * config.height()) as usize;
    if let Some(base) = work.base {
        decoder.decode_into(config, base, &mut pixels)?;
        ensure!(
            pixels.len() == expected,
            "Calibration base geometry mismatch"
        );
    }
    let retained = work.fixture.retained_pixel(&pixels);
    decoder.decode_into(config, &work.fixture.bytes, &mut pixels)?;
    work.fixture.verify(&pixels, retained)?;
    // Every successful warm-up signals ready then drops its sender. If a
    // worker fails or panics during warm-up, the coordinator can detect
    // channel closure and release all other workers without a barrier hang.
    ready
        .send(())
        .context("Calibration readiness receiver closed")?;
    drop(ready);
    start.recv().context("Calibration start canceled")?;
    let mut worst = Duration::ZERO;
    for _ in 0..SAMPLES {
        let began = Instant::now();
        decoder.decode_into(config, &work.fixture.bytes, &mut pixels)?;
        worst = worst.max(began.elapsed());
        ensure!(
            pixels.len() == expected,
            "Calibration decode geometry changed"
        );
    }
    let finished = Instant::now();
    ensure!(!worst.is_zero(), "Local clock cannot measure decoder work");
    work.fixture.verify(&pixels, retained)?;
    Ok(WorkerResult {
        worst,
        finished,
        samples: SAMPLES,
    })
}

struct Batch {
    periods: Vec<Duration>,
    wall: Duration,
    worst: Duration,
    completed: usize,
}

fn measure_batch(work: &[Work<'_>]) -> Result<Batch> {
    ensure!(
        (1..=MAX_DEVICES).contains(&work.len()),
        "Invalid worker count"
    );
    thread::scope(|scope| -> Result<Batch> {
        let (ready_tx, ready_rx) = mpsc::channel();
        let mut starts = Vec::with_capacity(work.len());
        let mut workers = Vec::with_capacity(work.len());
        for item in work.iter().copied() {
            let (start_tx, start_rx) = mpsc::channel();
            let ready = ready_tx.clone();
            let worker = thread::Builder::new()
                .name("intralink-calibration".to_owned())
                .spawn_scoped(scope, move || run_worker(item, ready, start_rx))
                .context("Unable to start local calibration worker")?;
            starts.push(start_tx);
            workers.push(worker);
        }
        drop(ready_tx);
        let mut all_ready = true;
        for _ in work {
            if ready_rx.recv().is_err() {
                all_ready = false;
                break;
            }
        }
        let began = Instant::now();
        if all_ready {
            for start in &starts {
                if start.send(()).is_err() {
                    all_ready = false;
                    break;
                }
            }
        }
        // Dropping every sender also cancels any worker that did not receive
        // a start signal. Scoped handles join on every success/error path.
        drop(starts);
        // Join every handle even if one worker failed. Never leave a
        // panicking worker for the implicit scope destructor to rethrow.
        let joined: Vec<_> = workers.into_iter().map(|worker| worker.join()).collect();
        let results: Vec<_> = joined
            .into_iter()
            .map(|result| result.map_err(|_| anyhow::anyhow!("Calibration worker panicked"))?)
            .collect::<Result<_>>()?;
        ensure!(all_ready, "Calibration cohort did not start completely");
        let finished = results
            .iter()
            .map(|result| result.finished)
            .max()
            .context("No calibration results")?;
        let wall = finished.duration_since(began);
        let completed = results.iter().map(|result| result.samples).sum();
        ensure!(
            completed == work.len() * SAMPLES && !wall.is_zero(),
            "Incomplete calibration cohort"
        );
        // Limit every worker by both its own slowest decode and the observed
        // full-cohort wall-time per round. Never divide by worker count again.
        let round = Duration::from_nanos(u64::try_from(wall.as_nanos().div_ceil(SAMPLES as u128))?);
        let worst = results
            .iter()
            .map(|result| result.worst)
            .max()
            .context("No worker timing")?;
        let periods = results
            .into_iter()
            .map(|result| result.worst.max(round))
            .collect();
        Ok(Batch {
            periods,
            wall,
            worst,
            completed,
        })
    })
}

fn measure_cohort(height: u16, sets: &[&FixtureSet]) -> Result<DecodeCohort> {
    let first = sets.first().context("No calibration slots")?;
    ensure!(
        sets.iter().all(|set| set.items.len() == first.items.len()),
        "Inconsistent calibration fixture modes"
    );
    let mut periods = vec![Duration::ZERO; sets.len()];
    for index in 0..first.items.len() {
        let work: Vec<_> = sets
            .iter()
            .map(|set| Work {
                fixture: &set.items[index],
                base: set.base(index),
            })
            .collect();
        ensure!(
            work.iter()
                .all(|item| item.fixture.name() == first.items[index].name()),
            "Inconsistent calibration representation"
        );
        let result = measure_batch(&work)?;
        for (period, measured) in periods.iter_mut().zip(result.periods) {
            *period = (*period).max(measured);
        }
        println!(
            "cohort_height={height}, fixture={}, workers={}, completed={}, wall_ms={:.3}, worst_frame_ms={:.3}, aggregate_decode_fps={:.2}",
            first.items[index].name(),
            sets.len(),
            result.completed,
            result.wall.as_secs_f64() * 1000.0,
            result.worst.as_secs_f64() * 1000.0,
            result.completed as f64 / result.wall.as_secs_f64()
        );
    }
    let slots = sets
        .iter()
        .zip(periods)
        .enumerate()
        .map(|(index, (set, period))| {
            println!(
                "cohort_height={height}, slot={}, measured_height={}, effective_period_ms={:.3}",
                index + 1,
                set.height,
                period.as_secs_f64() * 1000.0
            );
            DecodeTier {
                height: set.height,
                period,
            }
        })
        .collect();
    Ok(DecodeCohort { height, slots })
}

pub(super) fn measure(mode: CodecMode, ceilings: &[ReceiverLimits]) -> Result<Vec<ReceiverLimits>> {
    ensure!(
        (1..=MAX_DEVICES).contains(&ceilings.len()),
        "Select 1-8 receiver slots"
    );
    let maximum = ceilings
        .iter()
        .map(|limit| limit.height())
        .max()
        .context("Missing receiver ceiling")?;
    println!(
        "Local concurrent decoder calibration: mode={mode:?}, slots={}, logical_cpus={:?}",
        ceilings.len(),
        thread::available_parallelism().ok().map(|n| n.get())
    );
    // Construct immutable fixtures only once per permitted geometry, outside
    // the measured region. Mixed ceilings are tested together as one cohort.
    let mut fixtures = Vec::<FixtureSet>::with_capacity(3);
    let mut cohorts = Vec::with_capacity(3);
    for height in [1080, 720, 540] {
        if height > maximum {
            continue;
        }
        for ceiling in ceilings {
            let target = height.min(ceiling.height());
            if !fixtures.iter().any(|set| set.height == target) {
                fixtures.push(FixtureSet::new(mode, target)?);
            }
        }
        let sets: Vec<_> = ceilings
            .iter()
            .map(|limit| {
                let target = height.min(limit.height());
                fixtures
                    .iter()
                    .find(|set| set.height == target)
                    .context("Missing local calibration fixture")
            })
            .collect::<Result<_>>()?;
        cohorts.push(measure_cohort(height, &sets)?);
        // Highest candidate first. Once every slot meets its minimum target,
        // the policy cannot prefer a smaller layout. Avoid unnecessary runs.
        if let Some(limits) = recommend(&cohorts, ceilings)
            && limits
                .iter()
                .zip(ceilings)
                .all(|(limit, ceiling)| limit.fps() >= ceiling.fps().min(30))
        {
            return Ok(limits);
        }
    }
    recommend(&cohorts, ceilings)
        .context("Measured concurrent decode budget cannot support the receiver profiles")
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    #[derive(Parser)]
    struct DiagnosticCli {
        #[command(flatten)]
        args: ProfileCheckArgs,
    }

    #[test]
    fn capture_lock_diagnostic_is_explicit_and_exclusive() {
        let default = DiagnosticCli::try_parse_from(["test"]).unwrap();
        assert!(!default.args.capture_lock);
        let selected = DiagnosticCli::try_parse_from(["test", "--capture-lock"]).unwrap();
        assert!(selected.args.capture_lock);
        for option in [
            "--host",
            "--encoder",
            "--hardware",
            "--pipeline",
            "--visual",
        ] {
            assert!(DiagnosticCli::try_parse_from(["test", "--capture-lock", option]).is_err());
        }
        assert!(DiagnosticCli::try_parse_from(["test", "--capture-lock", "--slots", "1"]).is_err());
    }

    #[test]
    fn staged_readback_diagnostic_is_explicit_and_exclusive() {
        let default = DiagnosticCli::try_parse_from(["test"]).unwrap();
        assert!(!default.args.staged_readback);
        let selected = DiagnosticCli::try_parse_from(["test", "--staged-readback"]).unwrap();
        assert!(selected.args.staged_readback);
        for option in [
            "--host",
            "--encoder",
            "--hardware",
            "--pipeline",
            "--visual",
            "--capture-lock",
        ] {
            assert!(DiagnosticCli::try_parse_from(["test", "--staged-readback", option]).is_err());
        }
        assert!(
            DiagnosticCli::try_parse_from(["test", "--staged-readback", "--slots", "1"]).is_err()
        );
    }

    #[test]
    fn real_software_fixtures_round_trip_without_timing_assumptions() -> Result<()> {
        for mode in [CodecMode::Jpeg, CodecMode::Tiles, CodecMode::Compact] {
            let fixture = FixtureSet::new(mode, 540)?;
            let cohort = measure_cohort(540, &[&fixture])?;
            assert_eq!(cohort.slots.len(), 1);
            assert_eq!(cohort.slots[0].height, 540);
            assert!(!cohort.slots[0].period.is_zero());
        }
        Ok(())
    }

    #[test]
    fn concurrent_mixed_geometry_workers_complete_and_join() -> Result<()> {
        let high = FixtureSet::new(CodecMode::Jpeg, 720)?;
        let low = FixtureSet::new(CodecMode::Jpeg, 540)?;
        let work = [
            Work {
                fixture: &high.items[0],
                base: None,
            },
            Work {
                fixture: &low.items[0],
                base: None,
            },
        ];
        let batch = measure_batch(&work)?;
        assert_eq!(batch.completed, 2 * SAMPLES);
        assert_eq!(batch.periods.len(), 2);
        assert!(batch.periods.iter().all(|period| !period.is_zero()));
        Ok(())
    }

    #[test]
    fn failed_warmup_cancels_waiting_workers() -> Result<()> {
        let mut bad = FixtureSet::new(CodecMode::Jpeg, 540)?;
        let good = FixtureSet::new(CodecMode::Jpeg, 540)?;
        bad.items[0].bytes.clear();
        let work = [
            Work {
                fixture: &good.items[0],
                base: None,
            },
            Work {
                fixture: &bad.items[0],
                base: None,
            },
        ];
        assert!(measure_batch(&work).is_err());
        assert!(measure_batch(&[]).is_err());
        Ok(())
    }
}
