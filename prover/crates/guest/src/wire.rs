use alloc::string::String;
use alloc::vec::Vec;

use bridge_return_core::{
    BridgeConfig, BridgeCoreError, PublicValues, Result, ReturnLeaf, SourceLockRef,
};
use bridge_return_sdk_ext::accumulator::{
    NonMembershipTerminal, NonMembershipWitness, SmtProofStep,
};
use unicity_token::api::bft::{RootTrustBase, RootTrustBaseNodeInfo, UnicityCertificate};
use unicity_token::api::NetworkId;
use unicity_token::cbor::Decoder;
use unicity_token::crypto::signature::PublicKey;
use unicity_token::transaction::Token;

use crate::{BridgeBurnWitness, BurnVerification, GuestInput, RelationWitness};

// v2 adds the per-burn verification mode tag (anchored vs certified).
const WIRE_VERSION: u32 = 2;

pub fn encode_guest_input(input: &GuestInput) -> Vec<u8> {
    let mut out = Vec::new();
    put_u32(&mut out, WIRE_VERSION);
    put_config(&mut out, &input.config);
    put_public_values(&mut out, &input.public_values);
    put_len(&mut out, input.return_leaves.len());
    for leaf in &input.return_leaves {
        put_return_leaf(&mut out, leaf);
    }
    put_len(&mut out, input.sorted_lock_refs.len());
    for lock_ref in &input.sorted_lock_refs {
        put_lock_ref(&mut out, lock_ref);
    }
    put_len(&mut out, input.witness.accumulator_witnesses.len());
    for witness in &input.witness.accumulator_witnesses {
        put_accumulator_witness(&mut out, witness);
    }
    put_len(&mut out, input.witness.bridge_burns.len());
    for burn in &input.witness.bridge_burns {
        put_bridge_burn(&mut out, burn);
    }
    out
}

pub fn decode_guest_input(bytes: &[u8]) -> Result<GuestInput> {
    let mut cursor = Cursor::new(bytes);
    if cursor.u32()? != WIRE_VERSION {
        return Err(BridgeCoreError::WireDecodeFailed);
    }
    let config = cursor.config()?;
    let public_values = cursor.public_values()?;
    let return_leaves = cursor.vec(Cursor::return_leaf)?;
    let sorted_lock_refs = cursor.vec(Cursor::lock_ref)?;
    let accumulator_witnesses = cursor.vec(Cursor::accumulator_witness)?;
    let bridge_burns = cursor.vec(Cursor::bridge_burn)?;
    if !cursor.is_empty() {
        return Err(BridgeCoreError::WireDecodeFailed);
    }
    Ok(GuestInput {
        config,
        public_values,
        return_leaves,
        sorted_lock_refs,
        witness: RelationWitness {
            accumulator_witnesses,
            bridge_burns,
        },
    })
}

fn put_config(out: &mut Vec<u8>, config: &BridgeConfig) {
    put_u64(out, config.source_chain_id);
    out.extend_from_slice(&config.vault);
    out.extend_from_slice(&config.asset);
    out.extend_from_slice(&config.token_type);
    out.extend_from_slice(&config.coin_id);
    put_u64(out, config.reason_tag);
    out.extend_from_slice(&config.lock_domain);
    out.extend_from_slice(&config.nullifier_domain);
}

fn put_public_values(out: &mut Vec<u8>, public_values: &PublicValues) {
    out.extend_from_slice(&public_values.domain_tag);
    out.extend_from_slice(&public_values.config_hash);
    out.extend_from_slice(&public_values.trust_base_hash);
    out.extend_from_slice(&public_values.spent_root_old);
    out.extend_from_slice(&public_values.spent_root_new);
    out.extend_from_slice(&public_values.return_root);
    out.extend_from_slice(&public_values.lock_ref_root);
    put_u32(out, public_values.batch_size);
    out.extend_from_slice(&public_values.total_amount);
}

fn put_return_leaf(out: &mut Vec<u8>, leaf: &ReturnLeaf) {
    out.extend_from_slice(&leaf.nullifier);
    out.extend_from_slice(&leaf.recipient);
    out.extend_from_slice(&leaf.amount);
    out.extend_from_slice(&leaf.fee_recipient);
    out.extend_from_slice(&leaf.fee_amount);
    put_u64(out, leaf.deadline);
}

