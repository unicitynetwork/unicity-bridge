use std::{
    collections::HashMap,
    sync::{Arc, RwLock},
    time::{SystemTime, UNIX_EPOCH},
};

use bridge_return_core::PublicValues;
use serde::{Deserialize, Serialize};

#[derive(Clone, Default)]
pub struct ReturnStore {
    inner: Arc<RwLock<StoreInner>>,
}

#[derive(Default)]
struct StoreInner {
    by_id: HashMap<String, ReturnRecord>,
    by_nullifier: HashMap<String, String>,
    batches: HashMap<String, BatchBundle>,
}

/// A proven batch's published on-chain bundle (§B4 `/batches/:id`) — anyone can
/// submit `publicValues`+`proofBytes` to the vault's `fulfillBatch`. `settle_txid`
/// fills once S4 lands it.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BatchBundle {
    pub batch_id: String,
    pub mode: String,
    pub vkey: Option<String>,
    pub public_values: String,
    pub proof_bytes: String,
    pub settle_txid: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReturnStatus {
    Queued,
    Proving,
    Proven,
    Submitted,
    Settled,
    Failed,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorKind {
    PrecheckRejected,
    ProvingFailed,
    SubmissionFailed,
    ChainRejected,
    ServiceUnavailable,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ReturnFailure {
    pub kind: ErrorKind,
    pub message: String,
    pub recoverable: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReturnEvent {
    pub at_ms: u128,
    pub status: ReturnStatus,
    pub code: String,
    pub message: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReturnRecord {
    pub return_id: String,
    pub nullifier: String,
    pub status: ReturnStatus,
    pub terminal: bool,
    pub success: Option<bool>,
    pub progress: u8,
    pub message: String,
    pub next_poll_ms: u64,
    pub batch_id: Option<String>,
    /// `fulfillBatch` txid once S4 settles (denormalized from the batch for the wallet).
    pub settle_txid: Option<String>,
    pub failure: Option<ReturnFailure>,
    pub events: Vec<ReturnEvent>,
    pub batch_size: u32,
    pub total_amount: String,
    pub public_values_digest: String,
    pub public_values: PublicValuesHex,
    #[serde(skip)]
    pub wire_input: Vec<u8>,
    pub created_at_ms: u128,
    pub updated_at_ms: u128,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PublicValuesHex {
    pub domain_tag: String,
    pub config_hash: String,
    pub trust_base_hash: String,
    pub spent_root_old: String,
    pub spent_root_new: String,
    pub return_root: String,
    pub lock_ref_root: String,
    pub batch_size: u32,
    pub total_amount: String,
}

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("return not found: {0}")]
    NotFound(String),
}

impl ReturnStore {
    pub fn insert_or_get(&self, record: ReturnRecord) -> (ReturnRecord, bool) {
        let mut guard = self.inner.write().expect("store poisoned");
        if let Some(existing_id) = guard.by_nullifier.get(&record.nullifier) {
            if let Some(existing) = guard.by_id.get(existing_id) {
                return (existing.clone(), false);
            }
        }
        guard
            .by_nullifier
            .insert(record.nullifier.clone(), record.return_id.clone());
        guard.by_id.insert(record.return_id.clone(), record.clone());
        (record, true)
    }

    pub fn get(&self, id: &str) -> Option<ReturnRecord> {
        self.inner
            .read()
            .expect("store poisoned")
            .by_id
            .get(id)
            .cloned()
    }

    /// Lookup by nullifier (`0x…` hex, case-insensitive) — wallet idempotency.
    pub fn get_by_nullifier(&self, nullifier: &str) -> Option<ReturnRecord> {
        let key = nullifier.to_lowercase();
        let guard = self.inner.read().expect("store poisoned");
        guard
            .by_nullifier
            .get(&key)
            .and_then(|id| guard.by_id.get(id).cloned())
    }

    /// Persist a proven batch's published bundle (idempotent on batch id).
    pub fn put_batch(&self, bundle: BatchBundle) {
        self.inner
            .write()
            .expect("store poisoned")
            .batches
            .insert(bundle.batch_id.clone(), bundle);
    }

    pub fn get_batch(&self, batch_id: &str) -> Option<BatchBundle> {
        self.inner
            .read()
            .expect("store poisoned")
            .batches
            .get(batch_id)
            .cloned()
    }

    /// Record the settle txid on a batch (S4) — surfaced on `/batches/:id`.
    pub fn set_batch_settle_txid(&self, batch_id: &str, txid: String) {
        if let Some(b) = self
            .inner
            .write()
            .expect("store poisoned")
            .batches
            .get_mut(batch_id)
        {
            b.settle_txid = Some(txid);
        }
    }

    /// Record the settle txid on a return record (denormalized for the wallet).
    pub fn set_return_settle_txid(&self, id: &str, txid: String) {
        if let Some(r) = self
            .inner
            .write()
            .expect("store poisoned")
            .by_id
            .get_mut(id)
        {
            r.settle_txid = Some(txid);
        }
    }

    pub fn queued(&self) -> Vec<ReturnRecord> {
        self.inner
            .read()
            .expect("store poisoned")
            .by_id
            .values()
            .filter(|r| r.status == ReturnStatus::Queued)
            .cloned()
            .collect()
    }

    pub fn update_status(
        &self,
        id: &str,
        status: ReturnStatus,
        batch_id: Option<String>,
        failure: Option<ReturnFailure>,
    ) -> Result<ReturnRecord, StoreError> {
        let mut guard = self.inner.write().expect("store poisoned");
        let record = guard
            .by_id
            .get_mut(id)
            .ok_or_else(|| StoreError::NotFound(id.to_string()))?;
        record.status = status;
        if batch_id.is_some() {
            record.batch_id = batch_id;
        }
        record.failure = failure;
        apply_status_defaults(record);
        record.updated_at_ms = now_ms();
        record.events.push(ReturnEvent {
            at_ms: record.updated_at_ms,
            status: record.status.clone(),
            code: event_code(&record.status).to_string(),
            message: record.message.clone(),
        });
        Ok(record.clone())
    }
}

impl ReturnRecord {
    pub fn queued(
        return_id: String,
        nullifier: [u8; 32],
        public_values_digest: [u8; 32],
        public_values: PublicValues,
        wire_input: Vec<u8>,
    ) -> Self {
        let now = now_ms();
        Self {
            return_id,
            nullifier: hex32(&nullifier),
            status: ReturnStatus::Queued,
            terminal: false,
            success: None,
            progress: 20,
            message: "Burn prechecked and queued for the next proving batch".to_string(),
            next_poll_ms: 5_000,
            batch_id: None,
            settle_txid: None,
            failure: None,
            events: vec![
                ReturnEvent {
                    at_ms: now,
                    status: ReturnStatus::Queued,
                    code: "prechecked".to_string(),
                    message: "Burn passed S1 precheck".to_string(),
                },
                ReturnEvent {
                    at_ms: now,
                    status: ReturnStatus::Queued,
                    code: "queued".to_string(),
                    message: "Return is waiting for batch formation".to_string(),
                },
            ],
            batch_size: public_values.batch_size,
            total_amount: hex32(&public_values.total_amount),
            public_values_digest: hex32(&public_values_digest),
            public_values: PublicValuesHex::from(public_values),
            wire_input,
            created_at_ms: now,
            updated_at_ms: now,
        }
    }
}

impl ReturnFailure {
    pub fn recoverable(kind: ErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
            recoverable: true,
        }
    }
}

impl From<PublicValues> for PublicValuesHex {
    fn from(value: PublicValues) -> Self {
        Self {
            domain_tag: hex32(&value.domain_tag),
            config_hash: hex32(&value.config_hash),
            trust_base_hash: hex32(&value.trust_base_hash),
            spent_root_old: hex32(&value.spent_root_old),
            spent_root_new: hex32(&value.spent_root_new),
            return_root: hex32(&value.return_root),
            lock_ref_root: hex32(&value.lock_ref_root),
            batch_size: value.batch_size,
            total_amount: hex32(&value.total_amount),
        }
    }
}

fn hex32(value: &[u8; 32]) -> String {
    format!("0x{}", hex::encode(value))
}

fn apply_status_defaults(record: &mut ReturnRecord) {
    match record.status {
        ReturnStatus::Queued => {
            record.terminal = false;
            record.success = None;
            record.progress = 20;
            record.message = "Return is waiting for batch formation".to_string();
            record.next_poll_ms = 5_000;
        }
        ReturnStatus::Proving => {
            record.terminal = false;
            record.success = None;
            record.progress = 45;
            record.message = "Batch is being proven".to_string();
            record.next_poll_ms = 15_000;
        }
        ReturnStatus::Proven => {
            record.terminal = false;
            record.success = None;
            record.progress = 70;
            record.message = "Proof is ready and waiting for chain submission".to_string();
            record.next_poll_ms = 10_000;
        }
        ReturnStatus::Submitted => {
            record.terminal = false;
            record.success = None;
            record.progress = 85;
            record.message =
                "fulfillBatch transaction submitted; waiting for chain finality".to_string();
            record.next_poll_ms = 10_000;
        }
        ReturnStatus::Settled => {
            record.terminal = true;
            record.success = Some(true);
            record.progress = 100;
            record.message = "Return settled on the source chain".to_string();
            record.next_poll_ms = 0;
        }
        ReturnStatus::Failed => {
            record.terminal = true;
            record.success = Some(false);
            record.progress = 100;
            if let Some(failure) = &record.failure {
                record.message = failure.message.clone();
            } else {
                record.message = "Return failed".to_string();
            }
            record.next_poll_ms = 0;
        }
    }
}

fn event_code(status: &ReturnStatus) -> &'static str {
    match status {
        ReturnStatus::Queued => "queued",
        ReturnStatus::Proving => "proving_started",
        ReturnStatus::Proven => "proof_ready",
        ReturnStatus::Submitted => "tx_submitted",
        ReturnStatus::Settled => "settled",
        ReturnStatus::Failed => "failed",
    }
}

fn now_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time before unix epoch")
        .as_millis()
}
