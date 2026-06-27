use alloc::collections::BTreeSet;
use alloc::vec::Vec;

use unicity_token::api::bft::{RootTrustBase, UnicityCertificate};
use unicity_token::api::InclusionProof;
use unicity_token::api::StateId;
use unicity_token::crypto::hash::{DataHash, HashAlgorithm};
use unicity_token::crypto::signature::Signature;
use unicity_token::predicate::builtin::SignaturePredicate;
use unicity_token::predicate::unlock::verify_signature_unlock;
use unicity_token::predicate::EncodedPredicate;
use unicity_token::transaction::{Minter, Token, Transaction};

use crate::{BridgeExtError, Result};

pub fn verify_token_anchored(
    token: &Token,
    trust_base: &RootTrustBase,
    anchor_certificate: &UnicityCertificate,
) -> Result<()> {
    let anchor_root = verify_anchor_certificate(trust_base, anchor_certificate)?;
    verify_token_against_root(token, trust_base, &anchor_root)
}

/// Verify a token in anchored mode against an **already-verified** anchor root.
/// Pair this with one [`verify_anchor_certificate`] call to amortize a single
/// BFT-quorum check across every token sharing the same anchor `UC*` (the §11
/// one-quorum-check shape): verify the anchor once, then verify each token's
/// transitions against the cached root without re-running the quorum.
pub fn verify_token_against_root(
    token: &Token,
    trust_base: &RootTrustBase,
    anchor_root: &[u8; 32],
) -> Result<()> {
    trust_base
        .validate()
        .map_err(|_| BridgeExtError::InvalidTrustBase)?;
    verify_genesis_against_root(token, trust_base, anchor_root)?;
    for transfer in token.transactions() {
        verify_inclusion_against_root(
            anchor_root,
            transfer.inclusion_proof(),
            transfer.transaction(),
        )
        .map_err(|_| BridgeExtError::Transfer)?;
    }
    Ok(())
}

pub fn verify_token_certified(token: &Token, trust_base: &RootTrustBase) -> Result<()> {
    trust_base
        .validate()
        .map_err(|_| BridgeExtError::InvalidTrustBase)?;
    verify_genesis_with_own_certificate(token, trust_base)?;
    for transfer in token.transactions() {
        verify_inclusion_with_own_certificate(
            trust_base,
            transfer.inclusion_proof(),
            transfer.transaction(),
        )
        .map_err(|_| BridgeExtError::Transfer)?;
    }
    Ok(())
}

pub fn verify_anchor_certificate(
    trust_base: &RootTrustBase,
    certificate: &UnicityCertificate,
) -> Result<[u8; 32]> {
    trust_base
        .validate()
        .map_err(|_| BridgeExtError::InvalidTrustBase)?;
    verify_unicity_certificate(trust_base, certificate)
}

pub fn verify_inclusion_against_root(
    expected_root: &[u8; 32],
    proof: &InclusionProof,
    transaction: &impl Transaction,
) -> Result<()> {
    let inclusion_certificate = proof
        .inclusion_certificate
        .as_ref()
        .ok_or(BridgeExtError::InclusionCertificateMissing)?;
    let certification_data = proof
        .certification_data
        .as_ref()
        .ok_or(BridgeExtError::CertificationDataMissing)?;

    if certification_data.lock_script() != transaction.lock_script()
        || certification_data.source_state_hash() != transaction.source_state_hash()
    {
        return Err(BridgeExtError::CertificationDataMismatch);
    }

    let tx_hash = transaction.calculate_transaction_hash();
    if certification_data.transaction_hash() != &tx_hash {
        return Err(BridgeExtError::TransactionHashMismatch);
    }

    let state_id = StateId::derive(transaction.lock_script(), transaction.source_state_hash());
    let expected_root = DataHash::new(HashAlgorithm::Sha256, *expected_root)
        .map_err(|_| BridgeExtError::PathInvalid)?;
    if !inclusion_certificate.verify(
        &state_id,
        certification_data.transaction_hash(),
        &expected_root,
    ) {
        return Err(BridgeExtError::PathInvalid);
    }

    let shard = &proof.unicity_certificate.shard_tree_certificate.shard;
    if shard.length() != 0 && !shard.is_prefix_of(state_id.bytes()) {
        return Err(BridgeExtError::ShardMismatch);
    }

    verify_predicate(
        transaction.lock_script(),
        transaction.source_state_hash(),
        certification_data.transaction_hash(),
        certification_data.unlock_script(),
    )
}

