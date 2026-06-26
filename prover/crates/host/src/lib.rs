use std::{fs, path::Path};

use bridge_return_core::{
    burn_transition_id, coin_id, config_hash, domain_tag, lock_digest, nullifier,
    public_values_abi, reason_cbor, reason_hash, recipient_commitment, return_root,
    sorted_lock_ref_root, token_type, BridgeBackReason, BridgeConfig, LockRecord, PublicValues,
    ReturnLeaf, SourceLockRef,
};
use bridge_return_guest::{execute, BridgeBurnWitness, GuestInput, RelationWitness};
use serde_json::Value;
use thiserror::Error;
use unicity_token::accumulator::{
    insert as accumulator_insert, verify_non_member, NonMembershipTerminal, NonMembershipWitness,
    SmtProofStep, EMPTY_TREE_ROOT,
};
use unicity_token::api::bft::{RootTrustBase, RootTrustBaseNodeInfo, UnicityCertificate};
use unicity_token::api::NetworkId;
use unicity_token::cbor::Decoder;
use unicity_token::crypto::signature::PublicKey;
use unicity_token::transaction::Token;

#[derive(Debug, Error)]
pub enum HostError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
    #[error("hex: {0}")]
    Hex(#[from] hex::FromHexError),
    #[error("{0}")]
    Check(String),
}

pub type Result<T> = std::result::Result<T, HostError>;

pub fn check_vectors(root: &Path) -> Result<()> {
    let version = fs::read_to_string(root.join("VERSION"))?;
    if version.trim() != bridge_return_core::BRIDGE_PROTO_VERSION.to_string() {
        return Err(HostError::Check(format!(
            "BRIDGE_PROTO_VERSION mismatch: vectors={}, core={}",
            version.trim(),
            bridge_return_core::BRIDGE_PROTO_VERSION
        )));
    }
    check_config(&read(root, "config/config-00.json")?)?;
    check_lock(
        &read(root, "lock/lock-00.json")?,
        &read(root, "config/config-00.json")?,
    )?;
    check_reason(&read(root, "reason/reason-00.json")?)?;
    check_nullifier(&read(root, "nullifier/nullifier-00.json")?)?;
    check_accumulator(&read(root, "accumulator/accumulator-00.json")?)?;
    check_public(
        &read(root, "public/public-00.json")?,
        &read(root, "config/config-00.json")?,
    )?;
    check_token(
        &read(root, "token/token-00.json")?,
        &read(root, "config/config-00.json")?,
    )?;
    println!("bridge return prover vectors ok");
    Ok(())
}

fn read(root: &Path, relative: &str) -> Result<Value> {
    Ok(serde_json::from_slice(&fs::read(root.join(relative))?)?)
}

fn check_config(v: &Value) -> Result<()> {
    let input = &v["in"];
    let output = &v["out"];
    let cfg = config_from_json(input, output)?;
    eq_hex(
        "token_type",
        token_type(
            str_field(input, "chain_id_str")?,
            str_field(input, "asset_evm_hex")?,
        ),
        output,
        "token_type",
    )?;
    eq_hex(
        "coin_id",
        coin_id(
            str_field(input, "chain_id_str")?,
            str_field(input, "asset_evm_hex")?,
        ),
        output,
        "coin_id",
    )?;
    eq_hex("config_hash", config_hash(&cfg), output, "config_hash")
}

fn check_lock(lock: &Value, config: &Value) -> Result<()> {
    let cfg = config_from_json(&config["in"], &config["out"])?;
    let input = &lock["in"];
    let output = &lock["out"];
    let recipient_cbor = bytes_field(input, "recipient_cbor")?;
    let record = LockRecord {
        nonce: u64_field(input, "nonce")?,
        amount: b32_field(input, "amount")?,
        unicity_token_id: b32_field(input, "unicity_token_id")?,
        recipient_commitment: recipient_commitment(&recipient_cbor),
    };
    eq_hex(
        "recipient_commitment",
        record.recipient_commitment,
        output,
        "recipient_commitment",
    )?;
    eq_hex(
        "lock_digest",
        lock_digest(&cfg, &record),
        output,
        "lock_digest",
    )
}

