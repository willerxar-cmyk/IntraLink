//! Explicit foreground local sampling only. No network or background service.
use std::{
    io::{self, Write},
    time::{Duration, Instant},
};

use anyhow::{Context, Result, ensure};
use intralink_core::resources::{Cpu, Memory};
use serde::Serialize;
use sysinfo::{MINIMUM_CPU_UPDATE_INTERVAL, System};

#[derive(Serialize)]
struct Sample {
    sequence: u16,
    interval_ms: u64,
    logical_processors: Option<u32>,
    cpu_usage_percent: Option<f32>,
    memory_total_bytes: Option<u64>,
    memory_available_bytes: Option<u64>,
    memory_used_bytes: Option<u64>,
}

impl Sample {
    fn new(sequence: u16, interval_ms: u64, cpu: Option<Cpu>, memory: Option<Memory>) -> Self {
        Self {
            sequence,
            interval_ms,
            logical_processors: cpu.map(Cpu::logical_processors),
            cpu_usage_percent: cpu.map(Cpu::usage_percent),
            memory_total_bytes: memory.map(Memory::total),
            memory_available_bytes: memory.map(Memory::available),
            memory_used_bytes: memory.map(Memory::used),
        }
    }

    fn write(&self, mut out: impl Write, json: bool) -> Result<()> {
        if json {
            serde_json::to_writer(&mut out, self)?;
            writeln!(out)?;
        } else {
            write!(out, "Sample {} ({} ms): ", self.sequence, self.interval_ms)?;
            match (self.logical_processors, self.cpu_usage_percent) {
                (Some(cpus), Some(percent)) => {
                    write!(out, "CPU {percent:.1}% ({cpus} logical CPUs)")?
                }
                _ => write!(out, "CPU unavailable")?,
            }
            match (self.memory_used_bytes, self.memory_total_bytes) {
                (Some(used), Some(total)) => {
                    writeln!(out, "; RAM {:.2}/{:.2} GiB used", gib(used), gib(total))?
                }
                _ => writeln!(out, "; RAM unavailable")?,
            }
        }
        out.flush()?;
        Ok(())
    }
}

fn gib(bytes: u64) -> f64 {
    bytes as f64 / 1_073_741_824.0
}

pub async fn report(interval_ms: u64, samples: u16, json: bool) -> Result<()> {
    ensure!(
        (500..=10000).contains(&interval_ms),
        "Sampling interval outside 500–10000 ms"
    );
    ensure!((1..=120).contains(&samples), "Sample count outside 1–120");
    let interval = Duration::from_millis(interval_ms).max(MINIMUM_CPU_UPDATE_INTERVAL);
    // Reuse one system snapshot and never enumerate processes, users, disks,
    // or interfaces. CPU usage needs two refreshes separated by real time.
    let mut system = System::new().context("Cannot initialize local resource sampler")?;
    system.refresh_cpu_usage();
    let mut previous = Instant::now();
    for sequence in 1..=samples {
        tokio::time::sleep(interval).await;
        system.refresh_cpu_usage();
        let now = Instant::now();
        let elapsed = u64::try_from(now.duration_since(previous).as_millis())
            .context("Sampling interval overflow")?;
        previous = now;
        system.refresh_memory();
        let cpu = Cpu::new(system.cpus().len(), system.global_cpu_usage());
        let memory = Memory::new(system.total_memory(), system.available_memory());
        let sample = Sample::new(sequence, elapsed, cpu, memory);
        sample.write(io::stdout().lock(), json)?;
        ensure!(
            cpu.is_some() || memory.is_some(),
            "The OS returned no valid resource metrics"
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_distinguishes_missing_metrics_from_zero_usage() -> Result<()> {
        let mut out = Vec::new();
        Sample::new(1, 1000, Cpu::new(4, 0.0), None).write(&mut out, true)?;
        let value: serde_json::Value = serde_json::from_slice(&out)?;
        assert_eq!(value["sequence"], 1);
        assert_eq!(value["interval_ms"], 1000);
        assert_eq!(value["cpu_usage_percent"].as_f64(), Some(0.0));
        assert_eq!(value["logical_processors"], 4);
        assert!(value["memory_total_bytes"].is_null());
        assert_eq!(out.last(), Some(&b'\n'));
        Ok(())
    }

    #[test]
    fn missing_cpu_is_not_printed_as_idle_and_ram_uses_bytes() -> Result<()> {
        let mut out = Vec::new();
        Sample::new(2, 500, None, Memory::new(2_147_483_648, 1_073_741_824))
            .write(&mut out, false)?;
        let text = String::from_utf8(out)?;
        assert!(text.contains("CPU unavailable"));
        assert!(text.contains("RAM 1.00/2.00 GiB used"));
        Ok(())
    }
}