fn verify_genesis_with_own_certificate(token: &Token, trust_base: &RootTrustBase) -> Result<()> {
    let genesis = token.genesis();
    let mint = genesis.transaction();
    if mint.network_id() != trust_base.network_id {
        return Err(BridgeExtError::NetworkMismatch);
    }

    let expected_lock = SignaturePredicate::new(
        Minter::public_key(mint.token_id()).map_err(|_| BridgeExtError::InvalidMintLockScript)?,
    )
    .to_encoded();
    let certified_lock = genesis
        .inclusion_proof()
        .certification_data
        .as_ref()
        .map(|c| c.lock_script());
    if certified_lock != Some(&expected_lock) {
        return Err(BridgeExtError::InvalidMintLockScript);
    }

    verify_inclusion_with_own_certificate(trust_base, genesis.inclusion_proof(), mint)
        .map_err(|_| BridgeExtError::Genesis)
}

fn verify_inclusion_with_own_certificate(
    trust_base: &RootTrustBase,
    proof: &InclusionProof,
    transaction: &impl Transaction,
) -> Result<()> {
    let root = verify_unicity_certificate(trust_base, &proof.unicity_certificate)?;
    verify_inclusion_against_root(&root, proof, transaction)
}

fn verify_genesis_against_root(
    token: &Token,
    trust_base: &RootTrustBase,
    anchor_root: &[u8; 32],
) -> Result<()> {
    let genesis = token.genesis();
    let mint = genesis.transaction();
    if mint.network_id() != trust_base.network_id {
        return Err(BridgeExtError::NetworkMismatch);
    }

    let expected_lock = SignaturePredicate::new(
        Minter::public_key(mint.token_id()).map_err(|_| BridgeExtError::InvalidMintLockScript)?,
    )
    .to_encoded();
    let certified_lock = genesis
        .inclusion_proof()
        .certification_data
        .as_ref()
        .map(|c| c.lock_script());
    if certified_lock != Some(&expected_lock) {
        return Err(BridgeExtError::InvalidMintLockScript);
    }

    verify_inclusion_against_root(anchor_root, genesis.inclusion_proof(), mint)
        .map_err(|_| BridgeExtError::Genesis)
}

fn verify_unicity_certificate(
    trust_base: &RootTrustBase,
    certificate: &UnicityCertificate,
) -> Result<[u8; 32]> {
    let seal = &certificate.unicity_seal;
    if seal.network_id != trust_base.network_id {
        return Err(BridgeExtError::SealNetworkMismatch);
    }

    let computed = certificate
        .computed_seal_hash()
        .map_err(|_| BridgeExtError::SealRootMismatch)?;
    if computed.data() != seal.hash.as_slice() {
        return Err(BridgeExtError::SealRootMismatch);
    }

    let seal_hash = seal.calculate_hash();
    let mut counted: BTreeSet<&str> = BTreeSet::new();
    let mut counted_keys = Vec::new();
    for (node_id, signature) in &seal.signatures {
        if counted.contains(node_id.as_str()) {
            continue;
        }
        let Some(key) = trust_base.signing_key(node_id) else {
            continue;
        };
        if counted_keys.iter().any(|seen| seen == key) {
            continue;
        }
        let Ok(signature) = Signature::decode(signature) else {
            continue;
        };
        if signature.verify(seal_hash.data(), key) {
            counted.insert(node_id);
            counted_keys.push(key.clone());
        }
    }
    if (counted.len() as u64) < trust_base.quorum_threshold {
        return Err(BridgeExtError::QuorumNotMet);
    }
    certificate
        .input_record
        .hash
        .as_slice()
        .try_into()
        .map_err(|_| BridgeExtError::PathInvalid)
}

fn verify_predicate(
    lock_script: &EncodedPredicate,
    source_state_hash: &DataHash,
    transaction_hash: &DataHash,
    unlock_script: &[u8],
) -> Result<()> {
    let predicate = SignaturePredicate::from_encoded(lock_script)
        .map_err(|_| BridgeExtError::NotAuthenticated)?;
    if verify_signature_unlock(
        predicate.public_key(),
        source_state_hash,
        transaction_hash,
        unlock_script,
    ) {
        Ok(())
    } else {
        Err(BridgeExtError::NotAuthenticated)
    }
}
