//! Exercise the real private child and Windows job without initializing CUDA.
#![cfg(all(target_os = "windows", feature = "nvenc-preview"))]

use std::{
    io::{Read, Write},
    process::{Child, Command, ExitStatus, Stdio},
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};

use intralink_core::h264::Profile;
use intralink_d3d11::{DecoderJob, FIXTURE_BITRATE};

fn worker() -> Child {
    Command::new(env!("CARGO_BIN_EXE_intralink"))
        .arg("desktop-decoder-worker")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("Start private decoder child")
}

fn wait(child: &mut Child) -> ExitStatus {
    let started = Instant::now();
    loop {
        if let Some(status) = child.try_wait().expect("Poll owned child") {
            return status;
        }
        if started.elapsed() > Duration::from_secs(5) {
            let _ = child.kill();
            let _ = child.wait();
            panic!("Owned decoder child exit deadline");
        }
        thread::sleep(Duration::from_millis(10));
    }
}

fn bootstrap() -> [u8; 24] {
    let mut bytes = [0; 24];
    bytes[..8].copy_from_slice(b"ILDEC001");
    bytes[8..20].copy_from_slice(&Profile::new(540, 60, 30, FIXTURE_BITRATE).unwrap().encode());
    bytes[20..22].copy_from_slice(&60u16.to_be_bytes());
    bytes[22] = 255;
    bytes
}

#[test]
fn invalid_bootstrap_is_rejected_before_driver_initialization() {
    let mut child = worker();
    let mut job = DecoderJob::attach(&child).expect("Attach owned decoder job");
    let mut invalid = bootstrap();
    invalid[23] = 1;
    child.stdin.take().unwrap().write_all(&invalid).unwrap();
    assert!(!wait(&mut child).success());
    let mut stdout = String::new();
    child
        .stdout
        .take()
        .unwrap()
        .read_to_string(&mut stdout)
        .unwrap();
    let mut stderr = String::new();
    child
        .stderr
        .take()
        .unwrap()
        .read_to_string(&mut stderr)
        .unwrap();
    assert!(!stdout.contains("ILDEC READY"));
    assert!(stderr.contains("Invalid decoder IPC bootstrap"), "{stderr}");
    job.close().unwrap();
}

#[test]
fn kill_on_close_releases_a_real_child_blocked_on_ipc() {
    let mut child = worker();
    let mut job = DecoderJob::attach(&child).expect("Attach owned decoder job");
    // Keep the input open without bootstrap. The child cannot initialize
    // the driver and its stdout reader cannot receive a READY response.
    let input = child.stdin.take().unwrap();
    let mut output = child.stdout.take().unwrap();
    let (done, response) = mpsc::sync_channel(1);
    let reader = thread::spawn(move || {
        let mut byte = [0];
        let result = output.read(&mut byte);
        let _ = done.send(result);
    });
    assert!(matches!(
        response.recv_timeout(Duration::from_millis(100)),
        Err(mpsc::RecvTimeoutError::Timeout)
    ));
    assert!(child.try_wait().unwrap().is_none());
    job.close().expect("Kill only the private job");
    // KILL_ON_JOB_CLOSE may produce exit code zero. The decisive evidence
    // is that the process exited while stdin was still held open and no
    // READY/DONE was emitted. Normal supervisor success requires DONE too.
    let _status = wait(&mut child);
    drop(input);
    assert_eq!(
        response
            .recv_timeout(Duration::from_secs(2))
            .unwrap()
            .unwrap(),
        0
    );
    reader.join().unwrap();
}

#[test]
fn private_worker_rejects_vault_arguments() {
    for command in [
        "desktop-decoder-worker",
        "desktop-restricted-decoder-worker",
        "desktop-decoder-token-check",
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_intralink"))
            .args(["--vault", "unused-test-vault", command])
            .output()
            .unwrap();
        assert!(!output.status.success());
        assert!(String::from_utf8_lossy(&output.stderr).contains("cannot access a vault"));
    }
}

#[test]
fn explicit_job_termination_uses_failure_exit_status() {
    let mut child = worker();
    let mut job = DecoderJob::attach(&child).expect("Attach owned decoder job");
    let input = child.stdin.take().unwrap();
    assert!(child.try_wait().unwrap().is_none());
    job.terminate().expect("Terminate only owned decoder job");
    assert_eq!(wait(&mut child).code(), Some(1));
    drop(input);
    job.close().unwrap();
}

#[test]
fn restricted_decoder_requires_explicit_process_isolation() {
    let output = Command::new(env!("CARGO_BIN_EXE_intralink"))
        .args(["desktop-wgc-live-check", "--restricted-decoder"])
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("--isolated-decoder"));
}
