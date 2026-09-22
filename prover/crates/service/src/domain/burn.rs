use std::collections::HashSet;

use bridge_return_guest::GuestInput;
use serde::{Deserialize, Serialize};

use crate::store::{hex32, LeafHex, ReturnRecord, ReturnStatus};

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Burn {
    pub id: String,
    pub trust_base_hash: String,
    pub lock_nonces: Vec<u64>,
    pub leaf: LeafHex,
    #[serde(with = "super::hex_bytes")]
    pub wire_input: Vec<u8>,
}

impl Burn {
    pub fn from_input(id: String, input: &GuestInput, wire_input: Vec<u8>) -> Self {
        Self {
            id,
            trust_base_hash: hex32(&input.public_values.trust_base_hash),
            lock_nonces: input.sorted_lock_refs.iter().map(|r| r.nonce).collect(),
            leaf: LeafHex::from(input.return_leaves[0]),
            wire_input,
        }
    }

    pub fn nullifier(&self) -> &str {
        &self.leaf.nullifier
    }

    pub fn size(&self) -> usize {
        self.wire_input.len()
    }

    pub fn shares_trust_base(&self, other: &Burn) -> bool {
        self.trust_base_hash == other.trust_base_hash
    }

    pub fn nonces_disjoint(&self, taken: &HashSet<u64>) -> bool {
        self.lock_nonces.iter().all(|n| !taken.contains(n))
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Return {
    pub seq: u64,
    pub burn: Burn,
    pub record: ReturnRecord,
}

impl Return {
    pub fn is_pending(&self) -> bool {
        self.record.status == ReturnStatus::Queued || self.record.failed_recoverably()
    }

    pub fn is_due(&self, now_ms: u128) -> bool {
        self.record.not_before_ms.is_none_or(|t| now_ms >= t)
    }

    pub fn in_batch(&self, batch_id: &str) -> bool {
        self.record.batch_id.as_deref() == Some(batch_id)
            && matches!(
                self.record.status,
                ReturnStatus::Proving | ReturnStatus::Proven
            )
    }

    pub fn is_done(&self) -> bool {
        self.record.status == ReturnStatus::Settled
            || (self.record.status == ReturnStatus::Failed && !self.record.failed_recoverably())
    }
}

#[cfg(test)]
mod tests {
    use bridge_return_guest::wire;
    use bridge_return_host::fixture::build_b1_direct_bridge_fixture;

    use super::*;
    use crate::store::{ErrorKind, ReturnFailure};

    fn queued_return(not_before_ms: Option<u128>) -> Return {
        let fixture = build_b1_direct_bridge_fixture();
        let wire = wire::encode_guest_input(&fixture.input);
        let burn = Burn::from_input("r1".to_string(), &fixture.input, wire);
        let mut record = ReturnRecord::queued(
            "r1".to_string(),
            fixture.input.return_leaves[0].nullifier,
            [0; 32],
            fixture.input.public_values,
            Vec::new(),
            1_000,
        );
        record.not_before_ms = not_before_ms;
        Return {
            seq: 1,
            burn,
            record,
        }
    }

    #[test]
    fn burn_from_b1_fixture_reads_nonce_trust_base_and_leaf() {
        let fixture = build_b1_direct_bridge_fixture();
        let wire = wire::encode_guest_input(&fixture.input);
        let burn = Burn::from_input("r1".to_string(), &fixture.input, wire.clone());
        assert_eq!(burn.lock_nonces, vec![7]);
        assert_eq!(
            burn.trust_base_hash,
            hex32(&fixture.input.public_values.trust_base_hash)
        );
        assert_eq!(
            burn.nullifier(),
            hex32(&fixture.input.return_leaves[0].nullifier)
        );
        assert_eq!(burn.size(), wire.len());
        assert!(burn.nonces_disjoint(&HashSet::from([9])));
        assert!(!burn.nonces_disjoint(&HashSet::from([7])));
    }

    #[test]
    fn burn_round_trips_through_json() {
        let fixture = build_b1_direct_bridge_fixture();
        let wire = wire::encode_guest_input(&fixture.input);
        let burn = Burn::from_input("r1".to_string(), &fixture.input, wire);
        let json = serde_json::to_string(&burn).unwrap();
        assert_eq!(serde_json::from_str::<Burn>(&json).unwrap(), burn);
    }

    #[test]
    fn return_is_due_only_after_not_before() {
        assert!(queued_return(None).is_due(0));
        let scheduled = queued_return(Some(5_000));
        assert!(!scheduled.is_due(4_999));
        assert!(scheduled.is_due(5_000));
        assert!(scheduled.is_pending());
    }

    #[test]
    fn is_done_for_settled_and_final_failure() {
        let mut settled = queued_return(None);
        settled.record.transition(ReturnStatus::Settled, 2_000);
        assert!(settled.is_done());
        assert!(!settled.is_pending());

        let mut recoverable = queued_return(None);
        recoverable.record.failure =
            Some(ReturnFailure::recoverable(ErrorKind::ProvingFailed, "oom"));
        recoverable.record.transition(ReturnStatus::Failed, 2_000);
        assert!(!recoverable.is_done());
        assert!(recoverable.is_pending());

        let mut parked = queued_return(None);
        parked.record.failure = Some(ReturnFailure::terminal(ErrorKind::ProvingFailed, "parked"));
        parked.record.transition(ReturnStatus::Failed, 2_000);
        assert!(parked.is_done());
        assert!(!parked.is_pending());
    }

    #[test]
    fn in_batch_requires_the_batch_id_and_an_active_status() {
        let mut member = queued_return(None);
        assert!(!member.in_batch("b1"));
        member.record.batch_id = Some("b1".to_string());
        member.record.transition(ReturnStatus::Proving, 2_000);
        assert!(member.in_batch("b1"));
        assert!(!member.in_batch("b2"));
        member.record.transition(ReturnStatus::Settled, 3_000);
        assert!(!member.in_batch("b1"));
    }
}
