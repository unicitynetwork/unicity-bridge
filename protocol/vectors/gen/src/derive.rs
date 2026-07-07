//! The bridge derivations from `protocol/interop.md`.
//! Each function is the single, auditable definition of one cross-stack value;
//! the contracts (Solidity), the TS SDK, and the prover (Rust) must reproduce
//! these byte-for-byte.

use crate::abi::{self, Val};
use crate::cbor;
use crate::hash::{keccak256, sha256};

// Domain separators (00 §2, §3, §5, §7).
pub const DOMAIN_CONFIG: &str = "unicity-bridge-return-config:v1";
pub const DOMAIN_LOCK: &str = "unicity-bridge-lock:v1";
pub const DOMAIN_RETURN: &str = "unicity-bridge-return:v1";
pub const DOMAIN_BURN_TRANSITION: &str = "unicity-burn-transition:v1";
pub const DOMAIN_NULLIFIER: &str = "unicity-bridge-return-nullifier:v1";

/// Deployment configuration (00 §2).
pub struct Config {
    pub source_chain_id: u64,
    pub vault: [u8; 20],
    pub asset: [u8; 20],
    pub token_type: [u8; 32],
    pub coin_id: [u8; 32],
    pub reason_tag: u64,
    pub lock_domain: [u8; 32],
    pub nullifier_domain: [u8; 32],
}

/// `tokenType = SHA256("unicity-bridge:tron:<chainId>:<assetEvmHex>")`
/// (packages/bridge-plugin-tron-usdt/src/identifiers.ts, frozen in interop §2).
pub fn token_type(chain_id_str: &str, asset_evm_hex: &str) -> [u8; 32] {
    sha256(format!("unicity-bridge:tron:{chain_id_str}:{asset_evm_hex}").as_bytes())
}

/// `coinId = SHA256("unicity-bridge-coin:tron:<chainId>:<assetEvmHex>")`.
pub fn coin_id(chain_id_str: &str, asset_evm_hex: &str) -> [u8; 32] {
    sha256(format!("unicity-bridge-coin:tron:{chain_id_str}:{asset_evm_hex}").as_bytes())
}

/// `recipientCommitment = SHA256(recipient predicate CBOR)` (00 §3).
pub fn recipient_commitment(recipient_cbor: &[u8]) -> [u8; 32] {
    sha256(recipient_cbor)
}

/// `configHash = keccak256(abi.encode(...))` (00 §2). Keccak/ABI (00 §1).
pub fn config_hash(c: &Config) -> [u8; 32] {
    keccak256(&abi::encode(&[
        Val::Str(DOMAIN_CONFIG),
        Val::U64(c.source_chain_id),
        Val::Addr(c.vault),
        Val::Addr(c.asset),
        Val::B32(c.token_type),
        Val::B32(c.coin_id),
        Val::U64(c.reason_tag),
        Val::B32(c.lock_domain),
        Val::B32(c.nullifier_domain),
    ]))
}

/// Bridge-in lock record (00 §3). `asset/tokenType/coinId` come from `Config`.
pub struct LockRecord {
    pub nonce: u64,
    pub amount: [u8; 32],
    pub unicity_token_id: [u8; 32],
    pub recipient_commitment: [u8; 32],
}

/// `lockDigest = keccak256(abi.encode(...))` (00 §3). Keccak/ABI (00 §1).
pub fn lock_digest(c: &Config, r: &LockRecord) -> [u8; 32] {
    keccak256(&abi::encode(&[
        Val::Str(DOMAIN_LOCK),
        Val::U64(c.source_chain_id),
        Val::Addr(c.vault),
        Val::U256(u256(r.nonce)),
        Val::Addr(c.asset),
        Val::B32(c.token_type),
        Val::B32(c.coin_id),
        Val::U256(r.amount),
        Val::B32(r.unicity_token_id),
        Val::B32(r.recipient_commitment),
    ]))
}

/// Per-return parameters of a `BridgeBackReason` (00 §4).
pub struct ReasonParams {
    pub version: u64,
    pub recipient: [u8; 20],
    pub amount: [u8; 32],
    pub fee_recipient: [u8; 20],
    pub fee_amount: [u8; 32],
    pub deadline: u64,
}

/// Deterministic CBOR of the 11-field `BridgeBackReason` array under `reasonTag`
/// (00 §4). PROVISIONAL field CBOR types — confirm against the SDK's
/// deterministic-CBOR conventions at M0: version/chainId/deadline as CBOR uints;
/// addresses and ids as byte strings; amounts as minimal big-endian byte strings.
pub fn reason_cbor(c: &Config, r: &ReasonParams) -> Vec<u8> {
    let mut out = cbor::tag(c.reason_tag);
    out.extend(cbor::array_header(11));
    out.extend(cbor::uint(r.version));
    out.extend(cbor::uint(c.source_chain_id));
    out.extend(cbor::bytes(&c.vault));
    out.extend(cbor::bytes(&c.asset));
    out.extend(cbor::bytes(&c.token_type));
    out.extend(cbor::bytes(&c.coin_id));
    out.extend(cbor::bytes(&r.recipient));
    out.extend(cbor::bytes(&minimal_be(&r.amount)));
    out.extend(cbor::bytes(&r.fee_recipient));
    out.extend(cbor::bytes(&minimal_be(&r.fee_amount)));
    out.extend(cbor::uint(r.deadline));
    out
}

