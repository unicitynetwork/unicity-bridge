//! Byte-level bridge return contract shared by the prover host and guest.
//!
//! This crate implements the prover-side subset of
//! `docs/bridge/dev-plan/00-interop-contract.md` for
//! `BRIDGE_PROTO_VERSION = 1`.

#![cfg_attr(not(feature = "std"), no_std)]
#![forbid(unsafe_code)]

extern crate alloc;

mod abi;
mod cbor;
mod hash;

use alloc::vec::Vec;

pub const BRIDGE_PROTO_VERSION: u32 = 1;

pub const DOMAIN_CONFIG: &str = "unicity-bridge-return-config:v1";
pub const DOMAIN_LOCK: &str = "unicity-bridge-lock:v1";
pub const DOMAIN_RETURN: &str = "unicity-bridge-return:v1";
pub const DOMAIN_BURN_TRANSITION: &str = "unicity-burn-transition:v1";
pub const DOMAIN_NULLIFIER: &str = "unicity-bridge-return-nullifier:v1";

pub type Address = [u8; 20];
pub type Bytes32 = [u8; 32];
pub type U256 = [u8; 32];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BridgeConfig {
    pub source_chain_id: u64,
    pub vault: Address,
    pub asset: Address,
    pub token_type: Bytes32,
    pub coin_id: Bytes32,
    pub reason_tag: u64,
    pub lock_domain: Bytes32,
    pub nullifier_domain: Bytes32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LockRecord {
    pub nonce: u64,
    pub amount: U256,
    pub unicity_token_id: Bytes32,
    pub recipient_commitment: Bytes32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BridgeBackReason {
    pub version: u64,
    pub recipient: Address,
    pub amount: U256,
    pub fee_recipient: Address,
    pub fee_amount: U256,
    pub deadline: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ReturnLeaf {
    pub nullifier: Bytes32,
    pub recipient: Address,
    pub amount: U256,
    pub fee_recipient: Address,
    pub fee_amount: U256,
    pub deadline: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SourceLockRef {
    pub nonce: u64,
    pub digest: Bytes32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PublicValues {
    pub domain_tag: Bytes32,
    pub config_hash: Bytes32,
    pub trust_base_hash: Bytes32,
    pub spent_root_old: Bytes32,
    pub spent_root_new: Bytes32,
    pub return_root: Bytes32,
    pub lock_ref_root: Bytes32,
    pub batch_size: u32,
    pub total_amount: U256,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BridgeCoreError {
    DuplicateLockRefNonce,
    LockRefsNotSorted,
    WrongDomainTag,
    WrongConfigHash,
    WrongTrustBaseHash,
    WrongReturnRoot,
    WrongLockRefRoot,
    WrongBatchSize,
    WrongTotalAmount,
    WrongAccumulatorWitness,
    WrongSpentRootNew,
    TokenVerificationFailed,
    WrongLockObligation,
    BurnTransferMissing,
    BurnReasonMissing,
    BurnReasonMalformed,
    BurnReasonMismatch,
    WrongBurnPredicate,
    WrongNullifier,
    WrongBurnAmount,
}

pub type Result<T> = core::result::Result<T, BridgeCoreError>;

pub fn sha256(input: &[u8]) -> Bytes32 {
    hash::sha256(input)
}

pub fn keccak256(input: &[u8]) -> Bytes32 {
    hash::keccak256(input)
}

pub fn token_type(chain_id_str: &str, asset_evm_hex: &str) -> Bytes32 {
    let mut data = Vec::new();
    data.extend_from_slice(b"unicity-bridge:tron:");
    data.extend_from_slice(chain_id_str.as_bytes());
    data.push(b':');
    data.extend_from_slice(asset_evm_hex.as_bytes());
    sha256(&data)
}

pub fn coin_id(chain_id_str: &str, asset_evm_hex: &str) -> Bytes32 {
    let mut data = Vec::new();
    data.extend_from_slice(b"unicity-bridge-coin:tron:");
    data.extend_from_slice(chain_id_str.as_bytes());
    data.push(b':');
    data.extend_from_slice(asset_evm_hex.as_bytes());
    sha256(&data)
}

pub fn recipient_commitment(recipient_cbor: &[u8]) -> Bytes32 {
    sha256(recipient_cbor)
}

pub fn config_hash(config: &BridgeConfig) -> Bytes32 {
    keccak256(&abi::encode(&[
        abi::Val::Str(DOMAIN_CONFIG),
        abi::Val::U64(config.source_chain_id),
        abi::Val::Addr(config.vault),
        abi::Val::Addr(config.asset),
        abi::Val::B32(config.token_type),
        abi::Val::B32(config.coin_id),
        abi::Val::U64(config.reason_tag),
        abi::Val::B32(config.lock_domain),
        abi::Val::B32(config.nullifier_domain),
    ]))
}

pub fn lock_digest(config: &BridgeConfig, record: &LockRecord) -> Bytes32 {
    keccak256(&abi::encode(&[
        abi::Val::Str(DOMAIN_LOCK),
        abi::Val::U64(config.source_chain_id),
        abi::Val::Addr(config.vault),
        abi::Val::U256(u256_from_u64(record.nonce)),
        abi::Val::Addr(config.asset),
        abi::Val::B32(config.token_type),
        abi::Val::B32(config.coin_id),
        abi::Val::U256(record.amount),
        abi::Val::B32(record.unicity_token_id),
        abi::Val::B32(record.recipient_commitment),
    ]))
}

pub fn reason_cbor(config: &BridgeConfig, reason: &BridgeBackReason) -> Vec<u8> {
    let mut out = cbor::tag(config.reason_tag);
    out.extend(cbor::array_header(11));
    out.extend(cbor::uint(reason.version));
    out.extend(cbor::uint(config.source_chain_id));
    out.extend(cbor::bytes(&config.vault));
    out.extend(cbor::bytes(&config.asset));
    out.extend(cbor::bytes(&config.token_type));
    out.extend(cbor::bytes(&config.coin_id));
    out.extend(cbor::bytes(&reason.recipient));
    out.extend(cbor::bytes(&minimal_be(&reason.amount)));
    out.extend(cbor::bytes(&reason.fee_recipient));
    out.extend(cbor::bytes(&minimal_be(&reason.fee_amount)));
    out.extend(cbor::uint(reason.deadline));
    out
}

pub fn reason_hash(reason_bytes: &[u8]) -> Bytes32 {
    sha256(reason_bytes)
}

pub fn burn_transition_id(state_id: &Bytes32, tx_hash: &Bytes32) -> Bytes32 {
    cbor::h_array(&[
        cbor::text(DOMAIN_BURN_TRANSITION),
        cbor::bytes(state_id),
        cbor::bytes(tx_hash),
    ])
}

pub fn nullifier(config_hash: &Bytes32, burn_transition_id: &Bytes32) -> Bytes32 {
    cbor::h_array(&[
        cbor::text(DOMAIN_NULLIFIER),
        cbor::bytes(config_hash),
        cbor::bytes(burn_transition_id),
    ])
}

pub fn domain_tag() -> Bytes32 {
    keccak256(DOMAIN_RETURN.as_bytes())
}

pub fn return_root(leaves: &[ReturnLeaf]) -> Bytes32 {
    let mut buf = Vec::new();
    for leaf in leaves {
        buf.extend(abi::pack_words(&[
            abi::Val::B32(leaf.nullifier),
            abi::Val::Addr(leaf.recipient),
            abi::Val::U256(leaf.amount),
            abi::Val::Addr(leaf.fee_recipient),
            abi::Val::U256(leaf.fee_amount),
            abi::Val::U64(leaf.deadline),
        ]));
    }
    keccak256(&buf)
}

pub fn lock_ref_root(refs: &[SourceLockRef]) -> Result<Bytes32> {
    let mut prev = None;
    let mut buf = Vec::new();
    for lock_ref in refs {
        if let Some(prev_nonce) = prev {
            if lock_ref.nonce == prev_nonce {
                return Err(BridgeCoreError::DuplicateLockRefNonce);
            }
            if lock_ref.nonce < prev_nonce {
                return Err(BridgeCoreError::LockRefsNotSorted);
            }
        }
        prev = Some(lock_ref.nonce);
        buf.extend(abi::pack_words(&[
            abi::Val::U256(u256_from_u64(lock_ref.nonce)),
            abi::Val::B32(lock_ref.digest),
        ]));
    }
    Ok(keccak256(&buf))
}

pub fn sorted_lock_ref_root(refs: &[SourceLockRef]) -> Result<Bytes32> {
    let mut sorted = refs.to_vec();
    sorted.sort_by_key(|lock_ref| lock_ref.nonce);
    lock_ref_root(&sorted)
}

pub fn public_values_abi(public_values: &PublicValues) -> Vec<u8> {
    abi::encode(&[
        abi::Val::B32(public_values.domain_tag),
        abi::Val::B32(public_values.config_hash),
        abi::Val::B32(public_values.trust_base_hash),
        abi::Val::B32(public_values.spent_root_old),
        abi::Val::B32(public_values.spent_root_new),
        abi::Val::B32(public_values.return_root),
        abi::Val::B32(public_values.lock_ref_root),
        abi::Val::U32(public_values.batch_size),
        abi::Val::U256(public_values.total_amount),
    ])
}

pub fn public_values_digest(public_values: &PublicValues) -> Bytes32 {
    keccak256(&public_values_abi(public_values))
}

pub fn validate_public_values(
    config: &BridgeConfig,
    public_values: &PublicValues,
    leaves: &[ReturnLeaf],
    sorted_lock_refs: &[SourceLockRef],
) -> Result<()> {
    if public_values.domain_tag != domain_tag() {
        return Err(BridgeCoreError::WrongDomainTag);
    }
    if public_values.config_hash != config_hash(config) {
        return Err(BridgeCoreError::WrongConfigHash);
    }
    if public_values.return_root != return_root(leaves) {
        return Err(BridgeCoreError::WrongReturnRoot);
    }
    if public_values.lock_ref_root != lock_ref_root(sorted_lock_refs)? {
        return Err(BridgeCoreError::WrongLockRefRoot);
    }
    if public_values.batch_size != leaves.len() as u32 {
        return Err(BridgeCoreError::WrongBatchSize);
    }
    let total = sum_amounts(leaves);
    if public_values.total_amount != total {
        return Err(BridgeCoreError::WrongTotalAmount);
    }
    Ok(())
}

pub fn sum_amounts(leaves: &[ReturnLeaf]) -> U256 {
    let mut acc = [0u8; 32];
    for leaf in leaves {
        acc = add_u256(acc, leaf.amount);
    }
    acc
}

pub fn u256_from_u64(n: u64) -> U256 {
    let mut out = [0u8; 32];
    out[24..].copy_from_slice(&n.to_be_bytes());
    out
}

fn add_u256(mut left: U256, right: U256) -> U256 {
    let mut carry = 0u16;
    for i in (0..32).rev() {
        let sum = left[i] as u16 + right[i] as u16 + carry;
        left[i] = sum as u8;
        carry = sum >> 8;
    }
    left
}

fn minimal_be(value: &U256) -> Vec<u8> {
    let first = value.iter().position(|&byte| byte != 0).unwrap_or(32);
    value[first..].to_vec()
}