fn put_lock_ref(out: &mut Vec<u8>, lock_ref: &SourceLockRef) {
    put_u64(out, lock_ref.nonce);
    out.extend_from_slice(&lock_ref.digest);
}

fn put_accumulator_witness(out: &mut Vec<u8>, witness: &NonMembershipWitness) {
    match witness.terminal() {
        NonMembershipTerminal::Empty => out.push(0),
        NonMembershipTerminal::Occupied { key } => {
            out.push(1);
            out.extend_from_slice(key);
        }
    }
    put_len(out, witness.steps().len());
    for step in witness.steps() {
        out.push(step.depth());
        out.extend_from_slice(step.sibling_hash());
    }
}

fn put_bridge_burn(out: &mut Vec<u8>, burn: &BridgeBurnWitness) {
    put_bytes(out, &burn.token.to_cbor());
    put_trust_base(out, &burn.trust_base);
    match &burn.verification {
        BurnVerification::Anchored(anchor) => {
            out.push(0);
            put_bytes(out, &anchor.to_cbor());
        }
        BurnVerification::Certified => out.push(1),
    }
    put_u64(out, burn.lock_justification_tag);
}

fn put_trust_base(out: &mut Vec<u8>, trust_base: &RootTrustBase) {
    put_u64(out, trust_base.version);
    put_u16(out, trust_base.network_id.id());
    put_u64(out, trust_base.epoch);
    put_u64(out, trust_base.epoch_start_round);
    put_len(out, trust_base.root_nodes.len());
    for node in &trust_base.root_nodes {
        put_bytes(out, node.node_id.as_bytes());
        put_bytes(out, node.signing_key.as_bytes());
        put_u64(out, node.stake);
    }
    put_u64(out, trust_base.quorum_threshold);
}

fn put_bytes(out: &mut Vec<u8>, bytes: &[u8]) {
    put_len(out, bytes.len());
    out.extend_from_slice(bytes);
}

fn put_len(out: &mut Vec<u8>, len: usize) {
    put_u32(
        out,
        u32::try_from(len).expect("wire vector length exceeds u32"),
    );
}

fn put_u16(out: &mut Vec<u8>, value: u16) {
    out.extend_from_slice(&value.to_be_bytes());
}

fn put_u32(out: &mut Vec<u8>, value: u32) {
    out.extend_from_slice(&value.to_be_bytes());
}

fn put_u64(out: &mut Vec<u8>, value: u64) {
    out.extend_from_slice(&value.to_be_bytes());
}

struct Cursor<'a> {
    bytes: &'a [u8],
}

