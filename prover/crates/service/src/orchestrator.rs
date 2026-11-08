use std::{collections::HashMap, sync::Mutex, time::Duration};

use bridge_return_host::s2::RebuiltAccumulator;

use crate::{
    clock::Clock,
    domain::{
        assembler::{assemble, AssembleError, Exclusion},
        batch::{Batch, BatchStatus},
        burn::{Burn, LeafToSettle},
        event::Event,
        ledger::{WaitingRelease, RELEASE_RECHECK_MS},
        policy::BatchPolicy,
        retry::RetryPolicy,
    },
    ports::{ChainLog, ProofBackend, Settler},
    sequencer::{ChainSyncError, Confirmations, Release, SettlementState},
    shutdown::Drain,
    store::{hex32, parse_hex32, BatchBundle, ErrorKind, ReturnStore, Taken},
    submitter::SubmitOutcome,
};

/// A chain log that could not be read is read again after each of these delays before
/// the batch fails, so one failed RPC request costs the members neither an attempt nor
/// the proof they are waiting on.
const SYNC_RETRY_DELAYS: [Duration; 3] = [
    Duration::from_secs(15),
    Duration::from_secs(30),
    Duration::from_secs(60),
];

/// A settlement the relayer broadcast without seeing it settle is looked up this often while
/// the service waits on it, for up to [`SETTLEMENT_WAIT`]; no other batch starts meanwhile,
/// since one built on the vault's root before it would only be proven to be rebased. Still
/// undecided after that, it is looked up every [`RELEASE_RECHECK_MS`] while other batches run.
const SETTLEMENT_CHECK_EVERY: Duration = Duration::from_secs(30);
const SETTLEMENT_WAIT: Duration = Duration::from_secs(15 * 60);

/// A transaction the chain does not know may only be missing from the mempool of the node
/// that answered, and a signed transaction whose nonce is unused can still land; it counts
/// as dropped only once every lookup for this long has found it unknown.
const UNKNOWN_FOR_MS: u128 = 10 * 60_000;

/// A return whose nullifier the vault's log shows spent, by `txid` when the log names it.
struct Released {
    id: String,
    nullifier: String,
    txid: Option<String>,
}

impl From<&WaitingRelease> for Released {
    fn from(waiting: &WaitingRelease) -> Self {
        Self {
            id: waiting.id.clone(),
            nullifier: waiting.nullifier.clone(),
            txid: waiting.txid.clone(),
        }
    }
}

pub struct Orchestrator<P, S, C> {
    store: ReturnStore,
    prover: P,
    settler: S,
    chain: C,
    policy: BatchPolicy,
    retry: RetryPolicy,
    clock: Clock,
    drain: Drain,
    /// Since when each broadcast settlement has been found unknown at every lookup. Kept in
    /// memory: a start counts afresh.
    unknown_since: Mutex<HashMap<String, u128>>,
}

#[derive(Debug, Eq, PartialEq)]
enum Stage {
    Done,
    Rebase,
}

/// What a look at a broadcast settlement found.
enum Verdict {
    /// The batch's own transaction settled it.
    Settled(String),
    /// Every nullifier of the batch is released, by a transaction the service holds no
    /// receipt for.
    Released(RebuiltAccumulator),
    /// The vault root moved past the batch: it is proven again.
    Rebase(String),
    /// The batch's transaction will not settle it, and nothing else did: a submission that
    /// failed.
    Fail(String),
    /// Nothing decides it yet; it costs the members nothing.
    Wait(String),
}

impl<P: ProofBackend, S: Settler, C: ChainLog> Orchestrator<P, S, C> {
    pub fn new(
        store: ReturnStore,
        prover: P,
        settler: S,
        chain: C,
        policy: BatchPolicy,
        retry: RetryPolicy,
        clock: Clock,
    ) -> Self {
        Self {
            store,
            prover,
            settler,
            chain,
            policy,
            retry,
            clock,
            drain: Drain::default(),
            unknown_since: Mutex::new(HashMap::new()),
        }
    }

