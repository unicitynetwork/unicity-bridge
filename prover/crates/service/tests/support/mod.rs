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
    domain::{
        assembler::{Rejected, RejectionKind},
        burn::{Burn, LeafToSettle},
    },
    ports::{ChainLog, ProofBackend, Settler},
    prover::{ProofBundle, ProverError},
    sequencer::{ChainSyncError, Confirmations, Release, Unconfirmed},
    store::{BatchBundle, ReturnRecord},
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
        self.reject_as(nullifier, reason, RejectionKind::Recipient);
    }

    pub fn reject_as(&self, nullifier: &str, reason: &str, kind: RejectionKind) {
        self.rejected.lock().unwrap().push(Rejected {
            nullifier: nullifier.to_string(),
            reason: reason.to_string(),
            kind,
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

    async fn simulate(&self, _leaves: &[LeafToSettle]) -> Result<Vec<Rejected>, SimulateError> {
        Ok(self.rejected.lock().unwrap().clone())
    }
}

/// What the second source says about the releases it is asked to confirm.
#[derive(Clone, Debug, Default)]
pub enum SecondSource {
    /// Confirms a release the fake vault's log shows in the transaction asked about.
    #[default]
    Agrees,
    /// Confirms nothing, for this reason.
    Disagrees(String),
    /// Cannot be reached.
    Down,
}

#[derive(Clone, Default)]
pub struct FakeChainLog {
    log: Arc<Mutex<SettledLog>>,
    answers: Arc<Mutex<VecDeque<bool>>>,
    calls: Arc<AtomicUsize>,
    second: Arc<Mutex<SecondSource>>,
    asked: Arc<Mutex<Vec<Release>>>,
    delay: Arc<Mutex<Duration>>,
}

pub fn hex(bytes: &[u8; 32]) -> String {
    format!("0x{}", ::hex::encode(bytes))
}

impl FakeChainLog {
    pub fn new() -> Self {
        Self::default()
    }

    /// Settle `nullifiers` in a transaction of their own, and name it.
    pub fn settle_externally(&self, nullifiers: &[[u8; 32]]) -> String {
        let mut log = self.log.lock().unwrap();
        let acc = s2::rebuild(&log.batches).unwrap();
        let next = s2::next_batch(&acc, nullifiers).unwrap();
        let txid = format!("0x{:064x}", log.batches.len() + 0xe1);
        log.batches.push(SettledBatch {
            nullifiers: nullifiers.to_vec(),
            spent_root_old: next.spent_root_old,
            spent_root_new: next.spent_root_new,
            txid: Some(txid.clone()),
        });
        log.on_chain_spent_root = Some(next.spent_root_new);
        txid
    }

    /// The vault as a log that never showed a settlement.
    pub fn retract_everything(&self) {
        *self.log.lock().unwrap() = SettledLog::default();
    }

    /// Each read of the log from now on takes `delay`, as a slow events command does.
    pub fn slow(&self, delay: Duration) {
        *self.delay.lock().unwrap() = delay;
    }

    pub fn second_source(&self, second: SecondSource) {
        *self.second.lock().unwrap() = second;
    }

    /// Every release the second source was asked about.
    pub fn asked(&self) -> Vec<Release> {
        self.asked.lock().unwrap().clone()
    }

    pub fn spent_root(&self) -> [u8; 32] {
        s2::rebuild(&self.log.lock().unwrap().batches)
            .unwrap()
            .spent_root
    }

    pub fn forget_history(&self) {
        self.log.lock().unwrap().batches.clear();
    }

    /// The next reads answer in turn: `true` serves the log, `false` fails the way the
    /// events command does when an RPC request times out. Reads past the script serve the log.
    pub fn answer(&self, reachable: &[bool]) {
        self.answers
            .lock()
            .unwrap()
            .extend(reachable.iter().copied());
    }

    pub fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
}

impl ChainLog for FakeChainLog {
    async fn synced_accumulator(
        &self,
        known: Vec<SettledBatch>,
    ) -> Result<RebuiltAccumulator, ChainSyncError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let delay = *self.delay.lock().unwrap();
        if !delay.is_zero() {
            tokio::time::sleep(delay).await;
        }
        if self.answers.lock().unwrap().pop_front() == Some(false) {
            return Err(ChainSyncError::CommandFailed {
                status: "exit status: 1".to_string(),
                stderr: "request timeout (code=TIMEOUT, version=6.16.0)".to_string(),
            });
        }
        let mut log = self.log.lock().unwrap().clone();
        log.batches.extend(known);
        s2::rebuild_verified(&log).map_err(|e| ChainSyncError::Rebuild(e.to_string()))
    }

    async fn confirm_releases(
        &self,
        releases: &[Release],
    ) -> Result<Confirmations, ChainSyncError> {
        self.asked.lock().unwrap().extend(releases.iter().cloned());
        let second = self.second.lock().unwrap().clone();
        match second {
            SecondSource::Down => Err(ChainSyncError::CommandFailed {
                status: "exit status: 1".to_string(),
                stderr: "confirmation RPC unreachable".to_string(),
            }),
            SecondSource::Disagrees(reason) => Ok(Confirmations {
                confirmed: Vec::new(),
                unconfirmed: releases
                    .iter()
                    .map(|r| Unconfirmed {
                        nullifier: r.nullifier.clone(),
                        txid: r.txid.clone(),
                        reason: reason.clone(),
                    })
                    .collect(),
            }),
            SecondSource::Agrees => {
                let log = self.log.lock().unwrap();
                let shown = |r: &Release| {
                    log.batches.iter().any(|b| {
                        b.txid.as_deref() == Some(r.txid.as_str())
                            && b.nullifiers.iter().any(|n| hex(n) == r.nullifier)
                    })
                };
                Ok(Confirmations {
                    confirmed: releases.iter().filter(|r| shown(r)).cloned().collect(),
                    unconfirmed: Vec::new(),
                })
            }
        }
    }
}