impl<'a> Cursor<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Cursor { bytes }
    }

    fn is_empty(&self) -> bool {
        self.bytes.is_empty()
    }

    fn take(&mut self, len: usize) -> Result<&'a [u8]> {
        if self.bytes.len() < len {
            return Err(BridgeCoreError::WireDecodeFailed);
        }
        let (head, tail) = self.bytes.split_at(len);
        self.bytes = tail;
        Ok(head)
    }

    fn array<const N: usize>(&mut self) -> Result<[u8; N]> {
        self.take(N)?
            .try_into()
            .map_err(|_| BridgeCoreError::WireDecodeFailed)
    }

    fn u16(&mut self) -> Result<u16> {
        Ok(u16::from_be_bytes(self.array()?))
    }

    fn u32(&mut self) -> Result<u32> {
        Ok(u32::from_be_bytes(self.array()?))
    }

    fn u64(&mut self) -> Result<u64> {
        Ok(u64::from_be_bytes(self.array()?))
    }

    fn bytes(&mut self) -> Result<&'a [u8]> {
        let len = usize::try_from(self.u32()?).map_err(|_| BridgeCoreError::WireDecodeFailed)?;
        self.take(len)
    }

    fn vec<T>(&mut self, f: fn(&mut Self) -> Result<T>) -> Result<Vec<T>> {
        let len = usize::try_from(self.u32()?).map_err(|_| BridgeCoreError::WireDecodeFailed)?;
        let mut values = Vec::with_capacity(len);
        for _ in 0..len {
            values.push(f(self)?);
        }
        Ok(values)
    }

    fn config(&mut self) -> Result<BridgeConfig> {
        Ok(BridgeConfig {
            source_chain_id: self.u64()?,
            vault: self.array()?,
            asset: self.array()?,
            token_type: self.array()?,
            coin_id: self.array()?,
            reason_tag: self.u64()?,
            lock_domain: self.array()?,
            nullifier_domain: self.array()?,
        })
    }

    fn public_values(&mut self) -> Result<PublicValues> {
        Ok(PublicValues {
            domain_tag: self.array()?,
            config_hash: self.array()?,
            trust_base_hash: self.array()?,
            spent_root_old: self.array()?,
            spent_root_new: self.array()?,
            return_root: self.array()?,
            lock_ref_root: self.array()?,
            batch_size: self.u32()?,
            total_amount: self.array()?,
        })
    }

    fn return_leaf(&mut self) -> Result<ReturnLeaf> {
        Ok(ReturnLeaf {
            nullifier: self.array()?,
            recipient: self.array()?,
            amount: self.array()?,
            fee_recipient: self.array()?,
            fee_amount: self.array()?,
            deadline: self.u64()?,
        })
    }

    fn lock_ref(&mut self) -> Result<SourceLockRef> {
        Ok(SourceLockRef {
            nonce: self.u64()?,
            digest: self.array()?,
        })
    }

    fn accumulator_witness(&mut self) -> Result<NonMembershipWitness> {
        let terminal = match self.take(1)?[0] {
            0 => NonMembershipTerminal::Empty,
            1 => NonMembershipTerminal::Occupied { key: self.array()? },
            _ => return Err(BridgeCoreError::WireDecodeFailed),
        };
        let steps = self.vec(|cursor| {
            let depth = cursor.take(1)?[0];
            let sibling_hash = cursor.array()?;
            Ok(SmtProofStep::new(depth, sibling_hash))
        })?;
        Ok(NonMembershipWitness::new(terminal, steps))
    }

    fn bridge_burn(&mut self) -> Result<BridgeBurnWitness> {
        let token =
            Token::from_cbor(self.bytes()?).map_err(|_| BridgeCoreError::WireDecodeFailed)?;
        let trust_base = self.trust_base()?;
        let verification = match self.take(1)?[0] {
            0 => {
                let anchor_bytes = self.bytes()?;
                let decoder = Decoder::new(anchor_bytes);
                decoder
                    .finish()
                    .map_err(|_| BridgeCoreError::WireDecodeFailed)?;
                let anchor_certificate = UnicityCertificate::from_cbor(decoder)
                    .map_err(|_| BridgeCoreError::WireDecodeFailed)?;
                BurnVerification::Anchored(anchor_certificate)
            }
            1 => BurnVerification::Certified,
            _ => return Err(BridgeCoreError::WireDecodeFailed),
        };
        Ok(BridgeBurnWitness {
            token,
            trust_base,
            verification,
            lock_justification_tag: self.u64()?,
        })
    }

    fn trust_base(&mut self) -> Result<RootTrustBase> {
        let version = self.u64()?;
        let network_id =
            NetworkId::new(self.u16()?).map_err(|_| BridgeCoreError::WireDecodeFailed)?;
        let epoch = self.u64()?;
        let epoch_start_round = self.u64()?;
        let root_nodes = self.vec(|cursor| {
            let node_id = String::from_utf8(cursor.bytes()?.to_vec())
                .map_err(|_| BridgeCoreError::WireDecodeFailed)?;
            let signing_key = PublicKey::from_bytes(cursor.bytes()?)
                .map_err(|_| BridgeCoreError::WireDecodeFailed)?;
            Ok(RootTrustBaseNodeInfo {
                node_id,
                signing_key,
                stake: cursor.u64()?,
            })
        })?;
        let quorum_threshold = self.u64()?;
        let trust_base = RootTrustBase::new(
            version,
            network_id,
            epoch,
            epoch_start_round,
            root_nodes,
            quorum_threshold,
        );
        trust_base
            .validate()
            .map_err(|_| BridgeCoreError::WireDecodeFailed)?;
        Ok(trust_base)
    }
}
