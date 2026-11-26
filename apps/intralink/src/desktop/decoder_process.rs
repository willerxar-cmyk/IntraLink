//! Parent-side decoder process supervision. A bounded IPC thread owns the
//! pipes; the supervisor owns the child and its kill-on-close Windows job.
use std::{
    io::{BufRead, BufReader, Read, Write},
    process::{Child, Command, ExitStatus, Stdio},
    sync::mpsc::{self, Receiver, SyncSender},
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

use anyhow::{Context, Result, ensure};
use intralink_core::h264;
use intralink_d3d11::{DecoderJob, RestrictedDecoderChild};
use tokio::sync::mpsc as async_mpsc;

use super::decoder_ipc::Start;
use super::live_h264::DecodeJob;

const START_LIMIT: Duration = Duration::from_secs(6);
const FRAME_LIMIT: Duration = Duration::from_secs(2);
const EXIT_LIMIT: Duration = Duration::from_secs(4);
const MAX_LINE: usize = 1024;

enum Request {
    Start(Start, bool),
    Frame { bytes: Vec<u8>, sequence: u32 },
    Finish { submitted: usize, canceled: bool },
}

fn line(input: &mut impl BufRead) -> Result<String> {
    let mut bytes = Vec::with_capacity(128);
    let read = input
        .take((MAX_LINE + 1) as u64)
        .read_until(b'\n', &mut bytes)?;
    ensure!(
        read > 0 && read <= MAX_LINE && bytes.last() == Some(&b'\n'),
        "Decoder IPC response closed or exceeded line budget"
    );
    let text = String::from_utf8(bytes)?;
    Ok(text.trim_end_matches(['\r', '\n']).to_owned())
}

fn expect(input: &mut impl BufRead, expected: &str) -> Result<()> {
    let actual = line(input)?;
    ensure!(
        actual == expected,
        "Unexpected decoder IPC response: {actual:?}"
    );
    Ok(())
}

fn exchange(output: &mut impl Write, input: &mut impl BufRead, request: Request) -> Result<()> {
    match request {
        Request::Start(start, restricted) => {
            output.write_all(&start.encode())?;
            output.flush()?;
            expect(
                input,
                if restricted {
                    "ILDEC READY 1 RESTRICTED"
                } else {
                    "ILDEC READY 1"
                },
            )?;
        }
        Request::Frame { bytes, sequence } => {
            output.write_all(&u32::try_from(bytes.len())?.to_be_bytes())?;
            output.write_all(&bytes)?;
            output.flush()?;
            expect(input, &format!("ILDEC ACK {sequence}"))?;
        }
        Request::Finish {
            submitted,
            canceled,
        } => {
            if canceled {
                output.write_all(&0u32.to_be_bytes())?;
                output.flush()?;
            }
            let mut preview = false;
            let mut decoded = false;
            // Native finish emits two diagnostic lines followed by DONE.
            // No unbounded child output, free-form RPC or ignored protocol.
            for _ in 0..3 {
                let response = line(input)?;
                if response == format!("ILDEC DONE {submitted}") {
                    ensure!(
                        canceled || (preview && decoded),
                        "Missing native completion evidence"
                    );
                    return Ok(());
                }
                ensure!(!canceled, "Unexpected native output after cancellation");
                if !preview && response.starts_with(&format!("gpu_preview_check=passed, presented={submitted},")) {
                    preview=true;
                } else if !decoded && response.starts_with(&format!("nvdec_check=passed, submitted={submitted}, decoded={submitted}, displayed={submitted},")) {
                    decoded=true;
                } else {
                    anyhow::bail!("Unexpected decoder completion response: {response:?}");
                }
                println!("{response}");
            }
            anyhow::bail!("Missing decoder IPC completion");
        }
    }
    Ok(())
}

enum ChildProcess {
    Standard(Child),
    Restricted(RestrictedDecoderChild),
}
impl ChildProcess {
    fn id(&self) -> u32 {
        match self {
            Self::Standard(child) => child.id(),
            Self::Restricted(child) => child.id(),
        }
    }
    fn try_wait(&mut self) -> Result<Option<ExitStatus>> {
        match self {
            Self::Standard(child) => Ok(child.try_wait()?),
            Self::Restricted(child) => child.try_wait(),
        }
    }
}

struct Session {
    child: ChildProcess,
    job: DecoderJob,
    send: Option<SyncSender<Request>>,
    receive: Receiver<Result<()>>,
    io_worker: Option<JoinHandle<()>>,
    reaped: bool,
}

impl Session {
    fn spawn(restricted: bool) -> Result<Self> {
        let (child, job, mut output, input): (
            ChildProcess,
            DecoderJob,
            Box<dyn Write + Send>,
            Box<dyn Read + Send>,
        ) = if restricted {
            let (child, job, input, output) = RestrictedDecoderChild::spawn()?;
            (
                ChildProcess::Restricted(child),
                job,
                Box::new(input),
                Box::new(output),
            )
        } else {
            let mut child = Command::new(std::env::current_exe()?)
                .arg("desktop-decoder-worker")
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::inherit())
                .spawn()
                .context("Start owned decoder child")?;
            let job = match DecoderJob::attach(&child) {
                Ok(job) => job,
                Err(error) => {
                    child
                        .kill()
                        .context("Stop child after job attachment failure")?;
                    child.wait()?;
                    return Err(error.context("Attach private decoder job"));
                }
            };
            let input = child.stdin.take().context("Missing child input pipe")?;
            let output = child.stdout.take().context("Missing child output pipe")?;
            (
                ChildProcess::Standard(child),
                job,
                Box::new(input),
                Box::new(output),
            )
        };
        let mut input = BufReader::new(input);
        let (send, requests) = mpsc::sync_channel(1);
        let (responses, receive) = mpsc::sync_channel(1);
        let mut session = Self {
            child,
            job,
            send: Some(send),
            receive,
            io_worker: None,
            reaped: false,
        };
        session.io_worker = Some(
            thread::Builder::new()
                .name("intralink-decoder-ipc".into())
                .spawn(move || {
                    while let Ok(request) = requests.recv() {
                        let finished = matches!(request, Request::Finish { .. });
                        let response = exchange(&mut output, &mut input, request);
                        let failed = response.is_err();
                        if responses.send(response).is_err() || failed || finished {
                            break;
                        }
                    }
                })?,
        );
        println!(
            "decoder_process_started, pid={}, private_job=true",
            session.child.id()
        );
        Ok(session)
    }

    fn call(&mut self, request: Request, limit: Duration) -> Result<()> {
        self.send
            .as_ref()
            .context("Decoder IPC closed")?
            .try_send(request)
            .map_err(|_| anyhow::anyhow!("Decoder IPC queue unavailable"))?;
        let result = match self.receive.recv_timeout(limit) {
            Ok(result) => result,
            Err(error) => Err(anyhow::Error::new(error).context("Decoder IPC response deadline")),
        };
        if let Err(error) = result {
            // EOF can precede the process signal. Briefly poll the exact
            // owned handle to retain the original native exit status.
            let started = Instant::now();
            while started.elapsed() < Duration::from_millis(100) {
                if let Some(status) = self.child.try_wait()? {
                    self.reaped = true;
                    return Err(error.context(format!("Decoder child exited: {status}")));
                }
                thread::sleep(Duration::from_millis(5));
            }
            return Err(error);
        }
        Ok(())
    }

    fn wait_exit(&mut self) -> Result<std::process::ExitStatus> {
        let start = Instant::now();
        loop {
            if let Some(status) = self.child.try_wait()? {
                self.reaped = true;
                return Ok(status);
            }
            ensure!(start.elapsed() < EXIT_LIMIT, "Decoder child exit deadline");
            thread::sleep(Duration::from_millis(5));
        }
    }

    fn join_io(&mut self) -> Result<()> {
        self.send.take();
        if let Some(worker) = self.io_worker.take() {
            // A child exit closes the owned pipe handles. The thread must
            // terminate within the same bounded shutdown budget.
            let started = Instant::now();
            while !worker.is_finished() {
                ensure!(
                    started.elapsed() < EXIT_LIMIT,
                    "Decoder IPC thread exit deadline"
                );
                thread::sleep(Duration::from_millis(5));
            }
            worker
                .join()
                .map_err(|_| anyhow::anyhow!("Decoder IPC worker panicked"))?;
        }
        Ok(())
    }

    fn finish(mut self, submitted: usize, canceled: bool) -> Result<()> {
        self.call(
            Request::Finish {
                submitted,
                canceled,
            },
            EXIT_LIMIT,
        )?;
        let status = self.wait_exit()?;
        self.join_io()?;
        self.job.close()?;
        ensure!(status.success(), "Decoder process failed: {status}");
        println!(
            "decoder_process_exit=passed, pid={}, exit=0, ipc_joined=true",
            self.child.id()
        );
        Ok(())
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        if !self.reaped {
            // Explicit termination on any IPC failure/timeout. Kill-on-close
            // also handles abnormal termination of the parent itself.
            if let Err(error) = self.job.terminate() {
                eprintln!("Decoder termination: {error:#}");
            }
            if let Err(error) = self.job.close() {
                eprintln!("Decoder job release: {error:#}");
            }
            if let Err(error) = self.wait_exit() {
                eprintln!("Decoder process reap: {error:#}");
            }
        }
        if let Err(error) = self.join_io() {
            eprintln!("Decoder pipe cleanup: {error:#}");
        }
    }
}

