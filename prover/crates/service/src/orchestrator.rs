use std::time::Duration;

use bridge_return_host::s2::RebuiltAccumulator;

use crate::{
    clock::Clock,
    domain::{
        assembler::{assemble, AssembleError, Exclusion},
        batch::Batch,
        burn::{Burn, LeafToSettle},
        event::Event,
        policy::BatchPolicy,
        retry::RetryPolicy,
    },
    ports::{ChainLog, ProofBackend, Settler},
    shutdown::Drain,
    store::{hex32, BatchBundle, ErrorKind, ReturnStore, Taken},
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

pub struct Orchestrator<P, S, C> {
    store: ReturnStore,
    prover: P,
    settler: S,
    chain: C,
    policy: BatchPolicy,
    retry: RetryPolicy,
    clock: Clock,
}

#[derive(Debug, Eq, PartialEq)]
enum Stage {
    Done,
    Rebase,
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
        }
    }

    /// Prove and settle batches until `drain` starts, then return once the batch in flight
    /// has run to its end.
    pub async fn run(self, drain: Drain) {
        for batch in self.store.read(|ledger| ledger.unsettled_proofs()) {
            if drain.is_draining() {
                break;
            }
            self.run_batch(batch).await;
        }
        let mut collect = true;
        while !drain.is_draining() {
            match self
                .store
                .take_next(&self.policy, self.clock.now_ms(), collect)
            {
                Taken::Batch(batch) => {
                    self.run_batch(batch).await;
                    collect = false;
                }
                Taken::WaitUntil(at_ms) => {
                    tokio::select! {
                        _ = self.store.changed() => {}
                        _ = self.clock.sleep_until_ms(at_ms) => {}
                        _ = drain.started() => {}
                    }
                    collect = true;
                }
                Taken::Idle => {
                    tokio::select! {
                        _ = self.store.changed() => {}
                        _ = drain.started() => {}
                    }
                    collect = true;
                }
            }
        }
        tracing::info!("drained: no batch in flight, and none starts");
    }

    async fn run_batch(&self, batch: Batch) {
        let mut stage = if batch.bundle.is_some() {
            self.settle_stage(&batch).await
        } else {
            self.prove_stage(&batch).await
        };
        while stage == Stage::Rebase {
            let Some(current) = self.reload(&batch.id) else {
                return;
            };
            stage = self.prove_stage(&current).await;
        }
    }

    async fn prove_stage(&self, batch: &Batch) -> Stage {
        tracing::info!(batch_id = %batch.id, members = batch.members.len(), "batch proving started");
        let Some(acc) = self.synced(batch).await else {
            return Stage::Done;
        };
        let burns: Vec<Burn> = self
            .store
            .read(|ledger| ledger.burns(&batch.members).into_iter().cloned().collect());
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
        self.exclude(assembled.excluded.clone());
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
        let nullifiers = batch.nullifiers();
        let all_released = !nullifiers.is_empty()
            && nullifiers
                .iter()
                .all(|n| acc.tree.non_membership_witness(n).is_none());
        if all_released {
            tracing::info!(batch_id = %batch.id, "batch already released on chain");
            self.store.apply(Event::BatchSettled {
                id: batch.id.clone(),
                txid: None,
                at_ms: self.clock.now_ms(),
            });
            return Stage::Done;
        }
        if !batch.chains_onto(&acc.spent_root) {
            return self.rebase_or_fail(batch, "vault root moved after proving");
        }
        let Some(bundle) = &batch.bundle else {
            return Stage::Done;
        };
        match self.settler.submit(bundle).await {
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
            SubmitOutcome::StaleRoot => self.rebase_or_fail(batch, "vault: stale root"),
            SubmitOutcome::Failed { message } => {
                self.fail(batch, ErrorKind::SubmissionFailed, message)
            }
        }
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
            tracing::warn!(return_id = %id, ?reason, "return excluded from the batch");
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