    /// Prove and settle batches until `drain` starts, then return once the batch in flight
    /// has run to its end.
    pub async fn run(mut self, drain: Drain) {
        self.drain = drain.clone();
        // A settlement broadcast before a restart is looked up before anything else, and no
        // second transaction is ever sent for its batch.
        self.recheck_settlements().await;
        for batch in self.store.read(|ledger| ledger.unsettled_proofs()) {
            if drain.is_draining() {
                break;
            }
            self.run_batch(batch).await;
        }
        let mut collect = true;
        while !drain.is_draining() {
            self.recheck_settlements().await;
            self.recheck_releases().await;
            // A recheck runs commands that can take minutes; a drain begun meanwhile starts
            // no batch.
            if drain.is_draining() {
                break;
            }
            let recheck = self.next_check();
            match self
                .store
                .take_next(&self.policy, self.clock.now_ms(), collect)
            {
                Taken::Batch(batch) => {
                    self.run_batch(batch).await;
                    collect = false;
                }
                Taken::WaitUntil(at_ms) => {
                    let at_ms = recheck.map_or(at_ms, |r| r.min(at_ms));
                    tokio::select! {
                        _ = self.store.changed() => {}
                        _ = self.clock.sleep_until_ms(at_ms) => {}
                        _ = drain.started() => {}
                    }
                    collect = true;
                }
                Taken::Idle => {
                    let recheck = async {
                        match recheck {
                            Some(at_ms) => self.clock.sleep_until_ms(at_ms).await,
                            None => std::future::pending().await,
                        }
                    };
                    tokio::select! {
                        _ = self.store.changed() => {}
                        _ = recheck => {}
                        _ = drain.started() => {}
                    }
                    collect = true;
                }
            }
        }
        tracing::info!("drained: no batch in flight, and none starts");
    }

    async fn run_batch(&self, batch: Batch) {
        let stage = if batch.bundle.is_some() {
            self.settle_stage(&batch).await
        } else {
            self.prove_stage(&batch).await
        };
        self.carry_on(&batch.id, stage).await;
    }

    /// Prove the batch again for as long as its stages ask for a rebase.
    async fn carry_on(&self, batch_id: &str, mut stage: Stage) {
        while stage == Stage::Rebase {
            let Some(current) = self.reload(batch_id) else {
                return;
            };
            stage = self.prove_stage(&current).await;
        }
    }

    /// When the next release or broadcast settlement is due to be looked at again. Releases
    /// count only while a second source could confirm them, since nothing rechecks them
    /// otherwise.
    fn next_check(&self) -> Option<u128> {
        let confirms = self.chain.confirms();
        self.store.read(|ledger| {
            let releases = confirms.then(|| ledger.next_release_check()).flatten();
            [releases, ledger.next_settlement_check()]
                .into_iter()
                .flatten()
                .min()
        })
    }

    async fn prove_stage(&self, batch: &Batch) -> Stage {
        tracing::info!(batch_id = %batch.id, members = batch.members.len(), "batch proving started");
        let Some(acc) = self.synced(batch).await else {
            return Stage::Done;
        };
        let burns: Vec<Burn> = self
            .store
            .read(|ledger| ledger.burns(&batch.members).into_iter().cloned().collect());
        let nullifiers: HashMap<String, String> = burns
            .iter()
            .map(|b| (b.id.clone(), b.nullifier().to_string()))
            .collect();
        let leaves: Vec<LeafToSettle> = burns.iter().filter_map(Burn::leaf_to_settle).collect();
        let rejected = match self.settler.simulate(&leaves).await {
            Ok(rejected) => rejected,
            Err(e) => {
                return self.fail(
                    batch,
                    ErrorKind::ChainRejected,
                    format!("settlement simulation failed: {e}"),
                )
            }
        };
        let assembled = tokio::task::spawn_blocking(move || {
            let members: Vec<&Burn> = burns.iter().collect();
            assemble(&members, &acc, &rejected)
        })
        .await;
        let assembled = match assembled {
            Ok(Ok(assembled)) => assembled,
            Ok(Err(AssembleError::AllExcluded(excluded))) => {
                let excluded = self.confirmed(excluded, &nullifiers).await;
                self.exclude(excluded);
                return self.fail(
                    batch,
                    ErrorKind::ChainRejected,
                    "every member was excluded before proving",
                );
            }
            Ok(Err(e)) => return self.fail(batch, ErrorKind::ChainRejected, e.to_string()),
            Err(e) => {
                return self.fail(
                    batch,
                    ErrorKind::ProvingFailed,
                    format!("assembly task failed: {e}"),
                )
            }
        };
        let excluded = self
            .confirmed(assembled.excluded.clone(), &nullifiers)
            .await;
        self.exclude(excluded);
        let spent_root_old = hex32(&assembled.public_values.spent_root_old);
        let nullifiers: Vec<String> = assembled
            .leaves
            .iter()
            .map(|l| hex32(&l.nullifier))
            .collect();
        let reusable = self
            .store
            .read(|ledger| ledger.proof_for(&nullifiers, &spent_root_old));
        let bundle = match reusable {
            Some(reused) => {
                tracing::info!(batch_id = %batch.id, "reusing an earlier proof for the same batch");
                BatchBundle {
                    batch_id: batch.id.clone(),
                    settle_txid: None,
                    ..reused
                }
            }
            None => match self.prover.prove(&batch.id, assembled.wire.clone()).await {
                Ok(proof) => BatchBundle::proven(
                    batch.id.clone(),
                    &proof,
                    &assembled.leaves,
                    &assembled.lock_refs,
                ),
                Err(e) => return self.fail(batch, ErrorKind::ProvingFailed, e.to_string()),
            },
        };
        self.store.apply(Event::BatchProven {
            id: batch.id.clone(),
            bundle,
            spent_root_old,
            at_ms: self.clock.now_ms(),
        });
        tracing::info!(batch_id = %batch.id, leaves = assembled.leaves.len(), "batch proven");
        match self.reload(&batch.id) {
            Some(proven) => self.settle_stage(&proven).await,
            None => Stage::Done,
        }
    }

