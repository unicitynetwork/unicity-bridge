use std::{
    io,
    path::Path,
    sync::{Arc, Mutex},
};

use bridge_return_core::{PublicValues, ReturnLeaf, SourceLockRef};
use serde::{Deserialize, Serialize};
use tokio::sync::Notify;

use crate::{
    domain::{
        batch::Batch,
        burn::Burn,
        event::Event,
        ledger::{Health, Ledger},
        policy::{BatchPolicy, Decision},
        retry::RetryPolicy,
    },
    journal::Journal,
    prover::ProofBundle,
};

#[derive(Clone)]
pub struct ReturnStore {
    inner: Arc<Mutex<Inner>>,
    changed: Arc<Notify>,
}

struct Inner {
    ledger: Ledger,
    journal: Journal,
    retry: RetryPolicy,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Taken {
    Batch(Batch),
    WaitUntil(u128),
    Idle,
}

/// A proven batch's published on-chain bundle (§B4 `/batches/:id`) — anyone can
/// submit `publicValues`+`proofBytes`+`leaves`+`lockRefs` to the vault's
/// `fulfillBatch`. `settle_txid` fills once S4 lands it.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BatchBundle {
    pub batch_id: String,
    pub mode: String,
    pub vkey: Option<String>,
    pub public_values: String,
    pub proof_bytes: String,
    pub settle_txid: Option<String>,
    /// The `fulfillBatch` leaf calldata (nullifier/recipient/amount/fee/deadline) —
    /// plaintext witnesses for the amounts the proof's `return_root` commits to.
    #[serde(default)]
    pub leaves: Vec<LeafHex>,
    /// The `fulfillBatch` lock-ref calldata — plaintext witnesses for `lock_ref_root`.
    #[serde(default)]
    pub lock_refs: Vec<LockRefHex>,
}

/// Hex-encoded `ReturnLeaf` (07 §B4 self-settle calldata / S4 submit payload).
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LeafHex {
    pub nullifier: String,
    pub recipient: String,
    pub amount: String,
    pub fee_recipient: String,
    pub fee_amount: String,
    pub deadline: String,
}

/// Hex-encoded `SourceLockRef`.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LockRefHex {
    pub nonce: String,
    pub digest: String,
}

impl From<ReturnLeaf> for LeafHex {
    fn from(l: ReturnLeaf) -> Self {
        Self {
            nullifier: hex32(&l.nullifier),
            recipient: format!("0x{}", hex::encode(l.recipient)),
            amount: hex32(&l.amount),
            fee_recipient: format!("0x{}", hex::encode(l.fee_recipient)),
            fee_amount: hex32(&l.fee_amount),
            deadline: format!("0x{:x}", l.deadline),
        }
    }
}

impl From<SourceLockRef> for LockRefHex {
    fn from(r: SourceLockRef) -> Self {
        Self {
            nonce: format!("0x{:x}", r.nonce),
            digest: hex32(&r.digest),
        }
    }
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
    #[serde(default)]
    pub attempts: u32,
    #[serde(default)]
    pub not_before_ms: Option<u128>,
    #[serde(default)]
    pub queue_position: Option<usize>,
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

impl Default for ReturnStore {
    fn default() -> Self {
        Self::memory(RetryPolicy::default())
    }
}

impl ReturnStore {
    pub fn memory(retry: RetryPolicy) -> Self {
        Self::new(Ledger::default(), Journal::Memory, retry)
    }

    pub fn open(dir: &Path, retry: RetryPolicy, now_ms: u128) -> io::Result<Self> {
        let (mut journal, events) = Journal::open(dir)?;
        let mut ledger = Ledger::default();
        for event in events {
            ledger.apply(event, &retry);
        }
        for event in ledger.recover(now_ms) {
            journal.append(&event)?;
            ledger.apply(event, &retry);
        }
        journal.compact(&ledger)?;
        Ok(Self::new(ledger, journal, retry))
    }

    fn new(ledger: Ledger, journal: Journal, retry: RetryPolicy) -> Self {
        Self {
            inner: Arc::new(Mutex::new(Inner {
                ledger,
                journal,
                retry,
            })),
            changed: Arc::new(Notify::new()),
        }
    }

    pub fn accept(&self, burn: Burn, record: ReturnRecord, now_ms: u128) -> (ReturnRecord, bool) {
        let mut inner = self.inner.lock().expect("store poisoned");
        let known = inner
            .ledger
            .by_nullifier(&record.nullifier)
            .map(|r| (r.burn.id.clone(), r.record.failed_recoverably()));
        let id = match known {
            Some((id, false)) => return (inner.ledger.record(&id).expect("known"), false),
            Some((id, true)) => {
                inner.commit(Event::Requeued {
                    id: id.clone(),
                    at_ms: now_ms,
                });
                id
            }
            None => {
                let id = burn.id.clone();
                inner.commit(Event::Accepted { burn, record });
                id
            }
        };
        self.changed.notify_one();
        (inner.ledger.record(&id).expect("just committed"), true)
    }

