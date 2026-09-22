use std::collections::HashSet;

use bridge_return_core::{
    lock_ref_root, return_root, sum_amounts, BridgeConfig, PublicValues, ReturnLeaf, SourceLockRef,
};
use bridge_return_guest::{wire, GuestInput, RelationWitness};
use bridge_return_host::{
    s1,
    s2::{self, RebuiltAccumulator},
};
use serde::{Deserialize, Serialize};

use super::burn::Burn;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Rejected {
    pub nullifier: String,
    pub reason: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Exclusion {
    AlreadySettled,
    Rejected { message: String },
    Invalid { message: String },
}

#[derive(Clone, Debug)]
pub struct Assembled {
    pub wire: Vec<u8>,
    pub public_values: PublicValues,
    pub leaves: Vec<ReturnLeaf>,
    pub lock_refs: Vec<SourceLockRef>,
    pub kept: Vec<String>,
    pub excluded: Vec<(String, Exclusion)>,
}

#[derive(Debug, thiserror::Error)]
pub enum AssembleError {
    #[error("every member was excluded")]
    AllExcluded(Vec<(String, Exclusion)>),
    #[error("accumulator transition failed: {0}")]
    Accumulator(String),
    #[error("assembled batch failed precheck: {0}")]
    Precheck(String),
}

struct Merge {
    template: Option<(BridgeConfig, PublicValues)>,
    taken: HashSet<u64>,
    leaves: Vec<ReturnLeaf>,
    lock_refs: Vec<SourceLockRef>,
    burns: Vec<bridge_return_guest::BridgeBurnWitness>,
    kept: Vec<String>,
    excluded: Vec<(String, Exclusion)>,
}

pub fn assemble(
    members: &[&Burn],
    acc: &RebuiltAccumulator,
    rejected: &[Rejected],
) -> Result<Assembled, AssembleError> {
    let mut merge = Merge {
        template: None,
        taken: HashSet::new(),
        leaves: Vec::new(),
        lock_refs: Vec::new(),
        burns: Vec::new(),
        kept: Vec::new(),
        excluded: Vec::new(),
    };
    for burn in members {
        match admit(burn, acc, rejected, &merge) {
            Ok(input) => merge.take(burn, input),
            Err(reason) => merge.excluded.push((burn.id.clone(), reason)),
        }
    }
    let Some((config, template)) = merge.template else {
        return Err(AssembleError::AllExcluded(merge.excluded));
    };
    merge.lock_refs.sort_by_key(|r| r.nonce);
    let nullifiers: Vec<[u8; 32]> = merge.leaves.iter().map(|l| l.nullifier).collect();
    let next =
        s2::next_batch(acc, &nullifiers).map_err(|e| AssembleError::Accumulator(e.to_string()))?;
    let public_values = PublicValues {
        domain_tag: template.domain_tag,
        config_hash: template.config_hash,
        trust_base_hash: template.trust_base_hash,
        spent_root_old: next.spent_root_old,
        spent_root_new: next.spent_root_new,
        return_root: return_root(&merge.leaves),
        lock_ref_root: lock_ref_root(&merge.lock_refs)
            .map_err(|e| AssembleError::Precheck(format!("{e:?}")))?,
        batch_size: merge.leaves.len() as u32,
        total_amount: sum_amounts(&merge.leaves),
    };
    let input = GuestInput {
        config,
        public_values,
        return_leaves: merge.leaves.clone(),
        sorted_lock_refs: merge.lock_refs.clone(),
        witness: RelationWitness {
            accumulator_witnesses: next.witnesses,
            bridge_burns: merge.burns,
        },
    };
    let wire = wire::encode_guest_input(&input);
    s1::precheck_wire(&wire).map_err(|e| AssembleError::Precheck(e.to_string()))?;
    Ok(Assembled {
        wire,
        public_values,
        leaves: merge.leaves,
        lock_refs: merge.lock_refs,
        kept: merge.kept,
        excluded: merge.excluded,
    })
}

impl Merge {
    fn take(&mut self, burn: &Burn, input: GuestInput) {
        self.template
            .get_or_insert((input.config, input.public_values));
        self.taken
            .extend(input.sorted_lock_refs.iter().map(|r| r.nonce));
        self.leaves.extend(input.return_leaves);
        self.lock_refs.extend(input.sorted_lock_refs);
        self.burns.extend(input.witness.bridge_burns);
        self.kept.push(burn.id.clone());
    }
}

fn admit(
    burn: &Burn,
    acc: &RebuiltAccumulator,
    rejected: &[Rejected],
    merge: &Merge,
) -> Result<GuestInput, Exclusion> {
    let input = wire::decode_guest_input(&burn.wire_input).map_err(|e| Exclusion::Invalid {
        message: format!("stored wire input does not decode: {e:?}"),
    })?;
    if let Some((config, template)) = &merge.template {
        if input.config != *config {
            return Err(invalid("bridge config differs from the batch"));
        }
        if input.public_values.trust_base_hash != template.trust_base_hash {
            return Err(invalid("trust base differs from the batch"));
        }
    }
    if let Some(r) = rejected
        .iter()
        .find(|r| r.nullifier.eq_ignore_ascii_case(burn.nullifier()))
    {
        return Err(Exclusion::Rejected {
            message: r.reason.clone(),
        });
    }
    if input
        .return_leaves
        .iter()
        .any(|l| acc.tree.non_membership_witness(&l.nullifier).is_none())
    {
        return Err(Exclusion::AlreadySettled);
    }
    if input
        .sorted_lock_refs
        .iter()
        .any(|r| merge.taken.contains(&r.nonce))
    {
        return Err(invalid(
            "lock nonce already used by another burn in the batch",
        ));
    }
    Ok(input)
}

fn invalid(message: &str) -> Exclusion {
    Exclusion::Invalid {
        message: message.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use bridge_return_core::u256_from_u64;
    use bridge_return_host::{
        fixture::{build_b1_direct_bridge_fixture, build_settlement_fixture_continued},
        s2::SettledBatch,
    };
    use bridge_return_sdk_ext::accumulator::{
        insert, ordered_insert_witnesses, verify_non_member, NullifierTree, EMPTY_TREE_ROOT,
    };

    use super::*;
    use crate::store::hex32;

    fn config() -> BridgeConfig {
        build_b1_direct_bridge_fixture().input.config
    }

    fn member(id: &str, nonce: u64, salt: u8) -> (Burn, GuestInput) {
        let input =
            build_settlement_fixture_continued(config(), [0xB2; 20], 1_000_000, nonce, salt, &[])
                .input;
        let wire = wire::encode_guest_input(&input);
        (Burn::from_input(id.to_string(), &input, wire), input)
    }

    fn nullifier(input: &GuestInput) -> [u8; 32] {
        input.return_leaves[0].nullifier
    }

    fn empty() -> RebuiltAccumulator {
        s2::rebuild(&[]).unwrap()
    }

    fn settled(nullifiers: &[[u8; 32]]) -> RebuiltAccumulator {
        let (_, spent_root_new) =
            ordered_insert_witnesses(&NullifierTree::new(), nullifiers).unwrap();
        s2::rebuild(&[SettledBatch {
            nullifiers: nullifiers.to_vec(),
            spent_root_old: EMPTY_TREE_ROOT,
            spent_root_new,
        }])
        .unwrap()
    }

    fn fold(wire: &[u8]) -> ([u8; 32], [u8; 32]) {
        let input = wire::decode_guest_input(wire).unwrap();
        let mut running = input.public_values.spent_root_old;
        for (leaf, witness) in input
            .return_leaves
            .iter()
            .zip(&input.witness.accumulator_witnesses)
        {
            assert!(verify_non_member(&running, &leaf.nullifier, witness));
            running = insert(&running, &leaf.nullifier, witness).unwrap();
        }
        (running, input.public_values.spent_root_new)
    }

    #[test]
    fn merges_two_inputs_into_a_b2_batch() {
        let (a, a_input) = member("a", 11, 0x61);
        let (b, b_input) = member("b", 12, 0x62);
        let assembled = assemble(&[&a, &b], &empty(), &[]).unwrap();

        assert_eq!(assembled.kept, vec!["a", "b"]);
        assert!(assembled.excluded.is_empty());
        assert_eq!(assembled.public_values.batch_size, 2);
        assert_eq!(
            assembled.public_values.total_amount,
            u256_from_u64(2_000_000)
        );
        assert_eq!(
            assembled.leaves,
            vec![a_input.return_leaves[0], b_input.return_leaves[0]]
        );
        assert_eq!(
            assembled.public_values.return_root,
            return_root(&assembled.leaves)
        );
        assert_eq!(
            assembled.lock_refs,
            vec![a_input.sorted_lock_refs[0], b_input.sorted_lock_refs[0]]
        );
        assert_eq!(
            assembled.public_values.lock_ref_root,
            lock_ref_root(&assembled.lock_refs).unwrap()
        );
        assert_eq!(assembled.public_values.spent_root_old, EMPTY_TREE_ROOT);

        let report = s1::precheck_wire(&assembled.wire).unwrap();
        assert_eq!(report.batch_size, 2);
        assert_eq!(report.public_values, assembled.public_values);
        let (folded, spent_root_new) = fold(&assembled.wire);
        assert_eq!(folded, spent_root_new);
    }

    #[test]
    fn rebases_onto_a_settled_accumulator() {
        let (_, prior) = member("p", 10, 0x60);
        let acc = settled(&[nullifier(&prior)]);
        let (a, _) = member("a", 11, 0x61);
        let (b, _) = member("b", 12, 0x62);
        let assembled = assemble(&[&a, &b], &acc, &[]).unwrap();

        assert_eq!(assembled.public_values.spent_root_old, acc.spent_root);
        let (folded, spent_root_new) = fold(&assembled.wire);
        assert_eq!(folded, spent_root_new);
        assert!(s1::precheck_wire(&assembled.wire).is_ok());
    }

    #[test]
    fn excludes_an_already_settled_member() {
        let (a, a_input) = member("a", 11, 0x61);
        let (b, _) = member("b", 12, 0x62);
        let acc = settled(&[nullifier(&a_input)]);
        let assembled = assemble(&[&a, &b], &acc, &[]).unwrap();

        assert_eq!(assembled.kept, vec!["b"]);
        assert_eq!(
            assembled.excluded,
            vec![("a".to_string(), Exclusion::AlreadySettled)]
        );
        assert_eq!(assembled.public_values.batch_size, 1);
        assert!(s1::precheck_wire(&assembled.wire).is_ok());
    }

    #[test]
    fn excludes_a_rejected_recipient() {
        let (a, a_input) = member("a", 11, 0x61);
        let (b, _) = member("b", 12, 0x62);
        let rejected = [Rejected {
            nullifier: hex32(&nullifier(&a_input))
                .to_uppercase()
                .replace("0X", "0x"),
            reason: "transfer would revert".to_string(),
        }];
        let assembled = assemble(&[&a, &b], &empty(), &rejected).unwrap();

        assert_eq!(assembled.kept, vec!["b"]);
        assert_eq!(
            assembled.excluded,
            vec![(
                "a".to_string(),
                Exclusion::Rejected {
                    message: "transfer would revert".to_string()
                }
            )]
        );
    }

    #[test]
    fn excludes_a_lock_nonce_conflict() {
        let (a, _) = member("a", 11, 0x61);
        let (c, _) = member("c", 11, 0x63);
        let assembled = assemble(&[&a, &c], &empty(), &[]).unwrap();

        assert_eq!(assembled.kept, vec!["a"]);
        assert_eq!(assembled.lock_refs.len(), 1);
        assert!(matches!(
            assembled.excluded.as_slice(),
            [(id, Exclusion::Invalid { .. })] if id == "c"
        ));
    }

    #[test]
    fn excludes_a_trust_base_mismatch() {
        let (a, _) = member("a", 11, 0x61);
        let (_, mut b_input) = member("b", 12, 0x62);
        b_input.public_values.trust_base_hash = [0xEE; 32];
        let b = Burn::from_input(
            "b".to_string(),
            &b_input,
            wire::encode_guest_input(&b_input),
        );
        let assembled = assemble(&[&a, &b], &empty(), &[]).unwrap();

        assert_eq!(assembled.kept, vec!["a"]);
        assert!(matches!(
            assembled.excluded.as_slice(),
            [(id, Exclusion::Invalid { .. })] if id == "b"
        ));
        assert!(s1::precheck_wire(&assembled.wire).is_ok());
    }

    #[test]
    fn excludes_undecodable_bytes() {
        let (a, _) = member("a", 11, 0x61);
        let mut broken = a.clone();
        broken.id = "broken".to_string();
        broken.wire_input.truncate(10);
        let assembled = assemble(&[&broken, &a], &empty(), &[]).unwrap();

        assert_eq!(assembled.kept, vec!["a"]);
        assert!(matches!(
            assembled.excluded.as_slice(),
            [(id, Exclusion::Invalid { .. })] if id == "broken"
        ));
    }

    #[test]
    fn single_member_equals_the_fixture_encoding() {
        let (a, a_input) = member("a", 11, 0x61);
        let assembled = assemble(&[&a], &empty(), &[]).unwrap();
        assert_eq!(assembled.wire, wire::encode_guest_input(&a_input));
    }

    #[test]
    fn all_excluded_is_an_error() {
        let (a, a_input) = member("a", 11, 0x61);
        let acc = settled(&[nullifier(&a_input)]);
        match assemble(&[&a], &acc, &[]) {
            Err(AssembleError::AllExcluded(excluded)) => {
                assert_eq!(excluded, vec![("a".to_string(), Exclusion::AlreadySettled)]);
            }
            other => panic!("expected AllExcluded, got {other:?}"),
        }
    }
}
