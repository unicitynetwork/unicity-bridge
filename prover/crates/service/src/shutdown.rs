//! Stopping the service in order. A drain lets the batch in flight run to its end (settled,
//! or failed as any batch fails), starts no other, and then lets the service exit, so a
//! restart costs neither a proof nor an attempt. SIGTERM starts one; a second SIGTERM, a
//! SIGINT or the drain timeout abandons the proof instead, and the batch goes back in the
//! queue without charging its members an attempt. Only a crash still costs one.

use std::{sync::Arc, time::Duration};

use tokio::{
    signal::unix::{signal, SignalKind},
    sync::{mpsc, watch},
    task::JoinHandle,
};

use crate::{clock::Clock, store::ReturnStore};

/// Whether the service is draining: the orchestrator starts no batch once it is, and
/// `/health` says so.
#[derive(Clone)]
pub struct Drain(Arc<watch::Sender<bool>>);

impl Default for Drain {
    fn default() -> Self {
        Self(Arc::new(watch::Sender::new(false)))
    }
}

impl Drain {
    pub fn start(&self) {
        self.0.send_replace(true);
    }

    pub fn is_draining(&self) -> bool {
        *self.0.borrow()
    }

    /// Resolves once the drain has started.
    pub async fn started(&self) {
        // The sender lives in `self`, so the channel stays open while this waits.
        let _ = self.0.subscribe().wait_for(|draining| *draining).await;
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StopSignal {
    /// SIGTERM: drain, or abandon the proof if a drain is already under way.
    Terminate,
    /// SIGINT (Ctrl-C): abandon the proof at once.
    Interrupt,
}

#[derive(Debug, Eq, PartialEq)]
pub enum Stopped {
    /// The batch in flight ran to its end and no other started.
    Drained,
    /// The orchestrator was stopped mid-batch, and a batch it was proving is back in the queue.
    Abandoned,
}

/// SIGTERM and SIGINT as they arrive. The handlers are installed before this returns, so
/// from then on neither signal ends the process by its default action.
pub fn stop_signals() -> mpsc::Receiver<StopSignal> {
    let mut terminate = signal(SignalKind::terminate()).expect("listen for SIGTERM");
    let mut interrupt = signal(SignalKind::interrupt()).expect("listen for SIGINT");
    let (tx, rx) = mpsc::channel(4);
    tokio::spawn(async move {
        loop {
            let signal = tokio::select! {
                Some(()) = terminate.recv() => StopSignal::Terminate,
                Some(()) = interrupt.recv() => StopSignal::Interrupt,
                else => return,
            };
            if tx.send(signal).await.is_err() {
                return;
            }
        }
    });
    rx
}

/// Wait for a stop signal and see it through; returns once the service may exit. When it
/// returns [`Stopped::Abandoned`] the orchestrator is gone and its proving batch re-queued,
/// and the caller kills whatever the proof left running.
pub async fn stop(
    signals: &mut mpsc::Receiver<StopSignal>,
    drain: &Drain,
    orchestrator: &mut JoinHandle<()>,
    timeout: Duration,
    store: &ReturnStore,
    clock: &Clock,
) -> Stopped {
    if next(signals).await == StopSignal::Terminate {
        drain.start();
        tracing::info!(
            timeout_secs = timeout.as_secs(),
            "draining: the batch in flight runs to its end and no other starts; \
             SIGTERM again or SIGINT abandons it",
        );
        tokio::select! {
            _ = &mut *orchestrator => return Stopped::Drained,
            _ = next(signals) => {}
            _ = expiry(timeout) => {
                tracing::warn!("the drain ran past {}s", timeout.as_secs());
            }
        }
    }
    orchestrator.abort();
    match (&mut *orchestrator).await {
        // The orchestrator had died on its own: that is a crash, which recovery charges.
        Err(e) if e.is_panic() => {}
        _ => {
            for batch_id in store.stop_proving(clock.now_ms()) {
                tracing::info!(
                    %batch_id,
                    "proof abandoned; the batch is proven again at start, without an attempt",
                );
            }
        }
    }
    Stopped::Abandoned
}

async fn next(signals: &mut mpsc::Receiver<StopSignal>) -> StopSignal {
    match signals.recv().await {
        Some(signal) => signal,
        None => std::future::pending().await,
    }
}

async fn expiry(timeout: Duration) {
    match timeout {
        Duration::ZERO => std::future::pending().await,
        limit => tokio::time::sleep(limit).await,
    }
}