    async fn settle_stage(&self, batch: &Batch) -> Stage {
        let Some(acc) = self.synced(batch).await else {
            return Stage::Done;
        };
        if all_released(batch, &acc) {
            return self.released(batch, &acc).await;
        }
        if !batch.chains_onto(&acc.spent_root) {
            return self.rebase_or_fail(batch, "vault root moved after proving");
        }
        let Some(bundle) = &batch.bundle else {
            return Stage::Done;
        };
        // The relayer prints the txid once it has signed the transaction, before sending it;
        // journaled then, it survives whatever happens to the service or the relayer next,
        // and no second transaction is ever sent for the batch while this one may land.
        let signed = {
            let (store, clock, id) = (self.store.clone(), self.clock.clone(), batch.id.clone());
            move |txid: &str| {
                tracing::info!(batch_id = %id, txid, "settlement signed; waiting on it");
                store.apply(Event::BatchSubmitted {
                    id: id.clone(),
                    txid: txid.to_string(),
                    at_ms: clock.now_ms(),
                });
            }
        };
        match self.settler.submit(bundle, &signed).await {
            SubmitOutcome::Skipped => {
                tracing::info!(batch_id = %batch.id, "no submitter configured or no proof to settle; batch left proven");
                Stage::Done
            }
            SubmitOutcome::Submitted { txid } => {
                tracing::info!(batch_id = %batch.id, %txid, "batch settled on chain");
                self.store.apply(Event::BatchSettled {
                    id: batch.id.clone(),
                    txid: Some(txid),
                    at_ms: self.clock.now_ms(),
                });
                Stage::Done
            }
            SubmitOutcome::Unconfirmed { txid } => {
                tracing::warn!(batch_id = %batch.id, %txid, "settlement not seen to settle; waiting on it");
                let journaled = self
                    .reload(&batch.id)
                    .is_some_and(|b| b.broadcast_txid.as_deref() == Some(txid.as_str()));
                if !journaled {
                    signed(&txid);
                }
                self.await_settlement(&batch.id).await
            }
            SubmitOutcome::StaleRoot => self.rebase_or_fail(batch, "vault: stale root"),
            SubmitOutcome::Failed { message } => {
                self.fail(batch, ErrorKind::SubmissionFailed, message)
            }
        }
    }

    /// Wait on the batch's broadcast settlement, looking it up every
    /// [`SETTLEMENT_CHECK_EVERY`] for up to [`SETTLEMENT_WAIT`]. Undecided by then, it is
    /// looked up again every [`RELEASE_RECHECK_MS`] while other batches run. A drain ends the
    /// wait at once: the settlement is journaled, and the next start looks it up.
    async fn await_settlement(&self, batch_id: &str) -> Stage {
        let until = self.clock.now_ms() + SETTLEMENT_WAIT.as_millis();
        loop {
            tokio::select! {
                _ = tokio::time::sleep(SETTLEMENT_CHECK_EVERY) => {}
                _ = self.drain.started() => {
                    tracing::info!(batch_id, "draining; the broadcast settlement is looked up at the next start");
                    return Stage::Done;
                }
            }
            let Some(batch) = self
                .reload(batch_id)
                .filter(|b| b.status == BatchStatus::Submitted)
            else {
                return Stage::Done;
            };
            let verdict = self.check_settlement(&batch).await;
            if let Verdict::Wait(why) = &verdict {
                if self.clock.now_ms() < until {
                    tracing::info!(batch_id, %why, "broadcast settlement not decided yet");
                    continue;
                }
            }
            return self.decide(&batch, verdict).await;
        }
    }

