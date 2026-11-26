//! Foreground view-only QUIC v8 and isolated v9 H.264 diagnostic. Shared bounded capture/receipt engine.
use std::{net::SocketAddr, time::Duration};

use anyhow::{Context, Result, bail, ensure};
use intralink_core::desktop::{
    ALPN, CONFIG_BYTES, CodecMode, CreditWindow, MAX_IN_FLIGHT_RECORDS, MAX_SECONDS, NO_UPDATE,
    REPEAT, REQUEST_BYTES, Record, VideoConfig, ViewPriority, ack_with_priority, frame_header,
    parse_length, request,
};
use intralink_core::{h264, pacing::CapturePacer, quality::ReceiptPressure};
use iroh::{Endpoint, EndpointAddr, EndpointId, SecretKey, endpoint::Connection};
use tokio::{
    sync::{mpsc, oneshot},
    time::{Instant, sleep, timeout},
};

use crate::{HANDSHAKE_LIMIT, bind_for_protocol, direct_candidates};

const REQUEST_LIMIT: Duration = Duration::from_secs(5);
const IO_LIMIT: Duration = Duration::from_secs(5);
const ACCEPT_LIMIT: Duration = Duration::from_secs(120);

/// A demand for exactly one frame, with ownership of a recyclable media allocation.
pub struct CaptureRequest {
    pub buffer: Vec<u8>,
    pub reply: oneshot::Sender<Result<Capture>>,
    /// Local transport observation, never a new remotely supplied command.
    pub receipt_pressure: ReceiptPressure,
}

pub enum Capture {
    Frame(Vec<u8>),
    Repeat(Vec<u8>),
    /// No new observed source generation; never synthesize a Repeat.
    NoUpdate(Vec<u8>),
    End,
}

pub struct VideoFrame {
    pub config: VideoConfig,
    pub payload: Vec<u8>,
    pub sequence: usize,
}

/// Received only by an explicitly isolated v9 diagnostic consumer.
/// The payload is a structurally checked access-unit envelope, not a trusted
/// or fully validated native decoder input.
pub struct H264Frame {
    pub profile: h264::Profile,
    pub payload: Vec<u8>,
    pub record_sequence: usize,
}

#[derive(Clone, Copy)]
enum SessionProfile {
    Legacy(VideoConfig),
    H264(h264::Profile),
}

#[derive(Clone, Copy)]
enum RequestedMode {
    Legacy(CodecMode),
    H264(h264::Profile),
}

impl RequestedMode {
    fn alpn(self) -> &'static [u8] {
        match self {
            Self::Legacy(_) => ALPN,
            Self::H264(_) => h264::ALPN,
        }
    }

    fn request(self) -> [u8; REQUEST_BYTES] {
        match self {
            Self::Legacy(mode) => request(mode),
            Self::H264(_) => *h264::REQUEST,
        }
    }
}

impl SessionProfile {
    fn requested(self) -> RequestedMode {
        match self {
            Self::Legacy(config) => RequestedMode::Legacy(config.mode()),
            Self::H264(profile) => RequestedMode::H264(profile),
        }
    }

    fn seconds(self) -> u64 {
        match self {
            Self::Legacy(config) => config.seconds(),
            Self::H264(profile) => profile.seconds(),
        }
    }

    fn max_frames(self) -> usize {
        match self {
            Self::Legacy(config) => config.max_frames(),
            Self::H264(profile) => profile.max_frames(),
        }
    }

    fn pacer(self) -> CapturePacer {
        match self {
            Self::Legacy(config) => CapturePacer::new(config),
            Self::H264(profile) => CapturePacer::for_h264(profile),
        }
    }

    fn h264_state(self) -> Option<h264::StreamState> {
        match self {
            Self::Legacy(_) => None,
            Self::H264(profile) => Some(h264::StreamState::new(profile)),
        }
    }
}

struct MediaFrame {
    profile: SessionProfile,
    payload: Vec<u8>,
    sequence: usize,
}

fn validate_media(state: &mut Option<h264::StreamState>, bytes: &[u8]) -> Result<()> {
    if let Some(state) = state {
        state
            .accept(bytes)
            .map_err(|error| anyhow::anyhow!("Invalid H.264 access unit: {error:?}"))?;
    }
    Ok(())
}