    pub fn apply(&self, event: Event) {
        self.inner.lock().expect("store poisoned").commit(event);
    }

    pub fn take_next(&self, policy: &BatchPolicy, now_ms: u128, collect: bool) -> Taken {
        let mut inner = self.inner.lock().expect("store poisoned");
        match policy.next(&inner.ledger.pending(), now_ms, collect) {
            Decision::Batch(members) => {
                inner.commit(Event::BatchFormed {
                    members,
                    at_ms: now_ms,
                });
                Taken::Batch(inner.ledger.active_batch().cloned().expect("just formed"))
            }
            Decision::WaitUntil(at_ms) => Taken::WaitUntil(at_ms),
            Decision::Idle => Taken::Idle,
        }
    }

    pub fn read<T>(&self, f: impl FnOnce(&Ledger) -> T) -> T {
        f(&self.inner.lock().expect("store poisoned").ledger)
    }

    pub fn get(&self, id: &str) -> Option<ReturnRecord> {
        self.read(|ledger| ledger.record(id))
    }

    pub fn get_by_nullifier(&self, nullifier: &str) -> Option<ReturnRecord> {
        self.read(|ledger| {
            let id = ledger.by_nullifier(nullifier)?.burn.id.clone();
            ledger.record(&id)
        })
    }

    pub fn get_batch(&self, batch_id: &str) -> Option<BatchBundle> {
        self.read(|ledger| ledger.batch(batch_id)?.bundle.clone())
    }

    pub fn health(&self) -> Health {
        self.read(Ledger::health)
    }

    pub async fn changed(&self) {
        self.changed.notified().await
    }
}

impl Inner {
    fn commit(&mut self, event: Event) {
        if let Err(e) = self.journal.append(&event) {
            tracing::error!(error = %e, "journal append failed; the state directory is unusable");
            std::process::exit(70);
        }
        self.ledger.apply(event, &self.retry);
    }
}

impl BatchBundle {
    pub fn has_proof(&self) -> bool {
        self.proof_bytes != "0x"
    }

    pub fn proven(
        batch_id: String,
        proof: &ProofBundle,
        leaves: &[ReturnLeaf],
        lock_refs: &[SourceLockRef],
    ) -> Self {
        Self {
            batch_id,
            mode: proof.mode.clone(),
            vkey: proof.vkey_hash.clone(),
            public_values: format!("0x{}", hex::encode(&proof.public_values)),
            proof_bytes: format!("0x{}", hex::encode(&proof.proof_bytes)),
            settle_txid: None,
            leaves: leaves.iter().copied().map(LeafHex::from).collect(),
            lock_refs: lock_refs.iter().copied().map(LockRefHex::from).collect(),
        }
    }
}

impl ReturnRecord {
    pub fn queued(
        return_id: String,
        nullifier: [u8; 32],
        public_values_digest: [u8; 32],
        public_values: PublicValues,
        now: u128,
    ) -> Self {
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
            attempts: 0,
            not_before_ms: None,
            queue_position: None,
            created_at_ms: now,
            updated_at_ms: now,
        }
    }
}

impl ReturnRecord {
    pub(crate) fn transition(&mut self, status: ReturnStatus, at_ms: u128) {
        self.status = status;
        apply_status_defaults(self);
        self.updated_at_ms = at_ms;
        self.events.push(ReturnEvent {
            at_ms,
            status: self.status.clone(),
            code: event_code(&self.status).to_string(),
            message: self.message.clone(),
        });
    }

    pub(crate) fn transition_with(&mut self, status: ReturnStatus, at_ms: u128, message: &str) {
        self.transition(status, at_ms);
        self.message = message.to_string();
        if let Some(event) = self.events.last_mut() {
            event.message = message.to_string();
        }
    }

    pub fn failed_recoverably(&self) -> bool {
        self.status == ReturnStatus::Failed
            && self
                .failure
                .as_ref()
                .is_some_and(|failure| failure.recoverable)
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

    pub fn terminal(kind: ErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
            recoverable: false,
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

pub(crate) fn hex32(value: &[u8; 32]) -> String {
    format!("0x{}", hex::encode(value))
}

pub(crate) fn parse_hex32(input: &str) -> Option<[u8; 32]> {
    let raw = input.strip_prefix("0x").unwrap_or(input);
    hex::decode(raw).ok()?.try_into().ok()
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
