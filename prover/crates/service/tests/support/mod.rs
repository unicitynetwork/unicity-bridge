#![allow(dead_code)]

use std::{
    collections::{HashMap, VecDeque},
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
    sequencer::{ChainSyncError, Confirmations, Release, Settlement, SettlementState, Unconfirmed},
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

enum Step {
    Outcome(Outcome),
    /// Prints the txid, as the relayers do once they have signed, and never ends.
    SignAndHang(String),
}

#[derive(Clone)]
pub struct ScriptedSettler {
    outcomes: Arc<Mutex<VecDeque<Step>>>,
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
        self.outcomes
            .lock()
            .unwrap()
            .push_back(Step::Outcome(Box::new(outcome)));
        self
    }

    /// The next submission signs `txid` and then never returns, as a relayer still waiting
    /// on its receipt does when the service stops.
    pub fn then_sign_and_hang(self, txid: &str) -> Self {
        self.outcomes
            .lock()
            .unwrap()
            .push_back(Step::SignAndHang(txid.to_string()));
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
    async fn submit(
        &self,
        bundle: &BatchBundle,
        signed: &(dyn Fn(&str) + Send + Sync),
    ) -> SubmitOutcome {
        self.submitted.lock().unwrap().push(bundle.clone());
        let next = self.outcomes.lock().unwrap().pop_front();
        let outcome = match next {
            Some(Step::Outcome(outcome)) => outcome(bundle),
            Some(Step::SignAndHang(txid)) => {
                signed(&txid);
                return std::future::pending().await;
            }
            None => self.fallback.clone(),
        };
        if let SubmitOutcome::Submitted { txid } | SubmitOutcome::Unconfirmed { txid } = &outcome {
            signed(txid);
        }
        outcome
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

/// How the receipt command answers for a settlement transaction.
#[derive(Clone, Debug, Default)]
pub enum Receipts {
    /// A transaction the fake vault's log names is confirmed; any other is as scripted with
    /// [`FakeChainLog::transaction`], or unknown.
    #[default]
    Read,
    /// Every read fails, as an RPC that rate-limits does.
    Down,
    /// No receipt command is configured.
    Absent,
}

#[derive(Clone)]
pub struct FakeChainLog {
    log: Arc<Mutex<SettledLog>>,
    answers: Arc<Mutex<VecDeque<bool>>>,
    calls: Arc<AtomicUsize>,
    second: Arc<Mutex<SecondSource>>,
    confirms: Arc<Mutex<bool>>,
    asked: Arc<Mutex<Vec<Release>>>,
    delay: Arc<Mutex<Duration>>,
    receipts: Arc<Mutex<Receipts>>,
    transactions: Arc<Mutex<HashMap<String, SettlementState>>>,
    looked_up: Arc<Mutex<Vec<String>>>,
    unreadable: Arc<Mutex<bool>>,
}

impl Default for FakeChainLog {
    fn default() -> Self {
        Self {
            log: Arc::default(),
            answers: Arc::default(),
            calls: Arc::default(),
            second: Arc::default(),
            confirms: Arc::new(Mutex::new(true)),
            asked: Arc::default(),
            delay: Arc::default(),
            receipts: Arc::default(),
            transactions: Arc::default(),
            looked_up: Arc::default(),
            unreadable: Arc::default(),
        }
    }
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
        let txid = format!("0x{:064x}", self.log.lock().unwrap().batches.len() + 0xe1);
        self.settle_as(nullifiers, &txid);
        txid
    }

    /// Settle `nullifiers` in the transaction `txid`.
    pub fn settle_as(&self, nullifiers: &[[u8; 32]], txid: &str) {
        let mut log = self.log.lock().unwrap();
        let acc = s2::rebuild(&log.batches).unwrap();
        let next = s2::next_batch(&acc, nullifiers).unwrap();
        let txid = txid.to_string();
        log.batches.push(SettledBatch {
            nullifiers: nullifiers.to_vec(),
            spent_root_old: next.spent_root_old,
            spent_root_new: next.spent_root_new,
            txid: Some(txid),
        });
        log.on_chain_spent_root = Some(next.spent_root_new);
    }

    /// What the receipt command reads of `txid` while the log does not name it.
    pub fn transaction(&self, txid: &str, state: SettlementState) {
        self.transactions
            .lock()
            .unwrap()
            .insert(txid.to_string(), state);
    }

    pub fn receipts(&self, receipts: Receipts) {
        *self.receipts.lock().unwrap() = receipts;
    }

    /// Every settlement transaction the receipt command was asked about.
    pub fn looked_up(&self) -> Vec<String> {
        self.looked_up.lock().unwrap().clone()
    }

    /// Every read of the log fails until this is called with `false`.
    pub fn unreadable(&self, unreadable: bool) {
        *self.unreadable.lock().unwrap() = unreadable;
    }

    /// No confirmation command is configured.
    pub fn no_second_source(&self) {
        *self.confirms.lock().unwrap() = false;
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
        if self.answers.lock().unwrap().pop_front() == Some(false)
            || *self.unreadable.lock().unwrap()
        {
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
        if !*self.confirms.lock().unwrap() {
            return Err(ChainSyncError::NoConfirmationSource);
        }
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

    fn confirms(&self) -> bool {
        *self.confirms.lock().unwrap()
    }

    async fn settlement(&self, txid: &str) -> Result<Settlement, ChainSyncError> {
        self.looked_up.lock().unwrap().push(txid.to_string());
        let receipts = self.receipts.lock().unwrap().clone();
        match receipts {
            Receipts::Absent => return Err(ChainSyncError::NoReceiptSource),
            Receipts::Down => {
                return Err(ChainSyncError::CommandFailed {
                    status: "exit status: 1".to_string(),
                    stderr: "Too Many Requests (code=UNKNOWN_ERROR, version=6.16.0)".to_string(),
                })
            }
            Receipts::Read => {}
        }
        let logged = self
            .log
            .lock()
            .unwrap()
            .batches
            .iter()
            .any(|b| b.txid.as_deref() == Some(txid));
        let state = if logged {
            SettlementState::Confirmed
        } else {
            self.transactions
                .lock()
                .unwrap()
                .get(txid)
                .copied()
                .unwrap_or(SettlementState::Unknown)
        };
        let confirmations = match state {
            SettlementState::Confirmed | SettlementState::Reverted => 12,
            SettlementState::Pending => 3,
            SettlementState::Unknown => 0,
        };
        Ok(Settlement {
            txid: txid.to_string(),
            state,
            confirmations,
        })
    }
}
