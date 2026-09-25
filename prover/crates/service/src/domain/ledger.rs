use std::collections::HashMap;

use bridge_return_host::s2::SettledBatch;
use serde::{Deserialize, Serialize};

use super::{
    assembler::Exclusion,
    batch::{Batch, BatchStatus},
    burn::{Burn, Return},
    event::Event,
    retry::RetryPolicy,
};
use crate::store::{BatchBundle, ErrorKind, ReturnFailure, ReturnRecord, ReturnStatus};

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Ledger {
    returns: HashMap<String, Return>,
    by_nullifier: HashMap<String, String>,
    batches: HashMap<String, Batch>,
    accepted: u64,
    formed: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Health {
    pub queue_depth: usize,
    pub active_batch: Option<String>,
    pub active_batch_size: usize,
    pub proving_since_ms: Option<u128>,
    pub last_proof_ms: Option<u128>,
    pub average_proof_ms: Option<u128>,
}

impl Ledger {
    pub fn apply(&mut self, event: Event, retry: &RetryPolicy) {
        match event {
            Event::Snapshot(ledger) => *self = *ledger,
            Event::Accepted { burn, record } => self.accept(burn, record),
            Event::Requeued { id, at_ms } => self.requeue(&id, at_ms, retry),
            Event::BatchFormed { members, at_ms } => self.form(members, at_ms),
            Event::BatchProven {
                id,
                bundle,
                spent_root_old,
                at_ms,
            } => self.prove(&id, bundle, spent_root_old, at_ms),
            Event::BatchSettled { id, txid, at_ms } => self.settle(&id, txid, at_ms),
            Event::BatchFailed {
                id,
                kind,
                message,
                at_ms,
            } => self.fail(&id, kind, &message, at_ms, retry),
            Event::BatchRebased { id, at_ms } => self.rebase(&id, at_ms),
            Event::BatchInterrupted { id, at_ms } => self.interrupt(&id, at_ms, retry),
            Event::ReturnExcluded { id, reason, at_ms } => self.exclude(&id, reason, at_ms, retry),
        }
    }

    pub fn get(&self, id: &str) -> Option<&Return> {
        self.returns.get(id)
    }

    pub fn by_nullifier(&self, nullifier: &str) -> Option<&Return> {
        self.by_nullifier
            .get(&nullifier.to_lowercase())
            .and_then(|id| self.returns.get(id))
    }

    pub fn record(&self, id: &str) -> Option<ReturnRecord> {
        let mut record = self.returns.get(id)?.record.clone();
        record.queue_position = self
            .pending()
            .iter()
            .position(|r| r.burn.id == id)
            .map(|p| p + 1);
        Some(record)
    }

    pub fn pending(&self) -> Vec<&Return> {
        let mut pending: Vec<&Return> = self.returns.values().filter(|r| r.is_pending()).collect();
        pending.sort_by_key(|r| r.seq);
        pending
    }

    pub fn burns(&self, ids: &[String]) -> Vec<&Burn> {
        ids.iter()
            .filter_map(|id| self.returns.get(id).map(|r| &r.burn))
            .collect()
    }

    pub fn batch(&self, id: &str) -> Option<&Batch> {
        self.batches.get(id)
    }

    pub fn active_batch(&self) -> Option<&Batch> {
        self.batches
            .values()
            .find(|b| b.status == BatchStatus::Proving)
    }

    pub fn settled_batches(&self) -> Vec<SettledBatch> {
        self.batches
            .values()
            .filter(|b| b.status == BatchStatus::Settled)
            .filter_map(Batch::settled_batch)
            .collect()
    }

    pub fn unsettled_proofs(&self) -> Vec<Batch> {
        let mut proofs: Vec<Batch> = self
            .batches
            .values()
            .filter(|b| b.is_unsettled_proof())
            .cloned()
            .collect();
        proofs.sort_by_key(|b| b.formed_at_ms);
        proofs
    }

    pub fn proof_for(&self, nullifiers: &[String], spent_root_old: &str) -> Option<BatchBundle> {
        let wanted: Vec<String> = nullifiers.iter().map(|n| n.to_lowercase()).collect();
        self.batches
            .values()
            .filter(|b| b.status != BatchStatus::Settled)
            .filter(|b| b.spent_root_old.as_deref() == Some(spent_root_old))
            .filter(|b| {
                b.bundle.as_ref().is_some_and(|bundle| {
                    bundle.has_proof()
                        && bundle
                            .leaves
                            .iter()
                            .map(|l| l.nullifier.to_lowercase())
                            .eq(wanted.iter().cloned())
                })
            })
            .max_by_key(|b| b.proven_at_ms)
            .and_then(|b| b.bundle.clone())
    }

    pub fn recover(&self, at_ms: u128) -> Vec<Event> {
        let mut proving: Vec<&Batch> = self
            .batches
            .values()
            .filter(|b| b.status == BatchStatus::Proving)
            .collect();
        proving.sort_by_key(|b| b.formed_at_ms);
        proving
            .into_iter()
            .map(|b| Event::BatchInterrupted {
                id: b.id.clone(),
                at_ms,
            })
            .collect()
    }

    pub fn health(&self) -> Health {
        let active = self.active_batch();
        let durations: Vec<u128> = self
            .batches
            .values()
            .filter_map(Batch::proof_duration_ms)
            .collect();
        Health {
            queue_depth: self.pending().len(),
            active_batch: active.map(|b| b.id.clone()),
            active_batch_size: active.map_or(0, |b| b.members.len()),
            proving_since_ms: active.map(|b| b.formed_at_ms),
            last_proof_ms: self
                .batches
                .values()
                .filter(|b| b.proven_at_ms.is_some())
                .max_by_key(|b| b.proven_at_ms)
                .and_then(Batch::proof_duration_ms),
            average_proof_ms: (!durations.is_empty())
                .then(|| durations.iter().sum::<u128>() / durations.len() as u128),
        }
    }

    fn accept(&mut self, burn: Burn, record: ReturnRecord) {
        let nullifier = record.nullifier.to_lowercase();
        if self.by_nullifier.contains_key(&nullifier) {
            tracing::warn!(id = %burn.id, "accepted event for a known nullifier ignored");
            return;
        }
        self.accepted += 1;
        self.by_nullifier.insert(nullifier, burn.id.clone());
        self.returns.insert(
            burn.id.clone(),
            Return {
                seq: self.accepted,
                burn,
                record,
            },
        );
    }

    fn requeue(&mut self, id: &str, at_ms: u128, retry: &RetryPolicy) {
        let Some(entry) = self.returns.get_mut(id) else {
            tracing::warn!(id, "requeue for an unknown return ignored");
            return;
        };
        if !entry.record.failed_recoverably() {
            tracing::warn!(
                id,
                "requeue for a return that is not recoverably failed ignored"
            );
            return;
        }
        let record = &mut entry.record;
        record.failure = None;
        let message = match record.not_before_ms {
            Some(t) if t > at_ms => format!(
                "Return re-queued; retry {} of {} scheduled at {t}",
                record.attempts + 1,
                retry.max_attempts
            ),
            _ => "Return re-queued for the next proving batch".to_string(),
        };
        record.transition_with(ReturnStatus::Queued, at_ms, &message);
    }

    fn form(&mut self, members: Vec<String>, at_ms: u128) {
        self.formed += 1;
        let batch = Batch::formed(self.formed, members, at_ms);
        for id in &batch.members {
            match self.returns.get_mut(id) {
                Some(entry) if entry.is_pending() => {
                    entry.record.batch_id = Some(batch.id.clone());
                    entry.record.failure = None;
                    entry.record.transition(ReturnStatus::Proving, at_ms);
                }
                _ => tracing::warn!(id, batch_id = %batch.id, "batch member is not pending"),
            }
        }
        self.batches.insert(batch.id.clone(), batch);
    }

    fn prove(&mut self, id: &str, bundle: BatchBundle, spent_root_old: String, at_ms: u128) {
        let Some(batch) = self.batches.get_mut(id) else {
            tracing::warn!(batch_id = id, "proven event for an unknown batch ignored");
            return;
        };
        batch.status = BatchStatus::Proven;
        batch.bundle = Some(bundle);
        batch.spent_root_old = Some(spent_root_old);
        batch.proven_at_ms = Some(at_ms);
        for entry in self.members_of(id) {
            entry.record.transition(ReturnStatus::Proven, at_ms);
        }
    }

    fn settle(&mut self, id: &str, txid: Option<String>, at_ms: u128) {
        let Some(batch) = self.batches.get_mut(id) else {
            tracing::warn!(batch_id = id, "settled event for an unknown batch ignored");
            return;
        };
        batch.status = BatchStatus::Settled;
        if let Some(bundle) = &mut batch.bundle {
            bundle.settle_txid = txid.clone();
        }
        for entry in self.members_of(id) {
            entry.record.settle_txid = txid.clone();
            match txid {
                Some(_) => entry.record.transition(ReturnStatus::Settled, at_ms),
                None => entry.record.transition_with(
                    ReturnStatus::Settled,
                    at_ms,
                    "Return settled on the source chain (txid not recorded)",
                ),
            }
        }
    }

    fn fail(&mut self, id: &str, kind: ErrorKind, message: &str, at_ms: u128, retry: &RetryPolicy) {
        let Some(batch) = self.batches.get_mut(id) else {
            tracing::warn!(batch_id = id, "failed event for an unknown batch ignored");
            return;
        };
        batch.status = BatchStatus::Failed;
        for entry in self.members_of(id) {
            schedule_retry(&mut entry.record, kind.clone(), message, at_ms, retry);
        }
    }

    fn rebase(&mut self, id: &str, at_ms: u128) {
        let Some(batch) = self.batches.get_mut(id) else {
            tracing::warn!(batch_id = id, "rebase event for an unknown batch ignored");
            return;
        };
        batch.rebases += 1;
        batch.bundle = None;
        batch.spent_root_old = None;
        batch.proven_at_ms = None;
        batch.status = BatchStatus::Proving;
        for entry in self.members_of(id) {
            entry.record.transition_with(
                ReturnStatus::Proving,
                at_ms,
                "Vault root moved; re-proving the batch on the new root",
            );
        }
    }

    fn interrupt(&mut self, id: &str, at_ms: u128, retry: &RetryPolicy) {
        let Some(batch) = self.batches.get_mut(id) else {
            tracing::warn!(
                batch_id = id,
                "interrupted event for an unknown batch ignored"
            );
            return;
        };
        batch.status = BatchStatus::Interrupted;
        for entry in self.members_of(id) {
            if entry.record.status == ReturnStatus::Proving {
                schedule_after_interruption(&mut entry.record, at_ms, retry);
            }
        }
    }

    fn exclude(&mut self, id: &str, reason: Exclusion, at_ms: u128, retry: &RetryPolicy) {
        let Some(entry) = self.returns.get_mut(id) else {
            tracing::warn!(id, "exclusion for an unknown return ignored");
            return;
        };
        let record = &mut entry.record;
        match reason {
            Exclusion::AlreadySettled => record.transition_with(
                ReturnStatus::Settled,
                at_ms,
                "Nullifier already released on chain",
            ),
            Exclusion::Rejected { message } => schedule_retry(
                record,
                ErrorKind::SubmissionFailed,
                &format!("recipient rejected: {message}"),
                at_ms,
                retry,
            ),
            Exclusion::Invalid { message } => {
                record.failure = Some(ReturnFailure::terminal(ErrorKind::ChainRejected, message));
                record.transition(ReturnStatus::Failed, at_ms);
            }
        }
    }

    fn members_of(&mut self, batch_id: &str) -> impl Iterator<Item = &mut Return> {
        let ids = self
            .batches
            .get(batch_id)
            .map(|b| b.members.clone())
            .unwrap_or_default();
        let batch_id = batch_id.to_string();
        self.returns
            .values_mut()
            .filter(move |r| ids.contains(&r.burn.id) && r.in_batch(&batch_id))
    }
}

fn schedule_after_interruption(record: &mut ReturnRecord, at_ms: u128, retry: &RetryPolicy) {
    record.attempts += 1;
    if retry.parked(record.attempts) {
        record.not_before_ms = None;
        record.failure = Some(ReturnFailure::terminal(
            ErrorKind::ProvingFailed,
            format!(
                "proof interrupted by a service restart; parked after {} attempts, needs operator attention",
                record.attempts
            ),
        ));
        record.transition(ReturnStatus::Failed, at_ms);
        return;
    }
    record.not_before_ms = Some(retry.not_before(record.attempts, at_ms));
    record.failure = None;
    record.transition_with(
        ReturnStatus::Queued,
        at_ms,
        &format!(
            "Proof interrupted by a service restart; retry {} of {} scheduled",
            record.attempts + 1,
            retry.max_attempts
        ),
    );
}

fn schedule_retry(
    record: &mut ReturnRecord,
    kind: ErrorKind,
    message: &str,
    at_ms: u128,
    retry: &RetryPolicy,
) {
    record.attempts += 1;
    if retry.parked(record.attempts) {
        record.not_before_ms = None;
        record.failure = Some(ReturnFailure::terminal(
            kind,
            format!(
                "{message}; parked after {} attempts, needs operator attention",
                record.attempts
            ),
        ));
    } else {
        record.not_before_ms = Some(retry.not_before(record.attempts, at_ms));
        record.failure = Some(ReturnFailure::recoverable(
            kind,
            format!(
                "{message} (attempt {} of {}, retry scheduled)",
                record.attempts, retry.max_attempts
            ),
        ));
    }
    record.transition(ReturnStatus::Failed, at_ms);
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use bridge_return_core::public_values_abi;

    use bridge_return_core::PublicValues;

    use super::*;
    use crate::store::{hex32, LeafHex};

    const MINUTE: u128 = 60_000;

    fn retry() -> RetryPolicy {
        RetryPolicy {
            base: Duration::from_secs(60),
            max_attempts: 2,
            max_rebases: 3,
        }
    }

    fn leaf(nullifier: [u8; 32]) -> LeafHex {
        LeafHex {
            nullifier: hex32(&nullifier),
            recipient: "0x".to_string(),
            amount: "0x".to_string(),
            fee_recipient: "0x".to_string(),
            fee_amount: "0x".to_string(),
            deadline: "0x0".to_string(),
        }
    }

    fn accepted(id: &str, nullifier_byte: u8, nonce: u64, at_ms: u128) -> Event {
        let nullifier = [nullifier_byte; 32];
        let public_values = PublicValues {
            domain_tag: [0; 32],
            config_hash: [0; 32],
            trust_base_hash: [1; 32],
            spent_root_old: [0; 32],
            spent_root_new: [0; 32],
            return_root: [0; 32],
            lock_ref_root: [0; 32],
            batch_size: 1,
            total_amount: [0; 32],
        };
        Event::Accepted {
            burn: Burn {
                id: id.to_string(),
                trust_base_hash: hex32(&[1; 32]),
                lock_nonces: vec![nonce],
                leaf: leaf(nullifier),
                wire_input: vec![nullifier_byte; 8],
            },
            record: ReturnRecord::queued(id.to_string(), nullifier, [0; 32], public_values, at_ms),
        }
    }

    fn bundle(batch_id: &str, nullifiers: &[[u8; 32]]) -> BatchBundle {
        BatchBundle {
            batch_id: batch_id.to_string(),
            mode: "scripted".to_string(),
            vkey: None,
            public_values: "0xab".to_string(),
            proof_bytes: "0x01".to_string(),
            settle_txid: None,
            leaves: nullifiers.iter().copied().map(leaf).collect(),
            lock_refs: Vec::new(),
        }
    }

    fn ledger_with(events: Vec<Event>) -> Ledger {
        let mut ledger = Ledger::default();
        for event in events {
            ledger.apply(event, &retry());
        }
        ledger
    }

    fn formed(ledger: &mut Ledger, members: &[&str], at_ms: u128) -> String {
        ledger.apply(
            Event::BatchFormed {
                members: members.iter().map(|m| m.to_string()).collect(),
                at_ms,
            },
            &retry(),
        );
        ledger.active_batch().unwrap().id.clone()
    }

    fn proven(ledger: &mut Ledger, batch_id: &str, nullifiers: &[[u8; 32]], at_ms: u128) {
        ledger.apply(
            Event::BatchProven {
                id: batch_id.to_string(),
                bundle: bundle(batch_id, nullifiers),
                spent_root_old: hex32(&[0; 32]),
                at_ms,
            },
            &retry(),
        );
    }

    fn failed(ledger: &mut Ledger, batch_id: &str, at_ms: u128) {
        ledger.apply(
            Event::BatchFailed {
                id: batch_id.to_string(),
                kind: ErrorKind::ProvingFailed,
                message: "oom".to_string(),
                at_ms,
            },
            &retry(),
        );
    }

    fn status(ledger: &Ledger, id: &str) -> ReturnStatus {
        ledger.get(id).unwrap().record.status.clone()
    }

    #[test]
    fn settled_batches_carry_the_roots_of_their_public_values() {
        let public_values = PublicValues {
            domain_tag: [0xd; 32],
            config_hash: [0xc; 32],
            trust_base_hash: [1; 32],
            spent_root_old: [0; 32],
            spent_root_new: [7; 32],
            return_root: [2; 32],
            lock_ref_root: [3; 32],
            batch_size: 1,
            total_amount: [0; 32],
        };
        let mut ledger = ledger_with(vec![accepted("a", 0xA, 1, 100), accepted("b", 0xB, 2, 200)]);
        let first = formed(&mut ledger, &["a"], 300);
        let mut settled_bundle = bundle(&first, &[[0xA; 32]]);
        settled_bundle.public_values =
            format!("0x{}", hex::encode(public_values_abi(&public_values)));
        ledger.apply(
            Event::BatchProven {
                id: first.clone(),
                bundle: settled_bundle,
                spent_root_old: hex32(&[0; 32]),
                at_ms: 400,
            },
            &retry(),
        );
        ledger.apply(
            Event::BatchSettled {
                id: first,
                txid: Some("0xfeed".to_string()),
                at_ms: 500,
            },
            &retry(),
        );
        let second = formed(&mut ledger, &["b"], 600);
        proven(&mut ledger, &second, &[[0xB; 32]], 700);

        let settled = ledger.settled_batches();
        assert_eq!(settled.len(), 1);
        assert_eq!(settled[0].nullifiers, vec![[0xA; 32]]);
        assert_eq!(settled[0].spent_root_old, [0; 32]);
        assert_eq!(settled[0].spent_root_new, [7; 32]);
    }

    #[test]
    fn accepted_return_is_pending_with_position() {
        let ledger = ledger_with(vec![accepted("a", 0xA, 1, 100), accepted("b", 0xB, 2, 200)]);
        assert_eq!(ledger.pending().len(), 2);
        assert_eq!(ledger.record("a").unwrap().queue_position, Some(1));
        assert_eq!(ledger.record("b").unwrap().queue_position, Some(2));
        assert_eq!(
            ledger
                .by_nullifier(&hex32(&[0xB; 32]).to_uppercase())
                .unwrap()
                .burn
                .id,
            "b"
        );
        assert_eq!(ledger.get("a").unwrap().seq, 1);
    }

    #[test]
    fn accepted_twice_is_ignored() {
        let ledger = ledger_with(vec![
            accepted("a", 0xA, 1, 100),
            accepted("a2", 0xA, 1, 200),
        ]);
        assert_eq!(ledger.pending().len(), 1);
        assert!(ledger.get("a2").is_none());
    }

    #[test]
    fn formed_marks_members_proving_and_assigns_batch_id() {
        let mut ledger = ledger_with(vec![accepted("a", 0xA, 1, 100), accepted("b", 0xB, 2, 200)]);
        let batch_id = formed(&mut ledger, &["a", "b"], 300);
        for id in ["a", "b"] {
            let record = ledger.record(id).unwrap();
            assert_eq!(record.status, ReturnStatus::Proving);
            assert_eq!(record.batch_id.as_deref(), Some(batch_id.as_str()));
            assert_eq!(record.queue_position, None);
        }
        assert!(ledger.pending().is_empty());
        assert_eq!(ledger.batch(&batch_id).unwrap().members, vec!["a", "b"]);
    }

    #[test]
    fn proven_stores_bundle_and_root() {
        let mut ledger = ledger_with(vec![accepted("a", 0xA, 1, 100)]);
        let batch_id = formed(&mut ledger, &["a"], 300);
        proven(&mut ledger, &batch_id, &[[0xA; 32]], 400);
        let batch = ledger.batch(&batch_id).unwrap();
        assert_eq!(batch.status, BatchStatus::Proven);
        assert_eq!(
            batch.spent_root_old.as_deref(),
            Some(hex32(&[0; 32]).as_str())
        );
        assert_eq!(batch.proven_at_ms, Some(400));
        assert_eq!(batch.nullifiers(), vec![[0xA; 32]]);
        assert_eq!(status(&ledger, "a"), ReturnStatus::Proven);
        assert_eq!(ledger.unsettled_proofs().len(), 1);
    }

    #[test]
    fn settled_marks_members_and_txid() {
        let mut ledger = ledger_with(vec![accepted("a", 0xA, 1, 100)]);
        let batch_id = formed(&mut ledger, &["a"], 300);
        proven(&mut ledger, &batch_id, &[[0xA; 32]], 400);
        ledger.apply(
            Event::BatchSettled {
                id: batch_id.clone(),
                txid: Some("0xfeed".to_string()),
                at_ms: 500,
            },
            &retry(),
        );
        let record = ledger.record("a").unwrap();
        assert_eq!(record.status, ReturnStatus::Settled);
        assert_eq!(record.settle_txid.as_deref(), Some("0xfeed"));
        assert!(record.terminal);
        let batch = ledger.batch(&batch_id).unwrap();
        assert_eq!(batch.status, BatchStatus::Settled);
        assert_eq!(
            batch.bundle.as_ref().unwrap().settle_txid.as_deref(),
            Some("0xfeed")
        );
        assert!(ledger.unsettled_proofs().is_empty());
    }

    #[test]
    fn settled_without_txid_says_so() {
        let mut ledger = ledger_with(vec![accepted("a", 0xA, 1, 100)]);
        let batch_id = formed(&mut ledger, &["a"], 300);
        proven(&mut ledger, &batch_id, &[[0xA; 32]], 400);
        ledger.apply(
            Event::BatchSettled {
                id: batch_id,
                txid: None,
                at_ms: 500,
            },
            &retry(),
        );
        let record = ledger.record("a").unwrap();
        assert_eq!(record.status, ReturnStatus::Settled);
        assert_eq!(record.settle_txid, None);
        assert!(record.message.contains("txid not recorded"));
    }

    #[test]
    fn failed_schedules_retry_with_backoff() {
        let mut ledger = ledger_with(vec![accepted("a", 0xA, 1, 100), accepted("b", 0xB, 2, 200)]);
        let batch_id = formed(&mut ledger, &["a", "b"], 300);
        failed(&mut ledger, &batch_id, 1_000);
        for id in ["a", "b"] {
            let record = ledger.record(id).unwrap();
            assert_eq!(record.status, ReturnStatus::Failed);
            assert_eq!(record.attempts, 1);
            assert_eq!(record.not_before_ms, Some(1_000 + MINUTE));
            let failure = record.failure.unwrap();
            assert!(failure.recoverable);
            assert_eq!(failure.kind, ErrorKind::ProvingFailed);
            assert!(failure.message.contains("attempt 1 of 2"));
        }
        assert_eq!(ledger.batch(&batch_id).unwrap().status, BatchStatus::Failed);
        assert_eq!(ledger.pending().len(), 2);
        assert!(!ledger.get("a").unwrap().is_due(1_000 + MINUTE - 1));
    }

    #[test]
    fn failed_parks_after_max_attempts() {
        let mut ledger = ledger_with(vec![accepted("a", 0xA, 1, 100)]);
        let first = formed(&mut ledger, &["a"], 300);
        failed(&mut ledger, &first, 1_000);
        let second = formed(&mut ledger, &["a"], 2_000);
        failed(&mut ledger, &second, 3_000);
        let record = ledger.record("a").unwrap();
        assert_eq!(record.attempts, 2);
        assert_eq!(record.not_before_ms, None);
        let failure = record.failure.unwrap();
        assert!(!failure.recoverable);
        assert!(failure.message.contains("parked after 2 attempts"));
        assert!(ledger.pending().is_empty());
        assert!(ledger.get("a").unwrap().is_done());
    }

    #[test]
    fn failed_skips_done_members() {
        let mut ledger = ledger_with(vec![accepted("a", 0xA, 1, 100), accepted("b", 0xB, 2, 200)]);
        let batch_id = formed(&mut ledger, &["a", "b"], 300);
        ledger.apply(
            Event::ReturnExcluded {
                id: "a".to_string(),
                reason: Exclusion::AlreadySettled,
                at_ms: 400,
            },
            &retry(),
        );
        failed(&mut ledger, &batch_id, 1_000);
        assert_eq!(status(&ledger, "a"), ReturnStatus::Settled);
        assert_eq!(ledger.record("a").unwrap().attempts, 0);
        assert_eq!(status(&ledger, "b"), ReturnStatus::Failed);
    }

    #[test]
    fn requeued_keeps_schedule_and_attempts() {
        let mut ledger = ledger_with(vec![accepted("a", 0xA, 1, 100)]);
        let batch_id = formed(&mut ledger, &["a"], 300);
        failed(&mut ledger, &batch_id, 1_000);
        ledger.apply(
            Event::Requeued {
                id: "a".to_string(),
                at_ms: 2_000,
            },
            &retry(),
        );
        let record = ledger.record("a").unwrap();
        assert_eq!(record.status, ReturnStatus::Queued);
        assert_eq!(record.attempts, 1);
        assert_eq!(record.not_before_ms, Some(1_000 + MINUTE));
        assert_eq!(record.failure, None);
        assert!(record.message.contains("retry 2 of 2"));
        assert_eq!(record.queue_position, Some(1));
    }

    #[test]
    fn requeued_ignored_unless_recoverably_failed() {
        let mut ledger = ledger_with(vec![accepted("a", 0xA, 1, 100)]);
        let batch_id = formed(&mut ledger, &["a"], 300);
        ledger.apply(
            Event::Requeued {
                id: "a".to_string(),
                at_ms: 400,
            },
            &retry(),
        );
        assert_eq!(status(&ledger, "a"), ReturnStatus::Proving);
        assert_eq!(
            ledger.batch(&batch_id).unwrap().status,
            BatchStatus::Proving
        );
    }

    #[test]
    fn rebased_increments_and_clears_bundle() {
        let mut ledger = ledger_with(vec![accepted("a", 0xA, 1, 100)]);
        let batch_id = formed(&mut ledger, &["a"], 300);
        proven(&mut ledger, &batch_id, &[[0xA; 32]], 400);
        ledger.apply(
            Event::BatchRebased {
                id: batch_id.clone(),
                at_ms: 500,
            },
            &retry(),
        );
        let batch = ledger.batch(&batch_id).unwrap();
        assert_eq!(batch.rebases, 1);
        assert_eq!(batch.bundle, None);
        assert_eq!(batch.spent_root_old, None);
        assert_eq!(batch.status, BatchStatus::Proving);
        assert!(batch.can_rebase(3));
        let record = ledger.record("a").unwrap();
        assert_eq!(record.status, ReturnStatus::Proving);
        assert!(record.message.contains("root moved"));
        assert_eq!(record.batch_id.as_deref(), Some(batch_id.as_str()));
    }

    #[test]
    fn interrupted_requeues_with_backoff_and_parks_after_max_attempts() {
        let mut ledger = ledger_with(vec![accepted("a", 0xA, 1, 100)]);
        let first = formed(&mut ledger, &["a"], 300);
        ledger.apply(
            Event::BatchInterrupted {
                id: first.clone(),
                at_ms: 500,
            },
            &retry(),
        );
        let record = ledger.record("a").unwrap();
        assert_eq!(record.status, ReturnStatus::Queued);
        assert_eq!(record.attempts, 1);
        assert_eq!(record.not_before_ms, Some(500 + MINUTE));
        assert_eq!(record.failure, None);
        assert!(record.message.contains("retry 2 of 2"));
        assert_eq!(record.queue_position, Some(1));
        assert_eq!(
            ledger.batch(&first).unwrap().status,
            BatchStatus::Interrupted
        );
        assert!(ledger.active_batch().is_none());

        let second = formed(&mut ledger, &["a"], 70_000);
        ledger.apply(
            Event::BatchInterrupted {
                id: second,
                at_ms: 71_000,
            },
            &retry(),
        );
        let record = ledger.record("a").unwrap();
        assert_eq!(record.status, ReturnStatus::Failed);
        assert_eq!(record.attempts, 2);
        let failure = record.failure.unwrap();
        assert!(!failure.recoverable);
        assert!(failure.message.contains("interrupted"));
        assert!(ledger.pending().is_empty());
    }

    #[test]
    fn excluded_already_settled_marks_settled() {
        let mut ledger = ledger_with(vec![accepted("a", 0xA, 1, 100)]);
        formed(&mut ledger, &["a"], 300);
        ledger.apply(
            Event::ReturnExcluded {
                id: "a".to_string(),
                reason: Exclusion::AlreadySettled,
                at_ms: 400,
            },
            &retry(),
        );
        let record = ledger.record("a").unwrap();
        assert_eq!(record.status, ReturnStatus::Settled);
        assert_eq!(record.settle_txid, None);
        assert_eq!(record.success, Some(true));
        assert!(record.message.contains("already released"));
    }

    #[test]
    fn excluded_rejected_counts_an_attempt() {
        let mut ledger = ledger_with(vec![accepted("a", 0xA, 1, 100)]);
        formed(&mut ledger, &["a"], 300);
        ledger.apply(
            Event::ReturnExcluded {
                id: "a".to_string(),
                reason: Exclusion::Rejected {
                    message: "blocked".to_string(),
                },
                at_ms: 400,
            },
            &retry(),
        );
        let record = ledger.record("a").unwrap();
        assert_eq!(record.status, ReturnStatus::Failed);
        assert_eq!(record.attempts, 1);
        assert_eq!(record.not_before_ms, Some(400 + MINUTE));
        let failure = record.failure.unwrap();
        assert!(failure.recoverable);
        assert_eq!(failure.kind, ErrorKind::SubmissionFailed);
        assert!(failure.message.contains("blocked"));
    }

    #[test]
    fn excluded_invalid_is_final() {
        let mut ledger = ledger_with(vec![accepted("a", 0xA, 1, 100)]);
        formed(&mut ledger, &["a"], 300);
        ledger.apply(
            Event::ReturnExcluded {
                id: "a".to_string(),
                reason: Exclusion::Invalid {
                    message: "corrupt".to_string(),
                },
                at_ms: 400,
            },
            &retry(),
        );
        let record = ledger.record("a").unwrap();
        assert_eq!(record.status, ReturnStatus::Failed);
        let failure = record.failure.unwrap();
        assert!(!failure.recoverable);
        assert_eq!(failure.kind, ErrorKind::ChainRejected);
        assert!(ledger.get("a").unwrap().is_done());
    }

    #[test]
    fn recover_emits_interrupted_for_proving_batches_only() {
        let mut ledger = ledger_with(vec![accepted("a", 0xA, 1, 100), accepted("b", 0xB, 2, 200)]);
        let first = formed(&mut ledger, &["a"], 300);
        proven(&mut ledger, &first, &[[0xA; 32]], 400);
        let second = formed(&mut ledger, &["b"], 500);
        let events = ledger.recover(600);
        assert!(matches!(
            events.as_slice(),
            [Event::BatchInterrupted { id, at_ms: 600 }] if *id == second
        ));
        assert_eq!(ledger.unsettled_proofs()[0].id, first);
    }

    #[test]
    fn proof_for_matches_nullifiers_and_root() {
        let mut ledger = ledger_with(vec![accepted("a", 0xA, 1, 100), accepted("b", 0xB, 2, 200)]);
        let batch_id = formed(&mut ledger, &["a", "b"], 300);
        proven(&mut ledger, &batch_id, &[[0xA; 32], [0xB; 32]], 400);
        failed(&mut ledger, &batch_id, 500);
        let wanted = vec![hex32(&[0xA; 32]).to_uppercase(), hex32(&[0xB; 32])];
        assert!(ledger.proof_for(&wanted, &hex32(&[0; 32])).is_some());
        assert!(ledger.proof_for(&wanted, &hex32(&[9; 32])).is_none());
        assert!(ledger
            .proof_for(&[hex32(&[0xB; 32]), hex32(&[0xA; 32])], &hex32(&[0; 32]))
            .is_none());
        ledger.apply(
            Event::BatchSettled {
                id: batch_id,
                txid: None,
                at_ms: 600,
            },
            &retry(),
        );
        assert!(ledger.proof_for(&wanted, &hex32(&[0; 32])).is_none());
    }

    #[test]
    fn proof_for_ignores_a_bundle_without_proof_bytes() {
        let mut ledger = ledger_with(vec![accepted("a", 0xA, 1, 100)]);
        let batch_id = formed(&mut ledger, &["a"], 300);
        let mut precheck = bundle(&batch_id, &[[0xA; 32]]);
        precheck.proof_bytes = "0x".to_string();
        ledger.apply(
            Event::BatchProven {
                id: batch_id.clone(),
                bundle: precheck,
                spent_root_old: hex32(&[0; 32]),
                at_ms: 400,
            },
            &retry(),
        );
        failed(&mut ledger, &batch_id, 500);
        assert!(ledger
            .proof_for(&[hex32(&[0xA; 32])], &hex32(&[0; 32]))
            .is_none());
    }

    #[test]
    fn health_reports_depth_active_batch_and_proof_times() {
        let mut ledger = ledger_with(vec![
            accepted("a", 0xA, 1, 100),
            accepted("b", 0xB, 2, 200),
            accepted("c", 0xC, 3, 300),
            accepted("d", 0xD, 4, 400),
        ]);
        assert_eq!(ledger.health().average_proof_ms, None);
        let first = formed(&mut ledger, &["a"], 300);
        proven(&mut ledger, &first, &[[0xA; 32]], 3_300);
        let second = formed(&mut ledger, &["b"], 4_000);
        proven(&mut ledger, &second, &[[0xB; 32]], 9_000);
        let third = formed(&mut ledger, &["c"], 10_000);
        assert_eq!(
            ledger.health(),
            Health {
                queue_depth: 1,
                active_batch: Some(third),
                active_batch_size: 1,
                proving_since_ms: Some(10_000),
                last_proof_ms: Some(5_000),
                average_proof_ms: Some(4_000),
            }
        );
    }

    #[test]
    fn folding_events_equals_snapshot_roundtrip() {
        let mut ledger = ledger_with(vec![accepted("a", 0xA, 1, 100), accepted("b", 0xB, 2, 200)]);
        let batch_id = formed(&mut ledger, &["a", "b"], 300);
        proven(&mut ledger, &batch_id, &[[0xA; 32], [0xB; 32]], 400);
        failed(&mut ledger, &batch_id, 500);
        let snapshot = serde_json::to_string(&Event::Snapshot(Box::new(ledger.clone()))).unwrap();
        let mut restored = Ledger::default();
        restored.apply(serde_json::from_str(&snapshot).unwrap(), &retry());
        assert_eq!(
            serde_json::to_value(&restored).unwrap(),
            serde_json::to_value(&ledger).unwrap()
        );
        assert_eq!(restored.record("a"), ledger.record("a"));
        assert_eq!(restored.batch(&batch_id), ledger.batch(&batch_id));
    }
}