fn require_loopback(endpoint: &Endpoint) -> Result<()> {
    let sockets = endpoint.bound_sockets();
    ensure!(
        !sockets.is_empty() && sockets.iter().all(|addr| addr.ip().is_loopback()),
        "H.264 diagnostic requires loopback-only endpoint"
    );
    Ok(())
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct Stats {
    pub frames: usize,
    pub repeats: usize,
    pub no_updates: usize,
    pub media_bytes: u64,
}

impl Stats {
    fn records(&self) -> usize {
        self.frames + self.repeats + self.no_updates
    }
}

/// A local, session-owned wait policy. The transport still enforces the
/// negotiated interval and fails closed if a policy returns too early.
pub trait PacerWait: Send {
    fn wait(&mut self, delay: Duration) -> impl std::future::Future<Output = Result<()>> + Send;
}

struct TokioWait;
impl PacerWait for TokioWait {
    async fn wait(&mut self, delay: Duration) -> Result<()> {
        sleep(delay).await;
        Ok(())
    }
}

pub async fn bind_view(secret: SecretKey, addr: SocketAddr) -> Result<Endpoint> {
    bind_for_protocol(secret, addr, Some(ALPN)).await
}

/// Separate experimental ALPN, never enabled by the normal desktop Host.
pub async fn bind_h264_diagnostic(secret: SecretKey, addr: SocketAddr) -> Result<Endpoint> {
    ensure!(
        addr.ip().is_loopback(),
        "H.264 diagnostic binds loopback only"
    );
    let endpoint = bind_for_protocol(secret, addr, Some(h264::ALPN)).await?;
    if let Err(error) = require_loopback(&endpoint) {
        endpoint.close().await;
        return Err(error);
    }
    Ok(endpoint)
}

/// The caller must maintain visible local consent and cancel by dropping this
/// future and closing the endpoint when the local user stops sharing.
/// No capture request is emitted before identity and protocol validation.
pub async fn serve_view(
    endpoint: &Endpoint,
    approved: EndpointId,
    config: VideoConfig,
    source: mpsc::Sender<CaptureRequest>,
) -> Result<Stats> {
    Box::pin(serve_media(
        endpoint,
        approved,
        SessionProfile::Legacy(config),
        source,
        &mut TokioWait,
    ))
    .await
}

/// No normal Host or remotely supplied configuration enables this path.
pub async fn serve_h264_diagnostic(
    endpoint: &Endpoint,
    approved: EndpointId,
    profile: h264::Profile,
    source: mpsc::Sender<CaptureRequest>,
) -> Result<Stats> {
    serve_h264_diagnostic_with_waiter(endpoint, approved, profile, source, &mut TokioWait).await
}

/// Optional explicit local timer policy for the isolated diagnostic.
pub async fn serve_h264_diagnostic_with_waiter<W: PacerWait>(
    endpoint: &Endpoint,
    approved: EndpointId,
    profile: h264::Profile,
    source: mpsc::Sender<CaptureRequest>,
    waiter: &mut W,
) -> Result<Stats> {
    require_loopback(endpoint)?;
    Box::pin(serve_media(
        endpoint,
        approved,
        SessionProfile::H264(profile),
        source,
        waiter,
    ))
    .await
}

async fn serve_media<W: PacerWait>(
    endpoint: &Endpoint,
    approved: EndpointId,
    config: SessionProfile,
    source: mpsc::Sender<CaptureRequest>,
    waiter: &mut W,
) -> Result<Stats> {
    let incoming = timeout(ACCEPT_LIMIT, endpoint.accept())
        .await
        .context("No incoming view connection before deadline")?
        .context("View endpoint closed")?;
    let connection = timeout(HANDSHAKE_LIMIT, async { incoming.await }).await??;
    if connection.remote_id() != approved {
        connection.close(1u32.into(), b"not authorized");
        bail!("Unapproved controller; desktop request rejected");
    }
    let result = async {
        let requested = config.requested();
        ensure!(
            connection.alpn() == requested.alpn(),
            "Unexpected media ALPN"
        );
        let (mut tx, mut rx) = timeout(REQUEST_LIMIT, connection.accept_bi()).await??;
        let mut request = [0; REQUEST_BYTES];
        timeout(REQUEST_LIMIT, rx.read_exact(&mut request)).await??;
        ensure!(
            request == requested.request(),
            "Unsupported or unapproved media mode"
        );
        match config {
            SessionProfile::Legacy(legacy) => {
                timeout(IO_LIMIT, tx.write_all(&legacy.encode())).await??
            }
            SessionProfile::H264(profile) => {
                // The viewer explicitly requests the exact bounded profile,
                // before the Host is permitted to ask its local source.
                let mut offer = [0; h264::CONFIG_BYTES];
                timeout(REQUEST_LIMIT, rx.read_exact(&mut offer)).await??;
                ensure!(
                    h264::Profile::decode(offer) == Some(profile),
                    "H.264 profile offer not approved"
                );
                timeout(IO_LIMIT, tx.write_all(&profile.encode())).await??
            }
        }
        // Read receipts independently so background priority does not wait for
        // the credit window to fill. The queue and both futures are session-local.
        let (receipt_tx, mut receipt_rx) = mpsc::channel::<[u8; 4]>(MAX_IN_FLIGHT_RECORDS);
        let read_receipts = async move {
            loop {
                let mut bytes = [0; 4];
                if rx.read_exact(&mut bytes).await.is_err() || receipt_tx.send(bytes).await.is_err()
                {
                    break;
                }
            }
        };
        let stream = async {
            let deadline = Instant::now() + Duration::from_secs(config.seconds());
            let mut pacer = config.pacer();
            let mut media_state = config.h264_state();
            let mut stats = Stats::default();
            let mut buffer = Vec::new();
            let mut credit = CreditWindow::default();
            while stats.records() < config.max_frames() && Instant::now() < deadline {
                while let Ok(received) = receipt_rx.try_recv() {
                    ensure!(credit.acknowledge(received), "Invalid frame acknowledgment");
                }
                let receipt_pressure =
                    ReceiptPressure::from_backlog(credit.outstanding(), credit.priority());
                if !credit.has_credit() {
                    let remaining = deadline.saturating_duration_since(Instant::now());
                    if remaining.is_zero() {
                        break;
                    }
                    // No further capture while the bounded receipt window is full.
                    // A deadline or invalid receipt closes the session fail-closed.
                    let received = timeout(IO_LIMIT.min(remaining), receipt_rx.recv())
                        .await
                        .context("Desktop receipt window stalled")?
                        .context("Desktop receipt stream closed")?;
                    ensure!(credit.acknowledge(received), "Invalid frame acknowledgment");
                }
                let started = Instant::now();
                let remaining = deadline.saturating_duration_since(started);
                if remaining.is_zero() {
                    break;
                }
                let (reply, response) = oneshot::channel();
                let frame = timeout(remaining.min(IO_LIMIT), async {
                    source
                        .send(CaptureRequest {
                            buffer,
                            reply,
                            receipt_pressure,
                        })
                        .await
                        .map_err(|_| anyhow::anyhow!("Local capture source stopped"))?;
                    response.await.context("Local capture response dropped")?
                })
                .await
                .context("Local capture timed out")??;
                if Instant::now() >= deadline {
                    break;
                }
                let (header, data) = match frame {
                    Capture::End => break,
                    Capture::Repeat(data) => {
                        ensure!(stats.frames > 0 && data.is_empty(), "Invalid local repeat");
                        (REPEAT, data)
                    }
                    Capture::NoUpdate(data) => {
                        ensure!(
                            stats.frames > 0 && data.is_empty(),
                            "Invalid local no-update"
                        );
                        (NO_UPDATE, data)
                    }
                    Capture::Frame(data) => (
                        frame_header(data.len()).context("Invalid local frame length")?,
                        data,
                    ),
                };
                let observation = parse_length(header).context("Invalid local record")?;
                if matches!(observation, Record::Frame(_)) {
                    validate_media(&mut media_state, &data)?;
                }
                let remaining = deadline.saturating_duration_since(Instant::now());
                timeout(IO_LIMIT.min(remaining), async {
                    tx.write_all(&header).await?;
                    if matches!(observation, Record::Frame(_)) {
                        tx.write_all(&data).await?;
                    }
                    Ok::<(), anyhow::Error>(())
                })
                .await
                .context("Desktop transfer timed out")??;
                ensure!(credit.record_sent(), "Frame credit exhausted");
                match observation {
                    Record::Repeat => stats.repeats += 1,
                    Record::NoUpdate => stats.no_updates += 1,
                    Record::Frame(_) => {
                        stats.frames += 1;
                        stats.media_bytes += data.len() as u64;
                    }
                    Record::End => bail!("Unexpected local end"),
                }
                buffer = data;
                buffer.clear();
                // No-update can reduce polling demand, but does not verify unchanged pixels.
                let interval = pacer
                    .with_priority(!matches!(observation, Record::Frame(_)), credit.priority());
                let delay = interval
                    .saturating_sub(started.elapsed())
                    .min(deadline.saturating_duration_since(Instant::now()));
                if !delay.is_zero() {
                    let due = Instant::now() + delay;
                    timeout(IO_LIMIT, waiter.wait(delay))
                        .await
                        .context("Media pacing wait timed out")??;
                    ensure!(Instant::now() >= due, "Media pacing wait returned early");
                }
            }
            // No new capture after the deadline. Drain the remaining receipts
            // under a single cleanup budget rather than one timeout per record.
            // Keep the local capture channel alive until transport cleanup ends.
            timeout(IO_LIMIT, async {
                while credit.outstanding() > 0 {
                    let received = receipt_rx
                        .recv()
                        .await
                        .context("Desktop receipt stream closed")?;
                    ensure!(
                        credit.acknowledge(received),
                        "Invalid final frame acknowledgment"
                    );
                }
                Ok::<(), anyhow::Error>(())
            })
            .await
            .context("Desktop receipt drain timed out")??;
            timeout(IO_LIMIT, tx.write_all(&[0; 4])).await??;
            tx.finish()?;
            // All expected ACKs were consumed. Do not race the peer's QUIC close
            // against delivery of its send-side FIN. No further messages are handled.
            let _ = timeout(IO_LIMIT, connection.closed()).await;
            Ok::<Stats, anyhow::Error>(stats)
        };
        tokio::pin!(stream);
        tokio::select! {
            biased;
            result = &mut stream => result,
            () = read_receipts => stream.await,
        }
    }
    .await;
    if result.is_err() {
        connection.close(2u32.into(), b"view stopped");
    }
    result
}

/// Deliver each changed record in order to a bounded consumer. Repeat records
/// never trigger decoding. Receipt acknowledges acceptance by the consumer.
pub async fn view<F, Fut>(
    endpoint: &Endpoint,
    expected: EndpointId,
    candidates: &[SocketAddr],
    mode: CodecMode,
    display: F,
) -> Result<Stats>
where
    F: FnMut(VideoFrame) -> Fut,
    Fut: std::future::Future<Output = Result<()>>,
{
    view_with_buffers(endpoint, expected, candidates, mode, Vec::new, display).await
}

/// A consumer may return ownership of previously decoded payload allocations.
/// The provider must be nonblocking; an empty pool simply yields an empty Vec.
/// Ordered updates and credit acknowledgment semantics remain unchanged.
pub async fn view_with_buffers<F, Fut, B>(
    endpoint: &Endpoint,
    expected: EndpointId,
    candidates: &[SocketAddr],
    mode: CodecMode,
    take_buffer: B,
    display: F,
) -> Result<Stats>
where
    F: FnMut(VideoFrame) -> Fut,
    Fut: std::future::Future<Output = Result<()>>,
    B: FnMut() -> Vec<u8>,
{
    view_with_policy(
        endpoint,
        expected,
        candidates,
        mode,
        take_buffer,
        display,
        || ViewPriority::Foreground,
    )
    .await
}

/// Each viewer owns its own connection, even if multiple sessions share an endpoint.
/// Dropping a viewer future (local toggle/cancel) closes just this connection.
struct ViewConnectionGuard<'a>(Option<&'a Connection>);