    /// Broadcast settlements that a wait left undecided, or that a start found, looked up
    /// once each when due.
    async fn recheck_settlements(&self) {
        let due = self
            .store
            .read(|ledger| ledger.settlements_due(self.clock.now_ms()));
        for batch in due {
            if self.drain.is_draining() {
                return;
            }
            let verdict = self.check_settlement(&batch).await;
            // A rebase proves the batch again, which a drain starts no more than any other
            // proof: the next start looks the settlement up again instead.
            if matches!(verdict, Verdict::Rebase(_)) && self.drain.is_draining() {
                return;
            }
            let stage = self.decide(&batch, verdict).await;
            self.carry_on(&batch.id, stage).await;
        }
    }

    /// Look up the batch's broadcast transaction through the receipt command: settled, it
    /// settles the batch; reverted, or unknown to the chain well after it was broadcast, it
    /// will not, and the vault log decides the rest. A transaction still pending, or one
    /// that cannot be read, decides nothing.
    async fn check_settlement(&self, batch: &Batch) -> Verdict {
        let Some(txid) = batch.broadcast_txid.as_deref() else {
            return Verdict::Wait("no broadcast transaction is recorded for it".to_string());
        };
        let looked_up = self.chain.settlement(txid).await;
        if !matches!(&looked_up, Ok(s) if s.state == SettlementState::Unknown) {
            self.unknown_since.lock().unwrap().remove(&batch.id);
        }
        match looked_up {
            Ok(settlement) => match settlement.state {
                SettlementState::Confirmed => Verdict::Settled(txid.to_string()),
                SettlementState::Pending => Verdict::Wait(format!(
                    "{txid} is not settled yet ({} confirmation(s))",
                    settlement.confirmations
                )),
                SettlementState::Reverted => {
                    self.refuted(batch, format!("settlement {txid} reverted on chain"))
                        .await
                }
                SettlementState::Unknown => {
                    let now = self.clock.now_ms();
                    let since = *self
                        .unknown_since
                        .lock()
                        .unwrap()
                        .entry(batch.id.clone())
                        .or_insert(now);
                    if now.saturating_sub(since) < UNKNOWN_FOR_MS {
                        return Verdict::Wait(format!("the chain does not know {txid}"));
                    }
                    let verdict = self
                        .refuted(
                            batch,
                            format!(
                                "settlement {txid} was dropped: the chain has not known it for {} minutes",
                                UNKNOWN_FOR_MS / 60_000
                            ),
                        )
                        .await;
                    if !matches!(verdict, Verdict::Wait(_)) {
                        self.unknown_since.lock().unwrap().remove(&batch.id);
                    }
                    verdict
                }
            },
            Err(ChainSyncError::NoReceiptSource) => self.by_log_alone(batch).await,
            Err(error) => {
                Verdict::Wait(format!("the receipt of {txid} could not be read: {error}"))
            }
        }
    }

    /// The batch's own transaction will not settle it. The vault log says whether another
    /// did, or moved the root past it; only if neither is the batch failed, as a submission
    /// that did not land. A log that cannot be read decides nothing.
    async fn refuted(&self, batch: &Batch, why: String) -> Verdict {
        let known = self.store.read(|ledger| ledger.settled_batches());
        match self.chain.synced_accumulator(known).await {
            Ok(acc) if all_released(batch, &acc) => Verdict::Released(acc),
            Ok(acc) if !batch.chains_onto(&acc.spent_root) => {
                Verdict::Rebase(format!("{why}, and the vault root moved"))
            }
            Ok(_) => Verdict::Fail(why),
            Err(error) => Verdict::Wait(format!(
                "{why}, but the vault log could not be read: {error}"
            )),
        }
    }

