//! Dedicated session timer worker. The native handle never leaves its owner
//! thread. A single bounded request/result pair is reused without busy spin.
use anyhow::{Context, Result};
use intralink_d3d11::PreciseTimer;
use intralink_transport::desktop::PacerWait;
use std::{
    thread::{self, JoinHandle},
    time::Duration,
};
use tokio::sync::mpsc;

pub(super) struct TimerWorker {
    request: Option<mpsc::Sender<Duration>>,
    response: Option<mpsc::Receiver<Result<()>>>,
    worker: Option<JoinHandle<Result<()>>>,
}

impl TimerWorker {
    pub(super) fn start() -> Result<Self> {
        let (request, mut requests) = mpsc::channel::<Duration>(1);
        let (responses, response) = mpsc::channel::<Result<()>>(1);
        let worker = thread::Builder::new()
            .name("intralink-precise-timer".into())
            .spawn(move || {
                let mut timer = PreciseTimer::new()?;
                // First response is readiness. No fallback on unavailable timer.
                if responses.blocking_send(Ok(())).is_ok() {
                    while let Some(delay) = requests.blocking_recv() {
                        let result = timer.wait(delay);
                        let failed = result.is_err();
                        if responses.blocking_send(result).is_err() || failed {
                            break;
                        }
                    }
                }
                timer.close()
            })?;
        Ok(Self {
            request: Some(request),
            response: Some(response),
            worker: Some(worker),
        })
    }

    pub(super) async fn ready(&mut self) -> Result<()> {
        self.response
            .as_mut()
            .context("Timer is closed")?
            .recv()
            .await
            .context("Timer startup failed")?
    }

    pub(super) fn close(&mut self) -> Result<()> {
        self.request.take();
        self.response.take();
        if let Some(worker) = self.worker.take() {
            worker
                .join()
                .map_err(|_| anyhow::anyhow!("Timer worker panicked"))??;
        }
        Ok(())
    }
}

impl PacerWait for TimerWorker {
    async fn wait(&mut self, delay: Duration) -> Result<()> {
        if delay.is_zero() {
            return Ok(());
        }
        self.request
            .as_ref()
            .context("Timer is closed")?
            .send(delay)
            .await
            .map_err(|_| anyhow::anyhow!("Timer request channel closed"))?;
        self.response
            .as_mut()
            .context("Timer is closed")?
            .recv()
            .await
            .context("Timer response channel closed")?
    }
}

impl Drop for TimerWorker {
    fn drop(&mut self) {
        if let Err(error) = self.close() {
            eprintln!("Timer worker shutdown: {error:#}");
        }
    }
}