/// `burnTransitionId = H("unicity-burn-transition:v1", stateId, txHash)` (00 §5).
pub fn burn_transition_id(state_id: &[u8; 32], tx_hash: &[u8; 32]) -> [u8; 32] {
    cbor::h_array(&[
        cbor::text(DOMAIN_BURN_TRANSITION),
        cbor::bytes(state_id),
        cbor::bytes(tx_hash),
    ])
}

/// `nullifier = H("unicity-bridge-return-nullifier:v1", configHash, burnTransitionId)`
/// (00 §5). Note `configHash` (keccak) is fed into this SHA-256 hash.
pub fn nullifier(config_hash: &[u8; 32], burn_transition_id: &[u8; 32]) -> [u8; 32] {
    cbor::h_array(&[
        cbor::text(DOMAIN_NULLIFIER),
        cbor::bytes(config_hash),
        cbor::bytes(burn_transition_id),
    ])
}

/// One settlement leaf (00 §7).
pub struct ReturnLeaf {
    pub nullifier: [u8; 32],
    pub recipient: [u8; 20],
    pub amount: [u8; 32],
    pub fee_recipient: [u8; 20],
    pub fee_amount: [u8; 32],
    pub deadline: u64,
}

/// `returnRoot = keccak256( concat of fixed-width ReturnLeaf words )` (00 §7).
pub fn return_root(leaves: &[ReturnLeaf]) -> [u8; 32] {
    let mut buf = Vec::new();
    for l in leaves {
        buf.extend(abi::pack_words(&[
            Val::B32(l.nullifier),
            Val::Addr(l.recipient),
            Val::U256(l.amount),
            Val::Addr(l.fee_recipient),
            Val::U256(l.fee_amount),
            Val::U64(l.deadline),
        ]));
    }
    keccak256(&buf)
}

/// One source lock reference (00 §7).
pub struct LockRef {
    pub nonce: u64,
    pub digest: [u8; 32],
}

/// `lockRefRoot = keccak256( concat of fixed-width LockRef words )`, sorted by
/// nonce with duplicates rejected (00 §7).
pub fn lock_ref_root(refs: &[LockRef]) -> [u8; 32] {
    let mut sorted: Vec<&LockRef> = refs.iter().collect();
    sorted.sort_by_key(|r| r.nonce);
    for pair in sorted.windows(2) {
        assert_ne!(pair[0].nonce, pair[1].nonce, "duplicate lock-ref nonce");
    }
    let mut buf = Vec::new();
    for r in sorted {
        buf.extend(abi::pack_words(&[
            Val::U256(u256(r.nonce)),
            Val::B32(r.digest),
        ]));
    }
    keccak256(&buf)
}

/// The public statement the circuit commits and the vault decodes (00 §7).
pub struct PublicValues {
    pub domain_tag: [u8; 32],
    pub config_hash: [u8; 32],
    pub trust_base_hash: [u8; 32],
    pub spent_root_old: [u8; 32],
    pub spent_root_new: [u8; 32],
    pub return_root: [u8; 32],
    pub lock_ref_root: [u8; 32],
    pub batch_size: u32,
    pub total_amount: [u8; 32],
}

/// `domainTag = keccak256("unicity-bridge-return:v1")` (00 §7).
pub fn domain_tag() -> [u8; 32] {
    keccak256(DOMAIN_RETURN.as_bytes())
}

/// `abi.encode(PublicValues)` — all-static layout the vault `abi.decode`s (00 §7).
pub fn public_values_abi(p: &PublicValues) -> Vec<u8> {
    abi::encode(&[
        Val::B32(p.domain_tag),
        Val::B32(p.config_hash),
        Val::B32(p.trust_base_hash),
        Val::B32(p.spent_root_old),
        Val::B32(p.spent_root_new),
        Val::B32(p.return_root),
        Val::B32(p.lock_ref_root),
        Val::U32(p.batch_size),
        Val::U256(p.total_amount),
    ])
}

/// 32-byte big-endian representation of a small unsigned integer.
pub fn u256(n: u64) -> [u8; 32] {
    let mut w = [0u8; 32];
    w[24..].copy_from_slice(&n.to_be_bytes());
    w
}

/// Strip leading zero bytes (minimal big-endian); zero becomes the empty string.
fn minimal_be(b: &[u8; 32]) -> Vec<u8> {
    let first = b.iter().position(|&x| x != 0).unwrap_or(32);
    b[first..].to_vec()
}