pub(super) fn run(
    start: Start,
    mut jobs: async_mpsc::Receiver<DecodeJob>,
    restricted: bool,
) -> Result<usize> {
    let mut session = Session::spawn(restricted)?;
    session.call(Request::Start(start, restricted), START_LIMIT)?;
    println!(
        "decoder_process_policy=verified, restricted_token={restricted}, commit_limit_mib=2048, image_load_policy=true, extension_points_disabled=true, child_creation_disabled=true"
    );
    let mut samples = Vec::with_capacity(usize::from(start.samples));
    let mut first = None::<Instant>;
    let mut last = None::<Instant>;
    let mut bytes = Vec::new();
    for sequence in 1..=u32::from(start.samples) {
        let Some((packet, done)) = jobs.blocking_recv() else {
            break;
        };
        ensure!(
            packet.sequence() == sequence,
            "Decoder origin sequence mismatch"
        );
        h264::wrap(sequence, packet.keyframe(), packet.bytes(), &mut bytes)
            .map_err(|e| anyhow::anyhow!("Wrap trusted decoder IPC packet: {e:?}"))?;
        // Only opaque native-generated tokens can reach this sender; the
        // parent verified all received wire bytes before enqueuing the job.
        session.call(
            Request::Frame {
                bytes: std::mem::take(&mut bytes),
                sequence,
            },
            FRAME_LIMIT,
        )?;
        let submitted = Instant::now();
        first.get_or_insert(submitted);
        last = Some(submitted);
        samples.push(packet.elapsed_since_demand());
        if done.send(()).is_err() {
            break;
        }
    }
    let submitted = samples.len();
    let canceled = submitted != usize::from(start.samples);
    session.finish(submitted, canceled)?;
    if canceled {
        println!(
            "wgc_stream_viewer=cancelled, submitted={submitted}, worker_resources_released=true"
        );
        return Ok(submitted);
    }
    let steady = match (first, last) {
        (Some(first), Some(last)) if last > first => {
            (submitted - 1) as f64 / last.duration_since(first).as_secs_f64()
        }
        _ => 0.0,
    };
    samples.sort_unstable();
    let p50 = samples[(submitted - 1) / 2].as_secs_f64() * 1000.0;
    let p95 = samples[(submitted * 95).div_ceil(100) - 1].as_secs_f64() * 1000.0;
    println!(
        "wgc_stream_viewer=passed, submitted={submitted}, demand_to_decode_submit_p50_ms={p50:.3}, p95_ms={p95:.3}, steady_decode_submit_fps={steady:.2}, worker_resources_released=true"
    );
    Ok(submitted)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn response_lines_are_bounded_and_exact() {
        assert_eq!(
            line(&mut Cursor::new(b"ILDEC ACK 1\r\n")).unwrap(),
            "ILDEC ACK 1"
        );
        assert!(line(&mut Cursor::new(vec![b'x'; MAX_LINE + 1])).is_err());
        assert!(line(&mut Cursor::new(b"ILDEC ACK 1")).is_err());
        assert!(expect(&mut Cursor::new(b"ILDEC ACK 2\n"), "ILDEC ACK 1").is_err());
    }
}