fn check_reason(v: &Value) -> Result<()> {
    let input = &v["in"];
    let output = &v["out"];
    let cfg = BridgeConfig {
        source_chain_id: u64_field(input, "source_chain_id")?,
        vault: b20_field(input, "vault")?,
        asset: b20_field(input, "asset")?,
        token_type: b32_field(input, "token_type")?,
        coin_id: b32_field(input, "coin_id")?,
        reason_tag: u64_field(input, "reason_tag")?,
        lock_domain: [0u8; 32],
        nullifier_domain: [0u8; 32],
    };
    let reason = BridgeBackReason {
        version: u64_field(input, "version")?,
        recipient: b20_field(input, "recipient")?,
        amount: b32_field(input, "amount")?,
        fee_recipient: b20_field(input, "fee_recipient")?,
        fee_amount: b32_field(input, "fee_amount")?,
        deadline: u64_field(input, "deadline")?,
    };
    let reason_bytes = reason_cbor(&cfg, &reason);
    eq_hex_vec("reason_cbor", &reason_bytes, output, "reason_cbor")?;
    eq_hex(
        "reason_hash",
        reason_hash(&reason_bytes),
        output,
        "reason_hash",
    )
}

fn check_nullifier(v: &Value) -> Result<()> {
    let input = &v["in"];
    let output = &v["out"];
    let burn_id = burn_transition_id(
        &b32_field(input, "state_id")?,
        &b32_field(input, "tx_hash")?,
    );
    eq_hex("burn_transition_id", burn_id, output, "burn_transition_id")?;
    eq_hex(
        "nullifier",
        nullifier(&b32_field(input, "config_hash")?, &burn_id),
        output,
        "nullifier",
    )
}

fn check_accumulator(v: &Value) -> Result<()> {
    let input = &v["in"];
    let output = &v["out"];
    let empty_root = b32_field(input, "empty_root")?;
    if empty_root != EMPTY_TREE_ROOT {
        return Err(HostError::Check(format!(
            "empty accumulator root mismatch: actual=0x{} expected=0x{}",
            hex::encode(EMPTY_TREE_ROOT),
            hex::encode(empty_root)
        )));
    }

    let nullifiers = array_field(input, "nullifiers")?
        .iter()
        .map(|value| b32_value(value, "nullifier"))
        .collect::<Result<Vec<_>>>()?;
    let roots = array_field(output, "roots")?
        .iter()
        .map(|value| b32_value(value, "root"))
        .collect::<Result<Vec<_>>>()?;
    let witnesses = array_field(output, "witnesses")?;
    if witnesses.len() != nullifiers.len() || roots.len() != nullifiers.len() {
        return Err(HostError::Check(
            "accumulator vector length mismatch".to_string(),
        ));
    }

    let mut running = EMPTY_TREE_ROOT;
    for (idx, nullifier) in nullifiers.iter().enumerate() {
        let entry = &witnesses[idx];
        if b32_field(entry, "nullifier")? != *nullifier {
            return Err(HostError::Check(format!(
                "accumulator witness {idx} nullifier mismatch"
            )));
        }
        if b32_field(entry, "against_root")? != running {
            return Err(HostError::Check(format!(
                "accumulator witness {idx} against_root mismatch"
            )));
        }
        let witness = non_membership_witness_from_json(entry)?;
        if !verify_non_member(&running, nullifier, &witness) {
            return Err(HostError::Check(format!(
                "accumulator witness {idx} does not verify"
            )));
        }
        running = accumulator_insert(&running, nullifier, &witness)
            .ok_or_else(|| HostError::Check(format!("accumulator insert {idx} failed")))?;
        if running != roots[idx] || running != b32_field(entry, "new_root")? {
            return Err(HostError::Check(format!(
                "accumulator root {idx} mismatch: actual=0x{} expected=0x{}",
                hex::encode(running),
                hex::encode(roots[idx])
            )));
        }
    }
    Ok(())
}

fn check_public(public: &Value, config: &Value) -> Result<()> {
    let cfg = config_from_json(&config["in"], &config["out"])?;
    let input = &public["in"];
    let output = &public["out"];
    let leaves = array_field(input, "leaves")?
        .iter()
        .map(return_leaf_from_json)
        .collect::<Result<Vec<_>>>()?;
    let refs = array_field(input, "lock_refs")?
        .iter()
        .map(lock_ref_from_json)
        .collect::<Result<Vec<_>>>()?;
    let return_root_value = return_root(&leaves);
    let lock_ref_root_value = sorted_lock_ref_root(&refs)
        .map_err(|err| HostError::Check(format!("lock ref root: {err:?}")))?;
    let public_values = PublicValues {
        domain_tag: domain_tag(),
        config_hash: config_hash(&cfg),
        trust_base_hash: b32_field(input, "trust_base_hash")?,
        spent_root_old: b32_field(input, "spent_root_old")?,
        spent_root_new: b32_field(input, "spent_root_new")?,
        return_root: return_root_value,
        lock_ref_root: lock_ref_root_value,
        batch_size: u64_field(input, "batch_size")? as u32,
        total_amount: b32_field(input, "total_amount")?,
    };
    eq_hex("domain_tag", public_values.domain_tag, output, "domain_tag")?;
    eq_hex("return_root", return_root_value, output, "return_root")?;
    eq_hex(
        "lock_ref_root",
        lock_ref_root_value,
        output,
        "lock_ref_root",
    )?;
    eq_hex(
        "config_hash",
        public_values.config_hash,
        output,
        "config_hash",
    )?;
    eq_hex_vec(
        "public_values_abi",
        &public_values_abi(&public_values),
        output,
        "public_values_abi",
    )
}

