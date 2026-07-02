//! SP1 guest relation shell.
//!
//! M0 keeps this crate independent of the SP1 SDK so `cargo test` works with a
//! normal Rust toolchain. The SP1 entrypoint will deserialize `GuestInput`, call
//! `execute`, and commit `PublicValues` once E1-E3 supply the real witnesses.

#![no_std]
#![forbid(unsafe_code)]

extern crate alloc;

pub mod wire;

use alloc::vec::Vec;
use bridge_return_core::{
    burn_transition_id, config_hash, nullifier, public_values_abi, public_values_digest, sha256,
    validate_public_values, BridgeConfig as CoreBridgeConfig, BridgeCoreError, PublicValues,
    Result, ReturnLeaf, SourceLockRef, U256,
};
use bridge_return_sdk_ext::accumulator::{insert as accumulator_insert, NonMembershipWitness};
use bridge_return_sdk_ext::bridge::{
    bridge_lock_obligations_for_token_against_root, bridge_lock_obligations_for_token_certified,
    decode_bridged_payment_data, BridgeConfig as SdkBridgeConfig,
};
use bridge_return_sdk_ext::trust::canonical_hash;
use bridge_return_sdk_ext::verify::verify_anchor_certificate;
use unicity_token::api::bft::{RootTrustBase, UnicityCertificate};
use unicity_token::api::StateId;
use unicity_token::cbor::Decoder;
use unicity_token::payment::AssetId;
use unicity_token::predicate::builtin::BurnPredicate;
use unicity_token::transaction::Token;
use unicity_token::transaction::Transaction;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GuestInput {
    pub config: CoreBridgeConfig,
    pub public_values: PublicValues,
    pub return_leaves: Vec<ReturnLeaf>,
    pub sorted_lock_refs: Vec<SourceLockRef>,
    pub witness: RelationWitness,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RelationWitness {
    pub accumulator_witnesses: Vec<NonMembershipWitness>,
    pub bridge_burns: Vec<BridgeBurnWitness>,
}

/// How one burned token's transitions are proven against the trust base.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum BurnVerification {
    /// One shared anchor `UC*`; each transition is proven by anchored inclusion
    /// against its root. Burns sharing a byte-identical anchor amortize a single
    /// BFT-quorum check (the §11 one-quorum-check batch shape).
    Anchored(UnicityCertificate),
    /// Each transition carries its own `UnicityCertificate`, as served by a live
    /// aggregator (one quorum check per transition). Used until the aggregator
    /// serves historical inclusion proofs against a shared anchor (ZK_BACK3 §2.1).
    Certified,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BridgeBurnWitness {
    pub token: Token,
    pub trust_base: RootTrustBase,
    pub verification: BurnVerification,
    pub lock_justification_tag: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GuestOutput {
    pub public_values: PublicValues,
    pub public_values_abi: Vec<u8>,
    pub public_values_digest: [u8; 32],
}

pub fn execute(input: &GuestInput) -> Result<PublicValues> {
    validate_public_values(
        &input.config,
        &input.public_values,
        &input.return_leaves,
        &input.sorted_lock_refs,
    )?;
    validate_accumulator_transition(
        input.public_values.spent_root_old,
        input.public_values.spent_root_new,
        &input.return_leaves,
        &input.witness.accumulator_witnesses,
    )?;
    validate_bridge_burns(
        &input.config,
        input.public_values.trust_base_hash,
        &input.return_leaves,
        &input.sorted_lock_refs,
        &input.witness.bridge_burns,
    )?;
    Ok(input.public_values)
}

pub fn execute_public_output(input: &GuestInput) -> Result<GuestOutput> {
    let public_values = execute(input)?;
    Ok(public_output(public_values))
}

pub fn public_output(public_values: PublicValues) -> GuestOutput {
    GuestOutput {
        public_values,
        public_values_abi: public_values_abi(&public_values),
        public_values_digest: public_values_digest(&public_values),
    }
}

pub fn execute_wire(input: &[u8]) -> Result<GuestOutput> {
    let input = wire::decode_guest_input(input)?;
    execute_public_output(&input)
}

fn validate_bridge_burns(
    config: &CoreBridgeConfig,
    trust_base_hash: [u8; 32],
    leaves: &[ReturnLeaf],
    sorted_lock_refs: &[SourceLockRef],
    burns: &[BridgeBurnWitness],
) -> Result<()> {
    if burns.is_empty() {
        return Ok(());
    }
    if burns.len() != leaves.len() {
        return Err(BridgeCoreError::WrongBatchSize);
    }
    let sdk_config = sdk_bridge_config(config);
    let cfg_hash = config_hash(config);
    let mut obligations = Vec::with_capacity(burns.len());
    // One BFT-quorum check per *distinct* anchor `UC*` (ZK_BACK3 §11): burns that
    // share a byte-identical anchor reuse the verified root, so a batch under one
    // shared anchor pays a single quorum verification instead of one per burn.
    let mut anchor_roots: Vec<(&UnicityCertificate, [u8; 32])> = Vec::new();
    for (burn, leaf) in burns.iter().zip(leaves) {
        if canonical_hash(&burn.trust_base) != trust_base_hash {
            return Err(BridgeCoreError::WrongTrustBaseHash);
        }
        let burn_obligations = match &burn.verification {
            BurnVerification::Anchored(anchor) => {
                let anchor_root = match anchor_roots.iter().find(|(a, _)| *a == anchor) {
                    Some((_, root)) => *root,
                    None => {
                        let root = verify_anchor_certificate(&burn.trust_base, anchor)
                            .map_err(|_| BridgeCoreError::TokenVerificationFailed)?;
                        anchor_roots.push((anchor, root));
                        root
                    }
                };
                bridge_lock_obligations_for_token_against_root(
                    &burn.token,
                    &burn.trust_base,
                    &anchor_root,
                    burn.lock_justification_tag,
                    &sdk_config,
                    decode_bridged_payment_data,
                )
            }
            BurnVerification::Certified => bridge_lock_obligations_for_token_certified(
                &burn.token,
                &burn.trust_base,
                burn.lock_justification_tag,
                &sdk_config,
                decode_bridged_payment_data,
            ),
        }
        .map_err(|_| BridgeCoreError::TokenVerificationFailed)?;
        validate_terminal_burn(config, &cfg_hash, leaf, &burn.token)?;
        validate_current_value(config, leaf, &burn.token)?;
        obligations.extend(
            burn_obligations
                .into_iter()
                .map(|obligation| SourceLockRef {
                    nonce: obligation.nonce,
                    digest: obligation.digest,
                }),
        );
    }
    obligations.sort_by_key(|r| r.nonce);
    if obligations.as_slice() != sorted_lock_refs {
        return Err(BridgeCoreError::WrongLockObligation);
    }
    Ok(())
}

fn validate_current_value(
    config: &CoreBridgeConfig,
    leaf: &ReturnLeaf,
    token: &Token,
) -> Result<()> {
    let payment = decode_bridged_payment_data(
        token
            .genesis()
            .transaction()
            .data()
            .ok_or(BridgeCoreError::WrongBurnAmount)?,
    )
    .map_err(|_| BridgeCoreError::WrongBurnAmount)?;
    let asset = payment
        .get(&AssetId::new(config.coin_id.to_vec()))
        .ok_or(BridgeCoreError::WrongBurnAmount)?;
    let amount = u256_from_amount_bytes(&unicity_token::rsmst::encode_amount(asset.value()))
        .ok_or(BridgeCoreError::WrongBurnAmount)?;
    if amount != leaf.amount {
        return Err(BridgeCoreError::WrongBurnAmount);
    }
    Ok(())
}

fn validate_terminal_burn(
    config: &CoreBridgeConfig,
    cfg_hash: &[u8; 32],
    leaf: &ReturnLeaf,
    token: &Token,
) -> Result<()> {
    let burn = token
        .transactions()
        .last()
        .ok_or(BridgeCoreError::BurnTransferMissing)?;
    let reason_bytes = burn
        .transaction()
        .data()
        .ok_or(BridgeCoreError::BurnReasonMissing)?;
    let expected_burn = BurnPredicate::new(sha256(reason_bytes).to_vec()).to_encoded();
    if burn.recipient() != &expected_burn {
        return Err(BridgeCoreError::WrongBurnPredicate);
    }

    let reason = decode_bridge_back_reason(config.reason_tag, reason_bytes)?;
    if reason.version != 1
        || reason.source_chain_id != config.source_chain_id
        || reason.vault != config.vault
        || reason.asset != config.asset
        || reason.token_type != config.token_type
        || reason.coin_id != config.coin_id
        || reason.recipient != leaf.recipient
        || reason.amount != leaf.amount
        || reason.fee_recipient != leaf.fee_recipient
        || reason.fee_amount != leaf.fee_amount
        || reason.deadline != leaf.deadline
        || greater_than(&reason.fee_amount, &reason.amount)
    {
        return Err(BridgeCoreError::BurnReasonMismatch);
    }

    let state_id = StateId::derive(
        burn.transaction().lock_script(),
        burn.transaction().source_state_hash(),
    );
    let tx_hash = burn.transaction().calculate_transaction_hash();
    let tx_hash_bytes: [u8; 32] = tx_hash
        .data()
        .try_into()
        .map_err(|_| BridgeCoreError::WrongNullifier)?;
    let burn_id = burn_transition_id(state_id.bytes(), &tx_hash_bytes);
    let expected_nullifier = nullifier(cfg_hash, &burn_id);
    if leaf.nullifier != expected_nullifier {
        return Err(BridgeCoreError::WrongNullifier);
    }
    Ok(())
}

/// The 11-field `BridgeBackReason` decoded from a burn's auxiliary data (00 §4).
/// Exposed so the host/service can derive a `ReturnLeaf` from a burned token's
/// `reasonBytes` (the wallet's witness envelope carries only the bytes).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DecodedBridgeBackReason {
    pub version: u64,
    pub source_chain_id: u64,
    pub vault: [u8; 20],
    pub asset: [u8; 20],
    pub token_type: [u8; 32],
    pub coin_id: [u8; 32],
    pub recipient: [u8; 20],
    pub amount: U256,
    pub fee_recipient: [u8; 20],
    pub fee_amount: U256,
    pub deadline: u64,
}

/// Decode canonical `reasonBytes` (tag `expected_tag`, 11-field array) — the
/// inverse of [`bridge_return_core::reason_cbor`]. Strict: rejects a wrong tag,
/// wrong arity, or trailing bytes.
pub fn decode_bridge_back_reason(expected_tag: u64, bytes: &[u8]) -> Result<DecodedBridgeBackReason> {
    let decoder = Decoder::new(bytes);
    decoder
        .finish()
        .map_err(|_| BridgeCoreError::BurnReasonMalformed)?;
    let inner = decoder
        .expect_tag(expected_tag)
        .map_err(|_| BridgeCoreError::BurnReasonMalformed)?;
    let items = inner
        .array(Some(11))
        .map_err(|_| BridgeCoreError::BurnReasonMalformed)?;
    Ok(DecodedBridgeBackReason {
        version: uint(items[0])?,
        source_chain_id: uint(items[1])?,
        vault: bytes_n(items[2])?,
        asset: bytes_n(items[3])?,
        token_type: bytes_n(items[4])?,
        coin_id: bytes_n(items[5])?,
        recipient: bytes_n(items[6])?,
        amount: u256_bstr(items[7])?,
        fee_recipient: bytes_n(items[8])?,
        fee_amount: u256_bstr(items[9])?,
        deadline: uint(items[10])?,
    })
}

fn uint(decoder: Decoder<'_>) -> Result<u64> {
    decoder
        .uint()
        .map_err(|_| BridgeCoreError::BurnReasonMalformed)
}

fn bytes_n<const N: usize>(decoder: Decoder<'_>) -> Result<[u8; N]> {
    let bytes = decoder
        .bytes_value()
        .map_err(|_| BridgeCoreError::BurnReasonMalformed)?;
    bytes
        .try_into()
        .map_err(|_| BridgeCoreError::BurnReasonMalformed)
}

fn u256_bstr(decoder: Decoder<'_>) -> Result<U256> {
    let bytes = decoder
        .bytes_value()
        .map_err(|_| BridgeCoreError::BurnReasonMalformed)?;
    if bytes.len() > 32 || bytes.first() == Some(&0) {
        return Err(BridgeCoreError::BurnReasonMalformed);
    }
    let mut out = [0u8; 32];
    out[32 - bytes.len()..].copy_from_slice(bytes);
    Ok(out)
}

fn greater_than(left: &U256, right: &U256) -> bool {
    left.as_slice() > right.as_slice()
}

fn u256_from_amount_bytes(bytes: &[u8]) -> Option<U256> {
    if bytes.len() > 32 {
        return None;
    }
    let mut out = [0u8; 32];
    out[32 - bytes.len()..].copy_from_slice(&bytes);
    Some(out)
}

fn sdk_bridge_config(config: &CoreBridgeConfig) -> SdkBridgeConfig {
    SdkBridgeConfig {
        source_chain_id: config.source_chain_id,
        vault: config.vault,
        asset: config.asset,
        token_type: config.token_type,
        coin_id: config.coin_id,
    }
}

fn validate_accumulator_transition(
    spent_root_old: [u8; 32],
    spent_root_new: [u8; 32],
    leaves: &[ReturnLeaf],
    witnesses: &[NonMembershipWitness],
) -> Result<()> {
    if leaves.len() != witnesses.len() {
        return Err(BridgeCoreError::WrongBatchSize);
    }
    let mut running = spent_root_old;
    for (leaf, witness) in leaves.iter().zip(witnesses) {
        running = accumulator_insert(&running, &leaf.nullifier, witness)
            .ok_or(BridgeCoreError::WrongAccumulatorWitness)?;
    }
    if running != spent_root_new {
        return Err(BridgeCoreError::WrongSpentRootNew);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use bridge_return_core::{
        config_hash, domain_tag, lock_ref_root, reason_cbor, return_root, sum_amounts,
        BridgeBackReason, BridgeConfig, PublicValues, ReturnLeaf,
    };
    use bridge_return_sdk_ext::accumulator::{ordered_insert_witnesses, NullifierTree};

    fn cfg() -> BridgeConfig {
        BridgeConfig {
            source_chain_id: 1,
            vault: [0x11; 20],
            asset: [0x22; 20],
            token_type: [0x33; 32],
            coin_id: [0x44; 32],
            reason_tag: 39048,
            lock_domain: [0x55; 32],
            nullifier_domain: [0x66; 32],
        }
    }

    fn leaf(nullifier: [u8; 32], amount: u8) -> ReturnLeaf {
        let mut amount_word = [0u8; 32];
        amount_word[31] = amount;
        ReturnLeaf {
            nullifier,
            recipient: [0x77; 20],
            amount: amount_word,
            fee_recipient: [0u8; 20],
            fee_amount: [0u8; 32],
            deadline: 0,
        }
    }

    fn input(leaves: Vec<ReturnLeaf>) -> GuestInput {
        let cfg = cfg();
        let nullifiers = leaves.iter().map(|leaf| leaf.nullifier).collect::<Vec<_>>();
        let tree = NullifierTree::new();
        let (witnesses, spent_root_new) = ordered_insert_witnesses(&tree, &nullifiers).unwrap();
        let lock_refs = Vec::new();
        let public_values = PublicValues {
            domain_tag: domain_tag(),
            config_hash: config_hash(&cfg),
            trust_base_hash: [0x88; 32],
            spent_root_old: tree.root(),
            spent_root_new,
            return_root: return_root(&leaves),
            lock_ref_root: lock_ref_root(&lock_refs).unwrap(),
            batch_size: leaves.len() as u32,
            total_amount: sum_amounts(&leaves),
        };
        GuestInput {
            config: cfg,
            public_values,
            return_leaves: leaves,
            sorted_lock_refs: lock_refs,
            witness: RelationWitness {
                accumulator_witnesses: witnesses,
                bridge_burns: Vec::new(),
            },
        }
    }

    #[test]
    fn execute_threads_accumulator_witnesses() {
        let input = input(alloc::vec![leaf([0x01; 32], 3), leaf([0x02; 32], 4)]);
        assert!(execute(&input).is_ok());
    }

    #[test]
    fn execute_rejects_stale_accumulator_witness() {
        let mut input = input(alloc::vec![leaf([0x01; 32], 3), leaf([0x02; 32], 4)]);
        input.witness.accumulator_witnesses.swap(0, 1);
        assert!(execute(&input).is_err());
    }

    #[test]
    fn bridge_back_reason_decodes_core_encoding() {
        let cfg = cfg();
        let reason = BridgeBackReason {
            version: 1,
            recipient: [0x91; 20],
            amount: bridge_return_core::u256_from_u64(100),
            fee_recipient: [0x92; 20],
            fee_amount: bridge_return_core::u256_from_u64(3),
            deadline: 1234,
        };
        let bytes = reason_cbor(&cfg, &reason);
        let decoded = decode_bridge_back_reason(cfg.reason_tag, &bytes).unwrap();
        assert_eq!(decoded.version, 1);
        assert_eq!(decoded.source_chain_id, cfg.source_chain_id);
        assert_eq!(decoded.vault, cfg.vault);
        assert_eq!(decoded.asset, cfg.asset);
        assert_eq!(decoded.token_type, cfg.token_type);
        assert_eq!(decoded.coin_id, cfg.coin_id);
        assert_eq!(decoded.recipient, reason.recipient);
        assert_eq!(decoded.amount, reason.amount);
        assert_eq!(decoded.fee_recipient, reason.fee_recipient);
        assert_eq!(decoded.fee_amount, reason.fee_amount);
        assert_eq!(decoded.deadline, reason.deadline);
    }

    #[test]
    fn bridge_back_reason_rejects_wrong_tag() {
        let cfg = cfg();
        let reason = BridgeBackReason {
            version: 1,
            recipient: [0x91; 20],
            amount: bridge_return_core::u256_from_u64(100),
            fee_recipient: [0x92; 20],
            fee_amount: bridge_return_core::u256_from_u64(3),
            deadline: 1234,
        };
        let bytes = reason_cbor(&cfg, &reason);
        assert_eq!(
            decode_bridge_back_reason(cfg.reason_tag + 1, &bytes),
            Err(BridgeCoreError::BurnReasonMalformed)
        );
    }
}
