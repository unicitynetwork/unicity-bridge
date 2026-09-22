#![allow(dead_code)]

use std::{
    collections::VecDeque,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc, Mutex,
    },
    time::Duration,
};

use bridge_return_core::BridgeConfig;
use bridge_return_guest::{wire, GuestInput};
use bridge_return_host::{
    fixture::{build_b1_direct_bridge_fixture, build_settlement_fixture_continued},
    s1,
    s2::{self, RebuiltAccumulator, SettledBatch, SettledLog},
};
use bridge_return_service::{
    domain::{assembler::Rejected, burn::Burn},
    ports::{ChainLog, ProofBackend, Settler},
    prover::{ProofBundle, ProverError},
    sequencer::ChainSyncError,
    store::{BatchBundle, LeafHex, ReturnRecord},
    submitter::{SimulateError, SubmitOutcome},
};

pub fn config() -> BridgeConfig {
    build_b1_direct_bridge_fixture().input.config
}

pub fn member(nonce: u64, salt: u8) -> GuestInput {
    build_settlement_fixture_continued(config(), [0xB2; 20], 1_000_000, nonce, salt, &[]).input
}

pub fn nullifier_of(input: &GuestInput) -> [u8; 32] {
    input.return_leaves[0].nullifier
}

pub fn burn_and_record(id: &str, input: &GuestInput, now_ms: u128) -> (Burn, ReturnRecord) {
    let wire = wire::encode_guest_input(input);
    let report = s1::precheck_wire(&wire).unwrap();
    let burn = Burn::from_input(id.to_string(), input, wire);
    let record = ReturnRecord::queued(
        id.to_string(),
        nullifier_of(input),
        report.public_values_digest,
        report.public_values,
        now_ms,
    );
    (burn, record)
}

pub enum ProveStep {
    Succeed { delay: Duration },
    Fail(String),
    Hang,
}

#[derive(Clone)]
pub struct ScriptedProver {
    steps: Arc<Mutex<VecDeque<ProveStep>>>,
    calls: Arc<AtomicUsize>,
}

impl ScriptedProver {
    pub fn new(steps: Vec<ProveStep>) -> Self {
        Self {
            steps: Arc::new(Mutex::new(steps.into())),
            calls: Arc::new(AtomicUsize::new(0)),
        }
    }

    pub fn instant() -> Self {
        Self::new(Vec::new())
    }

    pub fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
}

impl ProofBackend for ScriptedProver {
    async fn prove(&self, _batch_id: &str, wire: Vec<u8>) -> Result<ProofBundle, ProverError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let step = self
            .steps
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or(ProveStep::Succeed {
                delay: Duration::ZERO,
            });
        match step {
            ProveStep::Succeed { delay } => {
                tokio::time::sleep(delay).await;
                let report = s1::precheck_wire(&wire)?;
                Ok(ProofBundle {
                    mode: "scripted".to_string(),
                    public_values: report.public_values_abi,
                    proof_bytes: vec![1],
                    vkey_hash: None,
                })
            }
            ProveStep::Fail(message) => Err(ProverError::Join(message)),
            ProveStep::Hang => std::future::pending().await,
        }
    }
}

type Outcome = Box<dyn FnOnce(&BatchBundle) -> SubmitOutcome + Send>;

#[derive(Clone)]
pub struct ScriptedSettler {
    outcomes: Arc<Mutex<VecDeque<Outcome>>>,
    fallback: SubmitOutcome,
    rejected: Arc<Mutex<Vec<Rejected>>>,
    submitted: Arc<Mutex<Vec<BatchBundle>>>,
}

impl ScriptedSettler {
    pub fn skipping() -> Self {
        Self::with_fallback(SubmitOutcome::Skipped)
    }

    pub fn settling(txid: &str) -> Self {
        Self::with_fallback(SubmitOutcome::Submitted {
            txid: txid.to_string(),
        })
    }

    fn with_fallback(fallback: SubmitOutcome) -> Self {
        Self {
            outcomes: Arc::new(Mutex::new(VecDeque::new())),
            fallback,
            rejected: Arc::new(Mutex::new(Vec::new())),
            submitted: Arc::new(Mutex::new(Vec::new())),
        }
    }

    pub fn then(self, outcome: SubmitOutcome) -> Self {
        self.then_with(move |_| outcome)
    }

    pub fn then_with(
        self,
        outcome: impl FnOnce(&BatchBundle) -> SubmitOutcome + Send + 'static,
    ) -> Self {
        self.outcomes.lock().unwrap().push_back(Box::new(outcome));
        self
    }

    pub fn reject(&self, nullifier: &str, reason: &str) {
        self.rejected.lock().unwrap().push(Rejected {
            nullifier: nullifier.to_string(),
            reason: reason.to_string(),
        });
    }

    pub fn submitted(&self) -> Vec<BatchBundle> {
        self.submitted.lock().unwrap().clone()
    }
}

impl Settler for ScriptedSettler {
    async fn submit(&self, bundle: &BatchBundle) -> SubmitOutcome {
        self.submitted.lock().unwrap().push(bundle.clone());
        let next = self.outcomes.lock().unwrap().pop_front();
        match next {
            Some(outcome) => outcome(bundle),
            None => self.fallback.clone(),
        }
    }

    async fn simulate(&self, _leaves: &[LeafHex]) -> Result<Vec<Rejected>, SimulateError> {
        Ok(self.rejected.lock().unwrap().clone())
    }
}

#[derive(Clone, Default)]
pub struct FakeChainLog {
    log: Arc<Mutex<SettledLog>>,
}

impl FakeChainLog {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn settle_externally(&self, nullifiers: &[[u8; 32]]) {
        let mut log = self.log.lock().unwrap();
        let acc = s2::rebuild(&log.batches).unwrap();
        let next = s2::next_batch(&acc, nullifiers).unwrap();
        log.batches.push(SettledBatch {
            nullifiers: nullifiers.to_vec(),
            spent_root_old: next.spent_root_old,
            spent_root_new: next.spent_root_new,
        });
        log.on_chain_spent_root = Some(next.spent_root_new);
    }

    pub fn spent_root(&self) -> [u8; 32] {
        s2::rebuild(&self.log.lock().unwrap().batches)
            .unwrap()
            .spent_root
    }
}

impl ChainLog for FakeChainLog {
    async fn synced_accumulator(&self) -> Result<RebuiltAccumulator, ChainSyncError> {
        let log = self.log.lock().unwrap().clone();
        s2::rebuild_verified(&log).map_err(|e| ChainSyncError::Rebuild(e.to_string()))
    }
}