fn check_token(token: &Value, config: &Value) -> Result<()> {
    let cfg = config_from_json(&config["in"], &config["out"])?;
    let input = &token["in"];
    let output = &token["out"];
    let leaves = array_field(input, "leaves")?
        .iter()
        .map(return_leaf_from_json)
        .collect::<Result<Vec<_>>>()?;
    let refs = array_field(input, "lock_refs")?
        .iter()
        .map(lock_ref_from_json)
        .collect::<Result<Vec<_>>>()?;
    let accumulator_witnesses = array_field(input, "accumulator_witnesses")?
        .iter()
        .map(non_membership_witness_from_json)
        .collect::<Result<Vec<_>>>()?;
    let burned_token = Token::from_cbor(&bytes_field(input, "token_cbor")?)
        .map_err(|err| HostError::Check(format!("token decode: {err}")))?;
    let trust_base = trust_base_from_json(&input["trust_base"])?;
    let anchor_bytes = bytes_field(input, "anchor_certificate_cbor")?;
    let anchor_decoder = Decoder::new(&anchor_bytes);
    anchor_decoder
        .finish()
        .map_err(|err| HostError::Check(format!("anchor certificate trailing data: {err}")))?;
    let anchor_certificate = UnicityCertificate::from_cbor(anchor_decoder)
        .map_err(|err| HostError::Check(format!("anchor certificate decode: {err}")))?;
    let public_values = public_values_from_json(output)?;
    let relation_input = GuestInput {
        config: cfg,
        public_values,
        return_leaves: leaves,
        sorted_lock_refs: refs,
        witness: RelationWitness {
            accumulator_witnesses,
            bridge_burns: vec![BridgeBurnWitness {
                token: burned_token,
                trust_base,
                anchor_certificate,
                lock_justification_tag: u64_field(input, "lock_justification_tag")?,
            }],
        },
    };
    let actual = execute(&relation_input)
        .map_err(|err| HostError::Check(format!("guest token vector rejected: {err:?}")))?;
    if actual != public_values {
        return Err(HostError::Check("token public values mismatch".to_string()));
    }
    eq_hex_vec(
        "public_values_abi",
        &public_values_abi(&public_values),
        output,
        "public_values_abi",
    )
}

fn config_from_json(input: &Value, output: &Value) -> Result<BridgeConfig> {
    Ok(BridgeConfig {
        source_chain_id: u64_field(input, "source_chain_id")?,
        vault: b20_field(input, "vault")?,
        asset: b20_field(input, "asset")?,
        token_type: b32_field(output, "token_type")?,
        coin_id: b32_field(output, "coin_id")?,
        reason_tag: u64_field(input, "reason_tag")?,
        lock_domain: b32_field(input, "lock_domain")?,
        nullifier_domain: b32_field(input, "nullifier_domain")?,
    })
}

fn return_leaf_from_json(v: &Value) -> Result<ReturnLeaf> {
    Ok(ReturnLeaf {
        nullifier: b32_field(v, "nullifier")?,
        recipient: b20_field(v, "recipient")?,
        amount: b32_field(v, "amount")?,
        fee_recipient: b20_field(v, "fee_recipient")?,
        fee_amount: b32_field(v, "fee_amount")?,
        deadline: u64_field(v, "deadline")?,
    })
}

fn lock_ref_from_json(v: &Value) -> Result<SourceLockRef> {
    Ok(SourceLockRef {
        nonce: u64_field(v, "nonce")?,
        digest: b32_field(v, "digest")?,
    })
}

fn public_values_from_json(v: &Value) -> Result<PublicValues> {
    Ok(PublicValues {
        domain_tag: b32_field(v, "domain_tag")?,
        config_hash: b32_field(v, "config_hash")?,
        trust_base_hash: b32_field(v, "trust_base_hash")?,
        spent_root_old: b32_field(v, "spent_root_old")?,
        spent_root_new: b32_field(v, "spent_root_new")?,
        return_root: b32_field(v, "return_root")?,
        lock_ref_root: b32_field(v, "lock_ref_root")?,
        batch_size: u64_field(v, "batch_size")? as u32,
        total_amount: b32_field(v, "total_amount")?,
    })
}

