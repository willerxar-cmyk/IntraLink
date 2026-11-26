//! Developer-only startup probe: same strict launcher, no GUI, network,
//! vault, codec or remote content. Keep the worker command fixed.
#![forbid(unsafe_code)]

#[cfg(all(windows, feature = "nvenc-preview"))]
fn main() -> anyhow::Result<()> {
    use anyhow::{bail, ensure};
    use intralink_d3d11::{RestrictedDecoderChild, harden_decoder_worker};
    use std::{
        io::{Read, Write},
        thread,
        time::{Duration, Instant},
    };

    const MARKER: &[u8] = b"restricted_headless_startup=passed\n";
    let mut args = std::env::args_os().skip(1);
    match (args.next(), args.next()) {
        (Some(command), None) if command == "desktop-restricted-decoder-worker" => {
            harden_decoder_worker(true)?;
            let mut stdout = std::io::stdout().lock();
            stdout.write_all(MARKER)?;
            stdout.flush()?;
        }
        (None, None) => {
            let (mut child, _job, input, output) = RestrictedDecoderChild::spawn()?;
            drop(input);
            let deadline = Instant::now() + Duration::from_secs(10);
            let status = loop {
                if let Some(status) = child.try_wait()? {
                    break status;
                }
                ensure!(
                    Instant::now() < deadline,
                    "Headless probe startup timed out"
                );
                thread::sleep(Duration::from_millis(20));
            };
            println!("headless_probe_child_pid={}, exit={status}", child.id());
            ensure!(status.success(), "Headless probe failed: {status}");
            // The job allows one process, and it exited. Read at most the
            // expected marker plus one byte; never trust arbitrary output.
            let mut observed = Vec::new();
            output
                .take((MARKER.len() + 1) as u64)
                .read_to_end(&mut observed)?;
            ensure!(observed == MARKER, "Headless probe marker mismatch");
            println!("restricted_headless_startup=passed, actual_child_exit=0");
        }
        _ => bail!("Invalid headless probe invocation"),
    }
    Ok(())
}

#[cfg(not(all(windows, feature = "nvenc-preview")))]
fn main() -> std::process::ExitCode {
    eprintln!("Restricted decoder probe requires Windows and nvenc-preview");
    std::process::ExitCode::FAILURE
}
