use bridge_return_core::PublicValues;
use bridge_return_host::s2::SettledBatch;
use serde::{Deserialize, Serialize};
use sha2::Digest;

use crate::store::{hex32, hex_bytes, parse_hex32, BatchBundle};

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Batch {
    pub id: String,
    pub members: Vec<String>,
    pub status: BatchStatus,
    pub rebases: u32,
    pub bundle: Option<BatchBundle>,
    pub spent_root_old: Option<String>,
    pub formed_at_ms: u128,
    pub proven_at_ms: Option<u128>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BatchStatus {
    Proving,
    Proven,
    Settled,
    Failed,
    Interrupted,
}

impl Batch {
    pub fn formed(seq: u64, members: Vec<String>, at_ms: u128) -> Self {
        let mut hasher = sha2::Sha256::new();
        hasher.update(b"bridge-return-service:batch-id:v1");
        hasher.update(seq.to_be_bytes());
        for id in &members {
            hasher.update(id.as_bytes());
        }
        Self {
            id: format!("0x{}", hex::encode(hasher.finalize())),
            members,
            status: BatchStatus::Proving,
            rebases: 0,
            bundle: None,
            spent_root_old: None,
            formed_at_ms: at_ms,
            proven_at_ms: None,
        }
    }

    pub fn can_rebase(&self, max: u32) -> bool {
        self.rebases < max
    }

    pub fn chains_onto(&self, root: &[u8; 32]) -> bool {
        self.spent_root_old.as_deref() == Some(hex32(root).as_str())
    }

    pub fn nullifiers(&self) -> Vec<[u8; 32]> {
        self.bundle
            .iter()
            .flat_map(|b| b.leaves.iter())
            .filter_map(|l| parse_hex32(&l.nullifier))
            .collect()
    }

    pub fn settled_batch(&self) -> Option<SettledBatch> {
        let bundle = self.bundle.as_ref()?;
        let public_values = PublicValues::from_abi(&hex_bytes(&bundle.public_values)?)?;
        Some(SettledBatch {
            nullifiers: self.nullifiers(),
            spent_root_old: public_values.spent_root_old,
            spent_root_new: public_values.spent_root_new,
        })
    }

    pub fn is_unsettled_proof(&self) -> bool {
        self.status == BatchStatus::Proven
            && self
                .bundle
                .as_ref()
                .is_some_and(|b| b.settle_txid.is_none())
    }

    pub fn proof_duration_ms(&self) -> Option<u128> {
        self.proven_at_ms
            .map(|t| t.saturating_sub(self.formed_at_ms))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::LeafHex;

    fn bundle(nullifiers: &[[u8; 32]]) -> BatchBundle {
        BatchBundle {
            batch_id: "b".to_string(),
            mode: "scripted".to_string(),
            vkey: None,
            public_values: "0x".to_string(),
            proof_bytes: "0x01".to_string(),
            settle_txid: None,
            leaves: nullifiers
                .iter()
                .map(|n| LeafHex {
                    nullifier: hex32(n),
                    recipient: "0x".to_string(),
                    amount: "0x".to_string(),
                    fee_recipient: "0x".to_string(),
                    fee_amount: "0x".to_string(),
                    deadline: "0x0".to_string(),
                })
                .collect(),
            lock_refs: Vec::new(),
        }
    }

    #[test]
    fn batch_id_differs_by_seq_for_same_members() {
        let members = vec!["r1".to_string(), "r2".to_string()];
        let first = Batch::formed(1, members.clone(), 0);
        let second = Batch::formed(2, members, 0);
        assert_ne!(first.id, second.id);
        assert_eq!(first.id.len(), 66);
        assert_eq!(first.status, BatchStatus::Proving);
    }

    #[test]
    fn can_rebase_is_bounded() {
        let mut batch = Batch::formed(1, vec!["r1".to_string()], 0);
        assert!(batch.can_rebase(1));
        batch.rebases = 1;
        assert!(!batch.can_rebase(1));
    }

    #[test]
    fn nullifiers_come_from_bundle_leaves() {
        let mut batch = Batch::formed(1, vec!["r1".to_string()], 0);
        assert!(batch.nullifiers().is_empty());
        assert!(!batch.is_unsettled_proof());
        batch.status = BatchStatus::Proven;
        batch.bundle = Some(bundle(&[[1; 32], [2; 32]]));
        assert_eq!(batch.nullifiers(), vec![[1; 32], [2; 32]]);
        assert!(batch.is_unsettled_proof());
    }

    #[test]
    fn chains_onto_compares_the_recorded_root() {
        let mut batch = Batch::formed(1, vec!["r1".to_string()], 0);
        assert!(!batch.chains_onto(&[0; 32]));
        batch.spent_root_old = Some(hex32(&[7; 32]));
        assert!(batch.chains_onto(&[7; 32]));
        assert!(!batch.chains_onto(&[8; 32]));
    }
}