fn trust_base_from_json(v: &Value) -> Result<RootTrustBase> {
    let nodes = array_field(v, "root_nodes")?
        .iter()
        .map(|node| {
            Ok(RootTrustBaseNodeInfo {
                node_id: str_field(node, "node_id")?.to_string(),
                signing_key: PublicKey::from_bytes(&bytes_field(node, "signing_key")?)
                    .map_err(|err| HostError::Check(format!("trust base signing key: {err}")))?,
                stake: u64_field(node, "stake")?,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    let network_id = NetworkId::new(
        u16::try_from(u64_field(v, "network_id")?)
            .map_err(|_| HostError::Check("network_id exceeds u16".to_string()))?,
    )
    .map_err(|err| HostError::Check(format!("network_id: {err}")))?;
    let trust_base = RootTrustBase::new(
        u64_field(v, "version")?,
        network_id,
        u64_field(v, "epoch")?,
        u64_field(v, "epoch_start_round")?,
        nodes,
        u64_field(v, "quorum_threshold")?,
    );
    trust_base
        .validate()
        .map_err(|err| HostError::Check(format!("trust base: {err}")))?;
    Ok(trust_base)
}

fn non_membership_witness_from_json(v: &Value) -> Result<NonMembershipWitness> {
    let terminal = match str_field(&v["terminal"], "kind")? {
        "empty" => NonMembershipTerminal::Empty,
        "occupied" => NonMembershipTerminal::Occupied {
            key: b32_field(&v["terminal"], "key")?,
        },
        other => {
            return Err(HostError::Check(format!(
                "unknown accumulator terminal kind {other}"
            )))
        }
    };
    let steps = array_field(v, "steps")?
        .iter()
        .map(|step| {
            Ok(SmtProofStep::new(
                u8::try_from(u64_field(step, "depth")?).map_err(|_| {
                    HostError::Check("accumulator step depth exceeds u8".to_string())
                })?,
                b32_field(step, "sibling_hash")?,
            ))
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(NonMembershipWitness::new(terminal, steps))
}

fn str_field<'a>(v: &'a Value, field: &str) -> Result<&'a str> {
    v[field]
        .as_str()
        .ok_or_else(|| HostError::Check(format!("missing string field {field}")))
}

fn u64_field(v: &Value, field: &str) -> Result<u64> {
    v[field]
        .as_u64()
        .ok_or_else(|| HostError::Check(format!("missing u64 field {field}")))
}

fn array_field<'a>(v: &'a Value, field: &str) -> Result<&'a Vec<Value>> {
    v[field]
        .as_array()
        .ok_or_else(|| HostError::Check(format!("missing array field {field}")))
}

fn bytes_field(v: &Value, field: &str) -> Result<Vec<u8>> {
    let value = str_field(v, field)?;
    bytes_from_hex(value)
}

fn b32_value(v: &Value, what: &str) -> Result<[u8; 32]> {
    fixed(
        bytes_from_hex(
            v.as_str()
                .ok_or_else(|| HostError::Check(format!("missing hex string {what}")))?,
        )?,
        what,
    )
}

fn bytes_from_hex(value: &str) -> Result<Vec<u8>> {
    Ok(hex::decode(value.strip_prefix("0x").unwrap_or(value))?)
}

fn b20_field(v: &Value, field: &str) -> Result<[u8; 20]> {
    fixed(bytes_field(v, field)?, field)
}

fn b32_field(v: &Value, field: &str) -> Result<[u8; 32]> {
    fixed(bytes_field(v, field)?, field)
}

fn fixed<const N: usize>(bytes: Vec<u8>, field: &str) -> Result<[u8; N]> {
    bytes.try_into().map_err(|bytes: Vec<u8>| {
        HostError::Check(format!("{field} length {}, expected {N}", bytes.len()))
    })
}

fn eq_hex(name: &str, actual: [u8; 32], output: &Value, field: &str) -> Result<()> {
    eq_hex_vec(name, &actual, output, field)
}

fn eq_hex_vec(name: &str, actual: &[u8], output: &Value, field: &str) -> Result<()> {
    let expected = bytes_field(output, field)?;
    if actual != expected {
        return Err(HostError::Check(format!(
            "{name} mismatch: actual=0x{} expected=0x{}",
            hex::encode(actual),
            hex::encode(expected)
        )));
    }
    Ok(())
}
