use std::{collections::VecDeque, sync::Arc, time::Duration};

use tokio::sync::{mpsc, Mutex};

use crate::{
    prover::Prover,
    store::{ReturnStatus, ReturnStore},
};

#[derive(Clone)]
pub struct QueueHandle {
    tx: mpsc::Sender<QueueCommand>,
    state: Arc<Mutex<QueueState>>,
}

#[derive(Default)]
struct QueueState {
    pending: VecDeque<String>,
    active_batch: Option<String>,
}

enum QueueCommand {
    Enqueue(String),
    Flush,
}

impl QueueHandle {
    pub async fn enqueue(&self, id: String) -> Result<(), QueueError> {
        self.tx
            .send(QueueCommand::Enqueue(id))
            .await
            .map_err(|_| QueueError::Closed)
    }

    pub async fn flush(&self) -> Result<(), QueueError> {
        self.tx
            .send(QueueCommand::Flush)
            .await
            .map_err(|_| QueueError::Closed)
    }

    pub async fn depth(&self) -> usize {
        self.state.lock().await.pending.len()
    }

    pub async fn active_batch(&self) -> Option<String> {
        self.state.lock().await.active_batch.clone()
    }
}

pub fn spawn(
    store: ReturnStore,
    prover: Prover,
    batch_target: usize,
    max_wait: Duration,
) -> QueueHandle {
    let (tx, rx) = mpsc::channel(128);
    let state = Arc::new(Mutex::new(QueueState::default()));
    let worker_state = state.clone();
    tokio::spawn(worker_loop(
        rx,
        worker_state,
        store,
        prover,
        batch_target.max(1),
        max_wait,
    ));
    QueueHandle { tx, state }
}

async fn worker_loop(
    mut rx: mpsc::Receiver<QueueCommand>,
    state: Arc<Mutex<QueueState>>,
    store: ReturnStore,
    prover: Prover,
    batch_target: usize,
    max_wait: Duration,
) {
    loop {
        let Some(first) = rx.recv().await else {
            return;
        };
        match first {
            QueueCommand::Enqueue(id) => state.lock().await.pending.push_back(id),
            QueueCommand::Flush => {}
        }

        let delay = tokio::time::sleep(max_wait);
        tokio::pin!(delay);
        loop {
            if state.lock().await.pending.len() >= batch_target {
                break;
            }
            tokio::select! {
                cmd = rx.recv() => {
                    let Some(cmd) = cmd else { break; };
                    match cmd {
                        QueueCommand::Enqueue(id) => state.lock().await.pending.push_back(id),
                        QueueCommand::Flush => break,
                    }
                }
                _ = &mut delay => break,
            }
        }

        let ids = {
            let mut guard = state.lock().await;
            let take = guard.pending.len().min(batch_target);
            (0..take)
                .filter_map(|_| guard.pending.pop_front())
                .collect::<Vec<_>>()
        };
        if ids.is_empty() {
            continue;
        }

        prove_single_flight(ids, &state, &store, &prover).await;
    }
}

async fn prove_single_flight(
    ids: Vec<String>,
    state: &Arc<Mutex<QueueState>>,
    store: &ReturnStore,
    prover: &Prover,
) {
    let batch_id = batch_id(&ids);
    state.lock().await.active_batch = Some(batch_id.clone());

    for id in &ids {
        let _ = store.update_status(id, ReturnStatus::Proving, Some(batch_id.clone()), None);
    }

    // R0/R1 service intake accepts one fully assembled guest input per return.
    // Live multi-burn batch assembly is the S1 extension tracked in 07 R1.
    for id in ids {
        let Some(record) = store.get(&id) else {
            continue;
        };
        match prover
            .prove(batch_id.clone(), record.wire_input.clone())
            .await
        {
            Ok(_) => {
                let _ =
                    store.update_status(&id, ReturnStatus::Proven, Some(batch_id.clone()), None);
            }
            Err(err) => {
                let _ = store.update_status(
                    &id,
                    ReturnStatus::Failed,
                    Some(batch_id.clone()),
                    Some(err.to_string()),
                );
            }
        }
    }
    state.lock().await.active_batch = None;
}

fn batch_id(ids: &[String]) -> String {
    let mut hasher = sha2::Sha256::new();
    use sha2::Digest;
    for id in ids {
        hasher.update(id.as_bytes());
    }
    format!("0x{}", hex::encode(hasher.finalize()))
}

#[derive(Debug, thiserror::Error)]
pub enum QueueError {
    #[error("queue worker is closed")]
    Closed,
}