    /// Without a receipt command only the vault log decides a broadcast settlement: the
    /// batch released, or overtaken. A transaction that never lands is not failed this way.
    async fn by_log_alone(&self, batch: &Batch) -> Verdict {
        let known = self.store.read(|ledger| ledger.settled_batches());
        match self.chain.synced_accumulator(known).await {
            Ok(acc) if all_released(batch, &acc) => Verdict::Released(acc),
            Ok(acc) if !batch.chains_onto(&acc.spent_root) => {
                Verdict::Rebase("vault root moved while the settlement was unconfirmed".to_string())
            }
            Ok(_) => Verdict::Wait(
                "no receipt command is configured, and the vault log does not show it yet"
                    .to_string(),
            ),
            Err(error) => Verdict::Wait(format!("the vault log could not be read: {error}")),
        }
    }

    async fn decide(&self, batch: &Batch, verdict: Verdict) -> Stage {
        match verdict {
            Verdict::Settled(txid) => {
                tracing::info!(batch_id = %batch.id, %txid, "broadcast settlement settled the batch");
                self.store.apply(Event::BatchSettled {
                    id: batch.id.clone(),
                    txid: Some(txid),
                    at_ms: self.clock.now_ms(),
                });
                Stage::Done
            }
            Verdict::Released(acc) => self.released(batch, &acc).await,
            Verdict::Rebase(why) => self.rebase_or_fail(batch, &why),
            Verdict::Fail(why) => self.fail(batch, ErrorKind::SubmissionFailed, why),
            Verdict::Wait(why) => {
                tracing::warn!(
                    batch_id = %batch.id,
                    %why,
                    "broadcast settlement still undecided; looked up again in {} minutes while other batches run",
                    RELEASE_RECHECK_MS / 60_000
                );
                self.store
                    .reschedule_settlement(&batch.id, self.clock.now_ms() + RELEASE_RECHECK_MS);
                Stage::Done
            }
        }
    }

    /// Every nullifier of the proven batch is spent on the vault, by a transaction the
    /// service holds no receipt for. The batch settles if a second source confirms every
    /// release in one transaction. Otherwise each member settles or waits on its own
    /// confirmation, and the batch ends failed, so its transition is never taken for one the
    /// vault made.
    async fn released(&self, batch: &Batch, acc: &RebuiltAccumulator) -> Stage {
        let members: Vec<Released> = self.store.read(|ledger| {
            batch
                .nullifiers()
                .iter()
                .filter_map(|n| {
                    let member = ledger.by_nullifier(&hex32(n))?;
                    member.in_batch(&batch.id).then(|| Released {
                        id: member.burn.id.clone(),
                        nullifier: hex32(n),
                        txid: acc.txids.get(n).cloned(),
                    })
                })
                .collect()
        });
        let verdicts = self.confirm(&members).await;
        // The vault made this batch's own transition only if its log holds it; a different
        // batch that released the same nullifiers did not, and recording it as settled
        // would give the next rebuild two continuations of one root.
        let made = batch
            .settled_batch()
            .is_some_and(|ours| acc.batches.iter().any(|b| b.same_transition(&ours)));
        let confirmed: Vec<&str> = verdicts
            .iter()
            .filter_map(|(_, verdict)| match verdict {
                Exclusion::AlreadySettled {
                    txid: Some(txid),
                    unconfirmed: None,
                } => Some(txid.as_str()),
                _ => None,
            })
            .collect();
        let one_transaction = confirmed.first().filter(|first| {
            made && confirmed.len() == verdicts.len()
                && confirmed.iter().all(|t| t.eq_ignore_ascii_case(first))
        });
        if let Some(txid) = one_transaction {
            tracing::info!(batch_id = %batch.id, %txid, "batch released on chain, confirmed by a second source");
            self.store.apply(Event::BatchSettled {
                id: batch.id.clone(),
                txid: Some(txid.to_string()),
                at_ms: self.clock.now_ms(),
            });
            return Stage::Done;
        }
        let message = if made {
            format!(
                "every member was released on chain without a receipt of the service's own; \
                 {} of {} confirmed by a second source",
                confirmed.len(),
                verdicts.len()
            )
        } else {
            format!(
                "every member was released on chain by a batch other than this one; \
                 {} of {} confirmed by a second source",
                confirmed.len(),
                verdicts.len()
            )
        };
        self.exclude(verdicts);
        self.fail(batch, ErrorKind::ChainRejected, message)
    }

