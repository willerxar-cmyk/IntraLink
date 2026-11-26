//! Developer-only startup and CUDA capability probes. The worker has no
//! GUI, vault, network session, parser, or bitstream input.
#![forbid(unsafe_code)]

#[cfg(all(windows, feature = "nvenc-preview"))]
fn main() -> anyhow::Result<()> {
    use anyhow::{Context, bail, ensure};
    use intralink_d3d11::{
        RestrictedDecoderChild, harden_decoder_worker, verify_restricted_headless_cuda,
    };
    use std::{
        io::{Read, Write},
        thread,
        time::{Duration, Instant},
    };

    const STARTUP: &[u8] = b"restricted_headless_startup=passed\n";
    const CUDA: &[u8] = b"restricted_headless_cuda=passed\n";
    let mut args = std::env::args_os().skip(1);
    let first = args.next();
    let second = args.next();
    ensure!(args.next().is_none(), "Unexpected headless probe argument");
    if first
        .as_deref()
        .is_some_and(|s| s == "desktop-restricted-decoder-worker")
    {
        ensure!(second.is_none(), "Unexpected worker argument");
        harden_decoder_worker(true)?;
        let mut stdin = std::io::stdin().lock();
        let mut request = [0u8; 9];
        stdin.read_exact(&mut request)?;
        ensure!(stdin.read(&mut [0u8; 1])? == 0, "Extra bootstrap data");
        let marker = match request[0] {
            0 if request[1..] == [0; 8] => STARTUP,
            1 => {
                let luid = request[1..].try_into().context("Invalid LUID")?;
                verify_restricted_headless_cuda(luid)?;
                CUDA
            }
            _ => bail!("Invalid headless probe bootstrap"),
        };
        let mut stdout = std::io::stdout().lock();
        stdout.write_all(marker)?;
        stdout.flush()?;
        return Ok(());
    }

    let mut request = [0u8; 9];
    let expected = match (first, second) {
        (None, None) => STARTUP,
        (Some(command), Some(luid)) if command == "--cuda-luid" => {
            let luid = luid.to_str().context("LUID must be UTF-8")?;
            let (high, low) = luid.split_once(':').context("Expected HIGH:LOW LUID")?;
            ensure!(
                high.len() == 8 && low.len() == 8,
                "Expected 8-digit LUID halves"
            );
            ensure!(
                high.bytes()
                    .chain(low.bytes())
                    .all(|b| b.is_ascii_hexdigit()),
                "Invalid LUID digits"
            );
            request[0] = 1;
            request[1..5].copy_from_slice(&u32::from_str_radix(low, 16)?.to_le_bytes());
            request[5..9].copy_from_slice(&u32::from_str_radix(high, 16)?.to_le_bytes());
            CUDA
        }
        _ => bail!("Usage: restricted_decoder_probe [--cuda-luid HIGH:LOW]"),
    };

    let (mut child, _job, mut input, output) = RestrictedDecoderChild::spawn()?;
    let sent = input.write_all(&request);
    drop(input);
    let deadline = Instant::now() + Duration::from_secs(10);
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }
        ensure!(Instant::now() < deadline, "Headless probe timed out");
        thread::sleep(Duration::from_millis(20));
    };
    println!("headless_probe_child_pid={}, exit={status}", child.id());
    ensure!(status.success(), "Headless probe failed: {status}");
    sent.context("Send headless probe bootstrap")?;
    // The job allows one process, and it exited. Read only the exact
    // expected marker plus one byte; never trust arbitrary output.
    let mut observed = Vec::new();
    output
        .take((expected.len() + 1) as u64)
        .read_to_end(&mut observed)?;
    ensure!(observed == expected, "Headless probe marker mismatch");
    println!(
        "{}, actual_child_exit=0",
        std::str::from_utf8(expected)?.trim_end()
    );
    Ok(())
}

#[cfg(not(all(windows, feature = "nvenc-preview")))]
fn main() -> std::process::ExitCode {
    eprintln!("Restricted decoder probe requires Windows and nvenc-preview");
    std::process::ExitCode::FAILURE
}