impl ViewConnectionGuard<'_> {
    fn finish(mut self, success: bool) {
        if let Some(connection) = self.0.take() {
            if success {
                connection.close(0u32.into(), b"viewer closed");
            } else {
                connection.close(2u32.into(), b"view stopped");
            }
        }
    }
}

impl Drop for ViewConnectionGuard<'_> {
    fn drop(&mut self) {
        if let Some(connection) = self.0.take() {
            connection.close(2u32.into(), b"view cancelled");
        }
    }
}

/// Feedback is an efficiency hint only; it cannot elevate the host-approved FPS.
pub async fn view_with_policy<F, Fut, B, P>(
    endpoint: &Endpoint,
    expected: EndpointId,
    candidates: &[SocketAddr],
    mode: CodecMode,
    take_buffer: B,
    display: F,
    mut priority: P,
) -> Result<Stats>
where
    F: FnMut(VideoFrame) -> Fut,
    Fut: std::future::Future<Output = Result<()>>,
    B: FnMut() -> Vec<u8>,
    P: FnMut() -> ViewPriority,
{
    view_with_observations(
        endpoint,
        expected,
        candidates,
        mode,
        take_buffer,
        display,
        |_| priority(),
    )
    .await
}

/// Observe each accepted ordered media record without decoding no-update
/// records. The observer must be nonblocking and cannot grant authorization.
pub async fn view_with_observations<F, Fut, B, P>(
    endpoint: &Endpoint,
    expected: EndpointId,
    candidates: &[SocketAddr],
    mode: CodecMode,
    take_buffer: B,
    mut display: F,
    observe: P,
) -> Result<Stats>
where
    F: FnMut(VideoFrame) -> Fut,
    Fut: std::future::Future<Output = Result<()>>,
    B: FnMut() -> Vec<u8>,
    P: FnMut(Record) -> ViewPriority,
{
    Box::pin(view_media(
        endpoint,
        expected,
        candidates,
        RequestedMode::Legacy(mode),
        take_buffer,
        move |frame| {
            let result = match frame.profile {
                SessionProfile::Legacy(config) => Ok(display(VideoFrame {
                    config,
                    payload: frame.payload,
                    sequence: frame.sequence,
                })),
                SessionProfile::H264(_) => {
                    Err(anyhow::anyhow!("Unexpected H.264 frame in v8 viewer"))
                }
            };
            async move { result?.await }
        },
        observe,
    ))
    .await
}

