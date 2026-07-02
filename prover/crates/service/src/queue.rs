use std::{collections::VecDeque, sync::Arc, time::Duration};

use bridge_return_guest::wire;
use tokio::sync::{mpsc, Mutex};

use crate::{
    prover::Prover,
    store::{BatchBundle, ErrorKind, ReturnFailure, ReturnStatus, ReturnStore},
    submitter::Submitter,
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
    submitter: Submitter,
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
        submitter,
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
    submitter: Submitter,
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

        prove_single_flight(ids, &state, &store, &prover, &submitter).await;
    }
}

async fn prove_single_flight(
    ids: Vec<String>,
    state: &Arc<Mutex<QueueState>>,
    store: &ReturnStore,
    prover: &Prover,
    submitter: &Submitter,
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
            Ok(bundle) => {
                // The wire input already carries this return's settlement leaf +
                // lock refs (decoded once at intake, api.rs); re-decode here so the
                // published bundle carries the plaintext fulfillBatch calldata too —
                // otherwise neither a self-settler nor the S4 submit command could
                // ever actually call fulfillBatch (only the proof would be public).
                let (leaves, lock_refs) = wire::decode_guest_input(&record.wire_input)
                    .map(|input| (input.return_leaves, input.sorted_lock_refs))
                    .unwrap_or_default();
                // Publish the bundle (self-settle source, §B4) before advancing.
                let batch_bundle = BatchBundle {
                    batch_id: batch_id.clone(),
                    mode: bundle.mode.clone(),
                    vkey: bundle.vkey_hash.clone(),
                    public_values: format!("0x{}", hex::encode(&bundle.public_values)),
                    proof_bytes: format!("0x{}", hex::encode(&bundle.proof_bytes)),
                    settle_txid: None,
                    leaves: leaves.into_iter().map(Into::into).collect(),
                    lock_refs: lock_refs.into_iter().map(Into::into).collect(),
                };
                store.put_batch(batch_bundle.clone());
                let _ =
                    store.update_status(&id, ReturnStatus::Proven, Some(batch_id.clone()), None);
                // S4: submit fulfillBatch → submitted → settled (or stop at proven
                // when no submitter is configured — the bundle is self-settleable).
                submit_batch(&submitter, store, &batch_id, &id, &batch_bundle).await;
            }
            Err(err) => {
                let _ = store.update_status(
                    &id,
                    ReturnStatus::Failed,
                    Some(batch_id.clone()),
                    Some(ReturnFailure::recoverable(
                        ErrorKind::ProvingFailed,
                        err.to_string(),
                    )),
                );
            }
        }
    }
    state.lock().await.active_batch = None;
}

/// Drive proven → submitted → settled via the configured {Submitter} (S4). With
/// no submitter the return stays `proven` — the published bundle is self-settleable
/// by anyone (the principal is never stuck; 06 §A1.2).
async fn submit_batch(
    submitter: &crate::submitter::Submitter,
    store: &ReturnStore,
    batch_id: &str,
    id: &str,
    bundle: &BatchBundle,
) {
    use crate::submitter::SubmitOutcome;
    match submitter.submit(bundle).await {
        SubmitOutcome::Skipped => {}
        SubmitOutcome::Submitted { txid } => {
            store.set_batch_settle_txid(batch_id, txid.clone());
            store.set_return_settle_txid(id, txid);
            let _ = store.update_status(id, ReturnStatus::Submitted, Some(batch_id.to_string()), None);
            let _ = store.update_status(id, ReturnStatus::Settled, Some(batch_id.to_string()), None);
        }
        SubmitOutcome::Failed { message } => {
            let _ = store.update_status(
                id,
                ReturnStatus::Failed,
                Some(batch_id.to_string()),
                Some(ReturnFailure::recoverable(ErrorKind::SubmissionFailed, message)),
            );
        }
    }
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