    /// Returns whose release the vault's log showed but no second source confirmed are
    /// checked again once due, by asking the second source about the transaction the log
    /// named: confirmed, a return settles. The log is read again only for a release the
    /// source answered for without confirming it, or one the log named no transaction for:
    /// still released, it waits (asked about the transaction the log now names, if that
    /// changed); gone from the log, it is proven again. Without a second source nothing could
    /// confirm them, and nothing is checked.
    async fn recheck_releases(&self) {
        if !self.chain.confirms() {
            return;
        }
        let now_ms = self.clock.now_ms();
        let due = self.store.read(|ledger| ledger.releases_due(now_ms));
        if due.is_empty() {
            return;
        }
        let asked: Vec<Released> = due.iter().map(Released::from).collect();
        let answer = self.ask(&asked).await;
        let mut doubtful = Vec::new();
        for (waiting, (id, verdict)) in due.into_iter().zip(self.verdicts(&asked, &answer)) {
            let unconfirmed = matches!(
                &verdict,
                Exclusion::AlreadySettled {
                    unconfirmed: Some(_),
                    ..
                }
            );
            if unconfirmed && (answer.is_ok() || waiting.txid.is_none()) {
                doubtful.push((waiting, verdict));
            } else {
                self.exclude(vec![(id, verdict)]);
            }
        }
        if doubtful.is_empty() {
            return;
        }
        let known = self.store.read(|ledger| ledger.settled_batches());
        let acc = match self.chain.synced_accumulator(known).await {
            Ok(acc) => acc,
            Err(error) => {
                let why = format!("the chain log could not be checked again: {error}");
                let waiting = doubtful
                    .into_iter()
                    .map(|(waiting, _)| {
                        (
                            waiting.id,
                            Exclusion::AlreadySettled {
                                txid: waiting.txid,
                                unconfirmed: Some(why.clone()),
                            },
                        )
                    })
                    .collect();
                self.exclude(waiting);
                return;
            }
        };
        let mut renamed = Vec::new();
        for (waiting, verdict) in doubtful {
            let spent = parse_hex32(&waiting.nullifier)
                .filter(|n| acc.tree.non_membership_witness(n).is_none());
            match spent {
                Some(n) => {
                    let txid = acc.txids.get(&n).cloned();
                    if txid == waiting.txid {
                        self.exclude(vec![(waiting.id, verdict)]);
                    } else {
                        renamed.push(Released {
                            id: waiting.id,
                            nullifier: waiting.nullifier,
                            txid,
                        });
                    }
                }
                None => {
                    tracing::warn!(return_id = %waiting.id, "the vault log no longer shows the return released; proving it again");
                    self.store.apply(Event::Requeued {
                        id: waiting.id,
                        at_ms: self.clock.now_ms(),
                    });
                }
            }
        }
        let verdicts = self.confirm(&renamed).await;
        self.exclude(verdicts);
    }

    /// Settle-or-wait verdicts for the members assembly found already released.
    async fn confirmed(
        &self,
        excluded: Vec<(String, Exclusion)>,
        nullifiers: &HashMap<String, String>,
    ) -> Vec<(String, Exclusion)> {
        let released: Vec<Released> = excluded
            .iter()
            .filter_map(|(id, exclusion)| match exclusion {
                Exclusion::AlreadySettled { txid, .. } => Some(Released {
                    id: id.clone(),
                    nullifier: nullifiers.get(id)?.clone(),
                    txid: txid.clone(),
                }),
                _ => None,
            })
            .collect();
        let mut verdicts: HashMap<String, Exclusion> =
            self.confirm(&released).await.into_iter().collect();
        excluded
            .into_iter()
            .map(|(id, exclusion)| {
                let verdict = verdicts.remove(&id).unwrap_or(exclusion);
                (id, verdict)
            })
            .collect()
    }

    /// Ask the second source about each release: a return settles in the transaction the
    /// log names only once that source confirms the release there.
    async fn confirm(&self, released: &[Released]) -> Vec<(String, Exclusion)> {
        let answer = self.ask(released).await;
        self.verdicts(released, &answer)
    }

    async fn ask(&self, released: &[Released]) -> Result<Confirmations, ChainSyncError> {
        let asked: Vec<Release> = released
            .iter()
            .filter_map(|r| {
                Some(Release {
                    nullifier: r.nullifier.clone(),
                    txid: r.txid.clone()?,
                })
            })
            .collect();
        if asked.is_empty() {
            return Ok(Confirmations::default());
        }
        self.chain.confirm_releases(&asked).await
    }