/// Request exactly the caller's locally selected profile. No codec fallback,
/// discovery, remote addresses or normal desktop renderer in this diagnostic.
pub async fn view_h264_diagnostic<F, Fut>(
    endpoint: &Endpoint,
    expected: EndpointId,
    candidates: &[SocketAddr],
    profile: h264::Profile,
    mut display: F,
) -> Result<Stats>
where
    F: FnMut(H264Frame) -> Fut,
    Fut: std::future::Future<Output = Result<()>>,
{
    require_loopback(endpoint)?;
    ensure!(
        candidates.iter().all(|addr| addr.ip().is_loopback()),
        "H.264 diagnostic routes must be loopback"
    );
    Box::pin(view_media(
        endpoint,
        expected,
        candidates,
        RequestedMode::H264(profile),
        Vec::new,
        move |frame| {
            let result = match frame.profile {
                SessionProfile::H264(profile) => Ok(display(H264Frame {
                    profile,
                    payload: frame.payload,
                    record_sequence: frame.sequence,
                })),
                SessionProfile::Legacy(_) => {
                    Err(anyhow::anyhow!("Unexpected v8 frame in H.264 diagnostic"))
                }
            };
            async move { result?.await }
        },
        |_| ViewPriority::Foreground,
    ))
    .await
}

