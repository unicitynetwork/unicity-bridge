use alloc::vec::Vec;

use num_bigint::BigUint;
use tiny_keccak::{Hasher, Keccak};
use unicity_token::api::bft::{RootTrustBase, UnicityCertificate};
use unicity_token::cbor::DecodeLimits;
use unicity_token::cbor::Decoder;
use unicity_token::crypto::hash::sha256;
use unicity_token::payment::{
    split_output_commitment, AssetId, PaymentAssetCollection, SplitManifest, SplitMintJustification,
};
use unicity_token::predicate::builtin::BurnPredicate;
use unicity_token::predicate::EncodedPredicate;
use unicity_token::rsmst::encode_amount;
use unicity_token::transaction::Token;
use unicity_token::transaction::{CertifiedMintTransaction, Transaction};

use crate::verify::{verify_token_against_root, verify_token_anchored, verify_token_certified};
use crate::{BridgeExtError, Result};

const DOMAIN_LOCK: &str = "unicity-bridge-lock:v1";
const BRIDGE_LOCK_JUSTIFICATION_VERSION: u64 = 1;

pub const TRON_USDT_LOCK_JUSTIFICATION_TAG: u64 = 1_330_002;

/// [`PaymentDataDecoder`] for a bridged token's `data` field. Bridge tokens use
/// bare `PaymentAssetCollection` CBOR. Sphere-internal payment envelopes are
/// not bridge tokens and must fail this decoder.
pub fn decode_bridged_payment_data(
    bytes: &[u8],
) -> core::result::Result<PaymentAssetCollection, unicity_token::Error> {
    PaymentAssetCollection::from_cbor_bytes(bytes)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeConfig {
    pub source_chain_id: u64,
    pub vault: [u8; 20],
    pub asset: [u8; 20],
    pub token_type: [u8; 32],
    pub coin_id: [u8; 32],
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeLockJustification {
    pub source_chain_id: u64,
    pub vault: [u8; 20],
    pub asset: [u8; 20],
    pub txid: [u8; 32],
    pub log_index: u64,
    pub amount: u64,
    pub nonce: u64,
}

impl BridgeLockJustification {
    pub fn from_cbor_bytes(bytes: &[u8], expected_tag: u64) -> Result<Self> {
        let decoder = Decoder::new(bytes);
        decoder
            .finish()
            .map_err(|_| BridgeExtError::BridgeLockJustificationMalformed)?;
        let inner = decoder
            .expect_tag(expected_tag)
            .map_err(|_| BridgeExtError::BridgeLockJustificationMalformed)?;
        let items = inner
            .array(Some(8))
            .map_err(|_| BridgeExtError::BridgeLockJustificationMalformed)?;
        if items[0]
            .uint()
            .map_err(|_| BridgeExtError::BridgeLockJustificationMalformed)?
            != BRIDGE_LOCK_JUSTIFICATION_VERSION
        {
            return Err(BridgeExtError::BridgeLockJustificationMalformed);
        }
        Ok(Self {
            source_chain_id: items[1]
                .uint()
                .map_err(|_| BridgeExtError::BridgeLockJustificationMalformed)?,
            vault: bytes_n(items[2])?,
            asset: bytes_n(items[3])?,
            txid: bytes_n(items[4])?,
            log_index: items[5]
                .uint()
                .map_err(|_| BridgeExtError::BridgeLockJustificationMalformed)?,
            amount: items[6]
                .uint()
                .map_err(|_| BridgeExtError::BridgeLockJustificationMalformed)?,
            nonce: items[7]
                .uint()
                .map_err(|_| BridgeExtError::BridgeLockJustificationMalformed)?,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BridgeLockObligation {
    pub nonce: u64,
    pub digest: [u8; 32],
}

pub type PaymentDataDecoder =
    fn(&[u8]) -> core::result::Result<PaymentAssetCollection, unicity_token::Error>;

pub fn bridge_lock_obligation(
    genesis: &CertifiedMintTransaction,
    justification_tag: u64,
    config: &BridgeConfig,
    decode_payment_data: PaymentDataDecoder,
) -> Result<BridgeLockObligation> {
    let mint = genesis.transaction();
    if mint.token_type().bytes() != config.token_type {
        return Err(BridgeExtError::BridgeLockTokenTypeMismatch);
    }
    let justification = BridgeLockJustification::from_cbor_bytes(
        mint.justification()
            .ok_or(BridgeExtError::BridgeLockJustificationMalformed)?,
        justification_tag,
    )?;
    if justification.source_chain_id != config.source_chain_id
        || justification.vault != config.vault
        || justification.asset != config.asset
    {
        return Err(BridgeExtError::BridgeLockConfigMismatch);
    }

    let assets = decode_payment_data(
        mint.data()
            .ok_or(BridgeExtError::BridgeLockPaymentDataMissing)?,
    )
    .map_err(|_| BridgeExtError::BridgeLockPaymentDataMalformed)?;
    let asset = assets
        .get(&AssetId::new(config.coin_id.to_vec()))
        .ok_or(BridgeExtError::BridgeLockCoinMissing)?;
    if asset.value() != &BigUint::from(justification.amount) {
        return Err(BridgeExtError::BridgeLockAmountMismatch);
    }

    let recipient_commitment = sha256(&mint.recipient().to_cbor());
    let amount =
        u256_from_biguint(asset.value()).ok_or(BridgeExtError::BridgeLockAmountMismatch)?;
    let digest = lock_digest(
        config,
        justification.nonce,
        &amount,
        mint.token_id().bytes(),
        recipient_commitment
            .data()
            .try_into()
            .expect("sha256 digest is always 32 bytes"),
    );
    Ok(BridgeLockObligation {
        nonce: justification.nonce,
        digest,
    })
}

pub fn bridge_lock_obligations_for_token_anchored(
    token: &Token,
    trust_base: &RootTrustBase,
    anchor_certificate: &UnicityCertificate,
    bridge_justification_tag: u64,
    config: &BridgeConfig,
    decode_payment_data: PaymentDataDecoder,
) -> Result<Vec<BridgeLockObligation>> {
    verify_token_anchored(token, trust_base, anchor_certificate)?;
    bridge_lock_obligations_for_verified_token(
        token,
        trust_base,
        bridge_justification_tag,
        config,
        decode_payment_data,
    )
}

/// Like [`bridge_lock_obligations_for_token_anchored`], but against an
/// **already-verified** anchor root. The caller runs one
/// [`verify_anchor_certificate`](crate::verify::verify_anchor_certificate) per
/// distinct anchor and reuses the root here, so a batch sharing one `UC*` pays a
/// single BFT-quorum check (the §11 one-quorum-check shape).
pub fn bridge_lock_obligations_for_token_against_root(
    token: &Token,
    trust_base: &RootTrustBase,
    anchor_root: &[u8; 32],
    bridge_justification_tag: u64,
    config: &BridgeConfig,
    decode_payment_data: PaymentDataDecoder,
) -> Result<Vec<BridgeLockObligation>> {
    verify_token_against_root(token, trust_base, anchor_root)?;
    bridge_lock_obligations_for_verified_token(
        token,
        trust_base,
        bridge_justification_tag,
        config,
        decode_payment_data,
    )
}

fn bridge_lock_obligations_for_verified_token(
    token: &Token,
    trust_base: &RootTrustBase,
    bridge_justification_tag: u64,
    config: &BridgeConfig,
    decode_payment_data: PaymentDataDecoder,
) -> Result<Vec<BridgeLockObligation>> {
    // A genesis that isn't a direct bridge-lock mint is *assumed* to be a split
    // output and re-tried against SplitMintJustification. But that assumption
    // can be wrong (e.g. a real config/tag/amount mismatch on a genuinely
    // direct mint) — surface the direct-path error in that case instead of the
    // split path's unrelated (and misleading) decode failure.
    let direct_err = match bridge_lock_obligation(
        token.genesis(),
        bridge_justification_tag,
        config,
        decode_payment_data,
    ) {
        Ok(obligation) => return Ok(alloc::vec![obligation]),
        Err(err) => err,
    };

    let mint = token.genesis().transaction();
    let Some(justification_bytes) = mint.justification() else {
        return Err(direct_err);
    };
    let Ok(justification) = SplitMintJustification::from_cbor(justification_bytes) else {
        return Err(direct_err);
    };
    verify_split_mint(
        token,
        &justification,
        trust_base,
        bridge_justification_tag,
        config,
        decode_payment_data,
    )
}

fn verify_split_mint(
    output: &Token,
    justification: &SplitMintJustification,
    trust_base: &RootTrustBase,
    bridge_justification_tag: u64,
    config: &BridgeConfig,
    decode_payment_data: PaymentDataDecoder,
) -> Result<Vec<BridgeLockObligation>> {
    let mint = output.genesis().transaction();
    let output_payment_bytes = mint
        .data()
        .ok_or(BridgeExtError::BridgeLockPaymentDataMissing)?;
    let output_assets = decode_payment_data(output_payment_bytes)
        .map_err(|_| BridgeExtError::BridgeLockPaymentDataMalformed)?;
    let burned = justification.token();
    if mint.network_id() != burned.genesis().transaction().network_id() {
        return Err(BridgeExtError::SplitNetworkMismatch);
    }

    let obligations = bridge_lock_obligations_for_token_certified(
        burned,
        trust_base,
        bridge_justification_tag,
        config,
        decode_payment_data,
    )?;

    let burn_transfer = burned
        .transactions()
        .last()
        .ok_or(BridgeExtError::SplitBurnTransferMissing)?;
    let manifest_bytes = burn_transfer
        .transaction()
        .data()
        .ok_or(BridgeExtError::SplitManifestMissing)?;
    let manifest = SplitManifest::from_cbor_bytes(manifest_bytes, DecodeLimits::DEFAULT)
        .map_err(|_| BridgeExtError::SplitManifestMalformed)?;
    let expected_burn =
        EncodedPredicate::from_predicate(&BurnPredicate::new(manifest.reason_hash().to_vec()));
    if burn_transfer.recipient() != &expected_burn {
        return Err(BridgeExtError::SplitBurnPredicateMismatch);
    }

    let source = burned.genesis().transaction();
    if mint.token_type() != source.token_type() {
        return Err(BridgeExtError::SplitTokenTypeMismatch);
    }
    let source_payment_bytes = source
        .data()
        .ok_or(BridgeExtError::SplitSourcePaymentDataMissing)?;
    let source_assets = decode_payment_data(source_payment_bytes)
        .map_err(|_| BridgeExtError::SplitSourcePaymentDataMissing)?;
    if source_assets.len() != manifest.len() {
        return Err(BridgeExtError::SplitManifestLengthMismatch);
    }
    if justification.proofs().len() != output_assets.len() {
        return Err(BridgeExtError::SplitProofCountMismatch);
    }

    let output_data = mint.data().ok_or(BridgeExtError::SplitMalformed)?;
    let commitment = split_output_commitment(
        burned.id(),
        mint.network_id(),
        mint.recipient(),
        mint.salt(),
        mint.token_id(),
        mint.token_type(),
        output_data,
    );
    for (asset, proof) in output_assets.as_slice().iter().zip(justification.proofs()) {
        let index = source_assets
            .as_slice()
            .iter()
            .position(|source_asset| source_asset.id() == asset.id())
            .ok_or(BridgeExtError::SplitSourceAssetMissing)?;
        let root = &manifest.roots()[index];
        let root_sum = proof
            .verify(mint.token_id().bytes(), &commitment, asset.value(), root)
            .ok_or(BridgeExtError::SplitAllocationProofInvalid)?;
        if &root_sum != source_assets.as_slice()[index].value() {
            return Err(BridgeExtError::SplitSourceAmountMismatch);
        }
    }

    Ok(obligations)
}

/// Verify a token in **certified** mode (each transition carries its own
/// `UnicityCertificate`, as served by a live aggregator) and collect its
/// bridge-lock obligations. This is the S1 entry for real testnet tokens; the
/// anchored variants ([`bridge_lock_obligations_for_token_anchored`] /
/// [`bridge_lock_obligations_for_token_against_root`]) apply once the aggregator
/// serves historical inclusion proofs against one shared anchor (§11 batching).
pub fn bridge_lock_obligations_for_token_certified(
    token: &Token,
    trust_base: &RootTrustBase,
    bridge_justification_tag: u64,
    config: &BridgeConfig,
    decode_payment_data: PaymentDataDecoder,
) -> Result<Vec<BridgeLockObligation>> {
    verify_token_certified(token, trust_base)?;
    bridge_lock_obligations_for_verified_token(
        token,
        trust_base,
        bridge_justification_tag,
        config,
        decode_payment_data,
    )
}

pub fn lock_digest(
    config: &BridgeConfig,
    nonce: u64,
    amount: &[u8; 32],
    token_id: &[u8; 32],
    recipient_commitment: &[u8; 32],
) -> [u8; 32] {
    keccak256(&abi_encode(&[
        AbiValue::Str(DOMAIN_LOCK),
        AbiValue::U64(config.source_chain_id),
        AbiValue::Address(config.vault),
        AbiValue::U256(u256_from_u64(nonce)),
        AbiValue::Address(config.asset),
        AbiValue::Bytes32(config.token_type),
        AbiValue::Bytes32(config.coin_id),
        AbiValue::U256(*amount),
        AbiValue::Bytes32(*token_id),
        AbiValue::Bytes32(*recipient_commitment),
    ]))
}

fn bytes_n<const N: usize>(decoder: Decoder<'_>) -> Result<[u8; N]> {
    let bytes = decoder
        .bytes_value()
        .map_err(|_| BridgeExtError::BridgeLockJustificationMalformed)?;
    bytes.try_into().map_err(|_| match N {
        20 => BridgeExtError::BridgeLockAddressLength,
        32 => BridgeExtError::BridgeLockBytes32Length,
        _ => BridgeExtError::BridgeLockJustificationMalformed,
    })
}

fn u256_from_u64(value: u64) -> [u8; 32] {
    let mut out = [0u8; 32];
    out[24..].copy_from_slice(&value.to_be_bytes());
    out
}

fn u256_from_biguint(value: &BigUint) -> Option<[u8; 32]> {
    let encoded = encode_amount(value);
    if encoded.len() > 32 {
        return None;
    }
    let mut out = [0u8; 32];
    out[32 - encoded.len()..].copy_from_slice(&encoded);
    Some(out)
}

#[derive(Clone, Copy)]
enum AbiValue {
    Str(&'static str),
    U64(u64),
    U256([u8; 32]),
    Address([u8; 20]),
    Bytes32([u8; 32]),
}

impl AbiValue {
    fn is_dynamic(self) -> bool {
        matches!(self, AbiValue::Str(_))
    }

    fn word(self) -> [u8; 32] {
        let mut out = [0u8; 32];
        match self {
            AbiValue::U64(value) => out[24..].copy_from_slice(&value.to_be_bytes()),
            AbiValue::U256(value) | AbiValue::Bytes32(value) => out.copy_from_slice(&value),
            AbiValue::Address(value) => out[12..].copy_from_slice(&value),
            AbiValue::Str(_) => panic!("dynamic ABI string has no static word"),
        }
        out
    }
}

fn abi_encode(values: &[AbiValue]) -> Vec<u8> {
    let head_size = values.len() * 32;
    let mut head = Vec::with_capacity(head_size);
    let mut tail = Vec::new();
    for value in values {
        if value.is_dynamic() {
            head.extend_from_slice(&u256_from_u64((head_size + tail.len()) as u64));
            if let AbiValue::Str(text) = value {
                tail.extend_from_slice(&u256_from_u64(text.len() as u64));
                let mut bytes = text.as_bytes().to_vec();
                let rem = bytes.len() % 32;
                if rem != 0 {
                    bytes.resize(bytes.len() + 32 - rem, 0);
                }
                tail.extend_from_slice(&bytes);
            }
        } else {
            head.extend_from_slice(&value.word());
        }
    }
    head.extend_from_slice(&tail);
    head
}

fn keccak256(input: &[u8]) -> [u8; 32] {
    let mut hasher = Keccak::v256();
    let mut out = [0u8; 32];
    hasher.update(input);
    hasher.finalize(&mut out);
    out
}