    fn verdicts(
        &self,
        released: &[Released],
        answer: &Result<Confirmations, ChainSyncError>,
    ) -> Vec<(String, Exclusion)> {
        released
            .iter()
            .map(|r| {
                let unconfirmed = match (&r.txid, answer) {
                    (None, _) => Some("the vault log names no transaction for it".to_string()),
                    (Some(txid), Ok(answer)) => answer
                        .verdict(&Release {
                            nullifier: r.nullifier.clone(),
                            txid: txid.clone(),
                        })
                        .err(),
                    (Some(_), Err(error)) => Some(error.to_string()),
                };
                match &unconfirmed {
                    None => tracing::info!(return_id = %r.id, txid = ?r.txid, "release confirmed by a second source"),
                    Some(why) => tracing::warn!(return_id = %r.id, txid = ?r.txid, %why, "release not confirmed by a second source; the return waits"),
                }
                (
                    r.id.clone(),
                    Exclusion::AlreadySettled {
                        txid: r.txid.clone(),
                        unconfirmed,
                    },
                )
            })
            .collect()
    }

    /// The vault's accumulator, or `None` once the batch has been failed for want of it.
    /// A log that could not be read is read again on [`SYNC_RETRY_DELAYS`] and then fails
    /// the batch as `ServiceUnavailable`; a log that was read but is unusable fails it at
    /// once as `ChainRejected`.
    async fn synced(&self, batch: &Batch) -> Option<RebuiltAccumulator> {
        let mut delays = SYNC_RETRY_DELAYS.iter();
        loop {
            let known = self.store.read(|ledger| ledger.settled_batches());
            let error = match self.chain.synced_accumulator(known).await {
                Ok(acc) => return Some(acc),
                Err(e) => e,
            };
            match delays.next() {
                Some(delay) if error.is_unavailable() => {
                    tracing::warn!(
                        batch_id = %batch.id,
                        %error,
                        "chain log unavailable; reading it again in {}s",
                        delay.as_secs()
                    );
                    tokio::time::sleep(*delay).await;
                }
                _ => {
                    let kind = if error.is_unavailable() {
                        ErrorKind::ServiceUnavailable
                    } else {
                        ErrorKind::ChainRejected
                    };
                    self.fail(
                        batch,
                        kind,
                        format!("accumulator not synced to chain: {error}"),
                    );
                    return None;
                }
            }
        }
    }

    fn rebase_or_fail(&self, batch: &Batch, why: &str) -> Stage {
        if !batch.can_rebase(self.retry.max_rebases) {
            return self.fail(
                batch,
                ErrorKind::ChainRejected,
                format!("{why}; rebased {} times already", batch.rebases),
            );
        }
        tracing::warn!(batch_id = %batch.id, rebases = batch.rebases + 1, "{why}; re-proving on the new root");
        self.store.apply(Event::BatchRebased {
            id: batch.id.clone(),
            at_ms: self.clock.now_ms(),
        });
        Stage::Rebase
    }

    fn fail(&self, batch: &Batch, kind: ErrorKind, message: impl Into<String>) -> Stage {
        let message = message.into();
        tracing::error!(batch_id = %batch.id, kind = ?kind, %message, "batch failed");
        self.store.apply(Event::BatchFailed {
            id: batch.id.clone(),
            kind,
            message,
            at_ms: self.clock.now_ms(),
        });
        Stage::Done
    }

    fn exclude(&self, excluded: Vec<(String, Exclusion)>) {
        for (id, reason) in excluded {
            tracing::warn!(return_id = %id, ?reason, "return excluded from proving");
            self.store.apply(Event::ReturnExcluded {
                id,
                reason,
                at_ms: self.clock.now_ms(),
            });
        }
    }

    fn reload(&self, batch_id: &str) -> Option<Batch> {
        self.store.read(|ledger| ledger.batch(batch_id).cloned())
    }
}

/// Whether the vault's log shows every nullifier of the proven batch spent.
fn all_released(batch: &Batch, acc: &RebuiltAccumulator) -> bool {
    let nullifiers = batch.nullifiers();
    !nullifiers.is_empty()
        && nullifiers
            .iter()
            .all(|n| acc.tree.non_membership_witness(n).is_none())
}