async fn view_media<F, Fut, B, P>(
    endpoint: &Endpoint,
    expected: EndpointId,
    candidates: &[SocketAddr],
    mode: RequestedMode,
    mut take_buffer: B,
    mut display: F,
    mut observe: P,
) -> Result<Stats>
where
    F: FnMut(MediaFrame) -> Fut,
    Fut: std::future::Future<Output = Result<()>>,
    B: FnMut() -> Vec<u8>,
    P: FnMut(Record) -> ViewPriority,
{
    let candidates = direct_candidates(candidates)?;
    let target = candidates
        .iter()
        .fold(EndpointAddr::new(expected), |target, addr| {
            target.with_ip_addr(*addr)
        });
    let connection = timeout(HANDSHAKE_LIMIT, endpoint.connect(target, mode.alpn())).await??;
    let guard = ViewConnectionGuard(Some(&connection));
    ensure!(
        connection.remote_id() == expected,
        "Remote identity mismatch"
    );
    let result = timeout(Duration::from_secs(MAX_SECONDS + 20), async {
        let (mut tx, mut rx) = timeout(REQUEST_LIMIT, connection.open_bi()).await??;
        timeout(REQUEST_LIMIT, tx.write_all(&mode.request())).await??;
        if let RequestedMode::H264(profile) = mode {
            timeout(REQUEST_LIMIT, tx.write_all(&profile.encode())).await??;
        }
        let config = match mode {
            RequestedMode::Legacy(requested) => {
                let mut header = [0; CONFIG_BYTES];
                timeout(IO_LIMIT, rx.read_exact(&mut header)).await??;
                let config =
                    VideoConfig::decode(header).context("Invalid desktop configuration")?;
                ensure!(
                    config.mode() == requested,
                    "Host selected a different media mode"
                );
                SessionProfile::Legacy(config)
            }
            RequestedMode::H264(expected) => {
                let mut header = [0; h264::CONFIG_BYTES];
                timeout(IO_LIMIT, rx.read_exact(&mut header)).await??;
                let profile =
                    h264::Profile::decode(header).context("Invalid H.264 configuration")?;
                ensure!(
                    profile == expected,
                    "Host H.264 profile differs from approved diagnostic profile"
                );
                SessionProfile::H264(profile)
            }
        };
        let mut media_state = config.h264_state();
        let mut stats = Stats::default();
        timeout(Duration::from_secs(config.seconds() + 10), async {
            loop {
                let mut header = [0; 4];
                timeout(IO_LIMIT, rx.read_exact(&mut header)).await??;
                let observation = parse_length(header).context("Oversized desktop record")?;
                match observation {
                    Record::End => {
                        timeout(IO_LIMIT, rx.read_to_end(0)).await??;
                        tx.finish()?;
                        return Ok::<Stats, anyhow::Error>(stats);
                    }
                    Record::Repeat => {
                        ensure!(
                            stats.frames > 0 && stats.records() < config.max_frames(),
                            "Invalid repeat record"
                        );
                        stats.repeats += 1;
                    }
                    Record::NoUpdate => {
                        ensure!(
                            stats.frames > 0 && stats.records() < config.max_frames(),
                            "Invalid no-update record"
                        );
                        stats.no_updates += 1;
                    }
                    Record::Frame(length) => {
                        ensure!(stats.records() < config.max_frames(), "Too many frames");
                        let mut data = take_buffer();
                        prepare_payload(&mut data, length)?;
                        timeout(IO_LIMIT, rx.read_exact(&mut data)).await??;
                        validate_media(&mut media_state, &data)?;
                        stats.frames += 1;
                        stats.media_bytes += length as u64;
                        timeout(
                            IO_LIMIT,
                            display(MediaFrame {
                                profile: config,
                                payload: data,
                                sequence: stats.records(),
                            }),
                        )
                        .await
                        .context("Local media consumer stalled")??;
                    }
                }
                let sequence = ack_with_priority(stats.records(), observe(observation))
                    .context("Frame sequence exhausted")?;
                timeout(IO_LIMIT, tx.write_all(&sequence)).await??;
            }
        })
        .await
        .context("Negotiated desktop deadline exceeded")?
    })
    .await;
    guard.finish(result.as_ref().is_ok_and(|r| r.is_ok()));
    result.context("Desktop session deadline exceeded")?
}

/// Keep accepted payload lengths bounded even if the local buffer provider
/// returns a larger allocation. Use exact reservation instead of geometric growth.
fn prepare_payload(data: &mut Vec<u8>, length: usize) -> Result<()> {
    ensure!(
        (1..=intralink_core::desktop::MAX_FRAME_BYTES).contains(&length),
        "Invalid payload size"
    );
    if data.capacity() > intralink_core::desktop::MAX_FRAME_BYTES {
        *data = Vec::new();
    }
    data.clear();
    data.try_reserve_exact(length)
        .context("Could not allocate desktop payload")?;
    data.resize(length, 0);
    Ok(())
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod h264_tests;
