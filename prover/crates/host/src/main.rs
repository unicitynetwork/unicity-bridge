use std::{env, path::PathBuf, process};

fn main() {
    let mut args = env::args().skip(1);
    let Some(cmd) = args.next() else {
        usage();
        return;
    };

    let result = match cmd.as_str() {
        "check-vectors" => {
            let root = args
                .next()
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("../bridge-vectors"));
            bridge_return_host::check_vectors(&root)
        }
        "emit-b1-token-vector" => {
            let fixture = bridge_return_host::fixture::build_b1_direct_bridge_fixture();
            println!(
                "{}",
                serde_json::to_string_pretty(&bridge_return_host::fixture::b1_fixture_json(
                    &fixture
                ))
                .expect("serialize fixture")
            );
            Ok(())
        }
        "emit-split-token-vector" => {
            let fixture = bridge_return_host::fixture::build_split_bridge_fixture();
            println!(
                "{}",
                serde_json::to_string_pretty(&bridge_return_host::fixture::split_fixture_json(
                    &fixture
                ))
                .expect("serialize fixture")
            );
            Ok(())
        }
        "emit-b1-wire-input" => {
            let fixture = bridge_return_host::fixture::build_b1_direct_bridge_fixture();
            println!(
                "0x{}",
                hex::encode(bridge_return_guest::wire::encode_guest_input(
                    &fixture.input
                ))
            );
            Ok(())
        }
        "emit-split-wire-input" => {
            let fixture = bridge_return_host::fixture::build_split_bridge_fixture();
            println!(
                "0x{}",
                hex::encode(bridge_return_guest::wire::encode_guest_input(
                    &fixture.input
                ))
            );
            Ok(())
        }
        "emit-b2-token-vector" => {
            let input = bridge_return_host::fixture::build_b2_direct_bridge_fixture();
            println!(
                "{}",
                serde_json::to_string_pretty(&bridge_return_host::fixture::b2_fixture_json(&input))
                    .expect("serialize fixture")
            );
            Ok(())
        }
        "emit-b2-wire-input" => {
            let input = bridge_return_host::fixture::build_b2_direct_bridge_fixture();
            println!(
                "0x{}",
                hex::encode(bridge_return_guest::wire::encode_guest_input(&input))
            );
            Ok(())
        }
        "emit-b2-shared-wire-input" => {
            let input = bridge_return_host::fixture::build_b2_shared_anchor_fixture();
            println!(
                "0x{}",
                hex::encode(bridge_return_guest::wire::encode_guest_input(&input))
            );
            Ok(())
        }
        "emit-config" => {
            let path = args.next();
            emit_config(path)
        }
        "emit-trust-base-hash" => {
            let path = args.next();
            emit_trust_base_hash(path)
        }
        "emit-settlement" => {
            let config = args.next();
            let recipient = args.next();
            let amount = args.next();
            emit_settlement(config, recipient, amount)
        }
        "emit-settlement-b2" => {
            let config = args.next();
            let recipient = args.next();
            let amount = args.next();
            emit_settlement_b2(config, recipient, amount)
        }
        "precheck-wire" => {
            let wire = args.next();
            precheck_wire(wire)
        }
        "sp1-execute" => {
            let elf = args.next().map(PathBuf::from);
            let wire = args.next();
            sp1_execute(elf, wire)
        }
        "sp1-mock-groth16" => {
            let elf = args.next().map(PathBuf::from);
            let wire = args.next();
            let proof = args.next().map(PathBuf::from);
            sp1_mock_groth16(elf, wire, proof)
        }
        "sp1-groth16" => {
            let elf = args.next().map(PathBuf::from);
            let wire = args.next();
            let proof = args.next().map(PathBuf::from);
            sp1_real_groth16(elf, wire, proof)
        }
        "sp1-vkey" => {
            let elf = args.next().map(PathBuf::from);
            sp1_vkey(elf)
        }
        "sp1-export" => {
            let elf = args.next().map(PathBuf::from);
            let proof = args.next().map(PathBuf::from);
            let out = args.next().map(PathBuf::from);
            sp1_export(elf, proof, out)
        }
        "sp1-proof-info" => {
            let proof = args.next().map(PathBuf::from);
            sp1_proof_info(proof)
        }
        _ => {
            usage();
            Ok(())
        }
    };

    if let Err(err) = result {
        eprintln!("{err}");
        process::exit(1);
    }
}

fn usage() {
    eprintln!("usage: bridge-return-host check-vectors [../bridge-vectors]");
    eprintln!("       bridge-return-host emit-b1-token-vector");
    eprintln!("       bridge-return-host emit-split-token-vector");
    eprintln!("       bridge-return-host emit-b1-wire-input");
    eprintln!("       bridge-return-host emit-split-wire-input");
    eprintln!("       bridge-return-host emit-b2-token-vector");
    eprintln!("       bridge-return-host emit-b2-wire-input");
    eprintln!("       bridge-return-host emit-b2-shared-wire-input");
    eprintln!("       bridge-return-host emit-config <config-in.json>                        # freeze: derive full config + config_hash");
    eprintln!("       bridge-return-host emit-trust-base-hash <trust-base.json>");
    eprintln!("       bridge-return-host emit-settlement <config.json> <recipient_hex> <amount>     # B=1 deploy-tailored");
    eprintln!("       bridge-return-host emit-settlement-b2 <config.json> <recipient_hex> <amount>  # B=2 deploy-tailored");
    eprintln!("       bridge-return-host precheck-wire <wire_hex>                            # S1 host precheck, no SP1");
    eprintln!("       bridge-return-host sp1-execute <guest.elf> <wire_hex>                 # --features sp1");
    eprintln!("       bridge-return-host sp1-mock-groth16 <guest.elf> <wire_hex> <proof.bin> # --features sp1");
    eprintln!("       bridge-return-host sp1-groth16 <guest.elf> <wire_hex> <proof.bin>      # --features sp1 (real CPU prove)");
    eprintln!("       bridge-return-host sp1-vkey <guest.elf>                                # --features sp1");
    eprintln!("       bridge-return-host sp1-export <guest.elf> <proof.bin> <bundle.json>    # --features sp1");
    eprintln!("       bridge-return-host sp1-proof-info <proof.bin>                         # --features sp1");
}

// Authoritative config derivation (Rust core): given the deployment inputs
// (source_chain_id, vault, asset [, reason_tag, lock_domain, nullifier_domain]),
// emit the full BridgeConfig with derived token_type/coin_id/config_hash/
// domain_tag. The config_hash equals the on-chain vault CONFIG_HASH (same
// keccak/ABI + DOMAIN_CONFIG), so this is the canonical "freeze the config" tool.
fn emit_config(path: Option<String>) -> bridge_return_host::Result<()> {
    use bridge_return_core::{coin_id, config_hash, domain_tag, token_type, BridgeConfig};

    let path = path.ok_or_else(|| {
        bridge_return_host::HostError::Check("missing config-in.json".to_string())
    })?;
    let json: serde_json::Value = serde_json::from_slice(&std::fs::read(&path)?)?;
    let err = |m: String| bridge_return_host::HostError::Check(m);

    let u64f = |k: &str| -> bridge_return_host::Result<u64> {
        json[k]
            .as_u64()
            .or_else(|| json[k].as_str().and_then(|s| s.parse().ok()))
            .ok_or_else(|| err(format!("missing u64 field {k}")))
    };
    let bytes = |k: &str| -> bridge_return_host::Result<Vec<u8>> {
        let s = json[k]
            .as_str()
            .ok_or_else(|| err(format!("missing hex field {k}")))?;
        Ok(hex::decode(s.strip_prefix("0x").unwrap_or(s))?)
    };
    let arr = |k: &str, n: usize| -> bridge_return_host::Result<Vec<u8>> {
        let v = bytes(k)?;
        if v.len() != n {
            return Err(err(format!("{k} length {}, expected {n}", v.len())));
        }
        Ok(v)
    };

    let source_chain_id = u64f("source_chain_id")?;
    let vault: [u8; 20] = arr("vault", 20)?.try_into().unwrap();
    let asset: [u8; 20] = arr("asset", 20)?.try_into().unwrap();
    let reason_tag = u64f("reason_tag")?;
    let lock_domain: [u8; 32] = arr("lock_domain", 32)?.try_into().unwrap();
    let nullifier_domain: [u8; 32] = arr("nullifier_domain", 32)?.try_into().unwrap();

    let chain_id_str = source_chain_id.to_string();
    let asset_evm_hex = hex::encode(asset); // lowercase, no 0x — matches the SDK derivation
    let token_type = token_type(&chain_id_str, &asset_evm_hex);
    let coin_id = coin_id(&chain_id_str, &asset_evm_hex);
    let cfg = BridgeConfig {
        source_chain_id,
        vault,
        asset,
        token_type,
        coin_id,
        reason_tag,
        lock_domain,
        nullifier_domain,
    };
    let h = |b: &[u8]| format!("0x{}", hex::encode(b));
    println!(
        "{}",
        serde_json::to_string_pretty(&serde_json::json!({
            "source_chain_id": source_chain_id,
            "vault": h(&vault),
            "asset": h(&asset),
            "token_type": h(&token_type),
            "coin_id": h(&coin_id),
            "reason_tag": reason_tag,
            "lock_domain": h(&lock_domain),
            "nullifier_domain": h(&nullifier_domain),
            "config_hash": h(&config_hash(&cfg)),
            "domain_tag": h(&domain_tag()),
        }))
        .expect("serialize config")
    );
    Ok(())
}

// Build a deployment-tailored settlement fixture and emit everything the on-chain
// fulfillBatch needs: the guest wire payload (to prove), the public values, the
// single return leaf + lock ref, and the lock-seed (amount, unicityTokenId,
// recipientCommitment, nonce) to call the vault's lock() so lockDigest[nonce]
// matches the circuit's lock ref. `vault` in the config must be the deployed
// vault address; the config's config_hash will then equal the vault CONFIG_HASH.
fn emit_settlement(
    config_path: Option<String>,
    recipient_hex: Option<String>,
    amount_str: Option<String>,
) -> bridge_return_host::Result<()> {
    use bridge_return_core::{coin_id, token_type, BridgeConfig};
    use unicity_token::crypto::hash::sha256;
    use unicity_token::transaction::Transaction;

    let err = |m: String| bridge_return_host::HostError::Check(m);
    let config_path = config_path.ok_or_else(|| err("missing config-in.json".to_string()))?;
    let recipient_hex = recipient_hex.ok_or_else(|| err("missing recipient_hex".to_string()))?;
    let amount: u64 = amount_str
        .ok_or_else(|| err("missing amount".to_string()))?
        .parse()
        .map_err(|_| err("amount must be u64".to_string()))?;
    let json: serde_json::Value = serde_json::from_slice(&std::fs::read(&config_path)?)?;

    let u64f = |k: &str| -> bridge_return_host::Result<u64> {
        json[k]
            .as_u64()
            .or_else(|| json[k].as_str().and_then(|s| s.parse().ok()))
            .ok_or_else(|| err(format!("missing u64 field {k}")))
    };
    let arr = |k: &str, n: usize| -> bridge_return_host::Result<Vec<u8>> {
        let s = json[k]
            .as_str()
            .ok_or_else(|| err(format!("missing hex field {k}")))?;
        let v = hex::decode(s.strip_prefix("0x").unwrap_or(s))?;
        if v.len() != n {
            return Err(err(format!("{k} length {}, expected {n}", v.len())));
        }
        Ok(v)
    };
    let from_hex = |s: &str, n: usize| -> bridge_return_host::Result<Vec<u8>> {
        let v = hex::decode(s.strip_prefix("0x").unwrap_or(s))?;
        if v.len() != n {
            return Err(err(format!("expected {n} bytes, got {}", v.len())));
        }
        Ok(v)
    };

    let source_chain_id = u64f("source_chain_id")?;
    let asset: [u8; 20] = arr("asset", 20)?.try_into().unwrap();
    let chain_id_str = source_chain_id.to_string();
    let asset_evm_hex = hex::encode(asset);
    let config = BridgeConfig {
        source_chain_id,
        vault: arr("vault", 20)?.try_into().unwrap(),
        asset,
        token_type: token_type(&chain_id_str, &asset_evm_hex),
        coin_id: coin_id(&chain_id_str, &asset_evm_hex),
        reason_tag: u64f("reason_tag")?,
        lock_domain: arr("lock_domain", 32)?.try_into().unwrap(),
        nullifier_domain: arr("nullifier_domain", 32)?.try_into().unwrap(),
    };
    let recipient: [u8; 20] = from_hex(&recipient_hex, 20)?.try_into().unwrap();

    let fx = bridge_return_host::fixture::build_settlement_fixture(config, recipient, amount, 0);
    let input = &fx.input;
    let pv = &input.public_values;
    let leaf = &input.return_leaves[0];
    let lref = &input.sorted_lock_refs[0];
    let mint = input.witness.bridge_burns[0].token.genesis().transaction();
    let unicity_token_id = mint.token_id().bytes();
    let recipient_commitment = sha256(&mint.recipient().to_cbor());
    let wire = bridge_return_guest::wire::encode_guest_input(input);
    let h = |b: &[u8]| format!("0x{}", hex::encode(b));

    println!(
        "{}",
        serde_json::to_string_pretty(&serde_json::json!({
            "guest_wire_input": h(&wire),
            "public_values": {
                "domain_tag": h(&pv.domain_tag),
                "config_hash": h(&pv.config_hash),
                "trust_base_hash": h(&pv.trust_base_hash),
                "spent_root_old": h(&pv.spent_root_old),
                "spent_root_new": h(&pv.spent_root_new),
                "return_root": h(&pv.return_root),
                "lock_ref_root": h(&pv.lock_ref_root),
                "batch_size": pv.batch_size,
                "total_amount": h(&pv.total_amount),
            },
            "leaf": {
                "nullifier": h(&leaf.nullifier),
                "recipient": h(&leaf.recipient),
                "amount": h(&leaf.amount),
                "fee_recipient": h(&leaf.fee_recipient),
                "fee_amount": h(&leaf.fee_amount),
                "deadline": leaf.deadline,
            },
            "lock_ref": { "nonce": lref.nonce, "digest": h(&lref.digest) },
            "lock_seed": {
                "amount": amount,
                "unicity_token_id": h(unicity_token_id),
                "recipient_commitment": h(recipient_commitment.data()),
                "nonce": lref.nonce,
            },
        }))
        .expect("serialize settlement")
    );
    Ok(())
}

// B=2 variant of emit-settlement: two burns (shared anchor) to the recipient at
// nonces 0 and 1, each for `amount`. Emits arrays of leaves / lock_refs /
// lock_seeds (lock seed 0 then 1, in nonce order) for the batched fulfillBatch.
fn emit_settlement_b2(
    config_path: Option<String>,
    recipient_hex: Option<String>,
    amount_str: Option<String>,
) -> bridge_return_host::Result<()> {
    use bridge_return_core::{coin_id, token_type, BridgeConfig};
    use unicity_token::crypto::hash::sha256;
    use unicity_token::transaction::Transaction;

    let err = |m: String| bridge_return_host::HostError::Check(m);
    let config_path = config_path.ok_or_else(|| err("missing config-in.json".to_string()))?;
    let recipient_hex = recipient_hex.ok_or_else(|| err("missing recipient_hex".to_string()))?;
    let amount: u64 = amount_str
        .ok_or_else(|| err("missing amount".to_string()))?
        .parse()
        .map_err(|_| err("amount must be u64".to_string()))?;
    let json: serde_json::Value = serde_json::from_slice(&std::fs::read(&config_path)?)?;
    let u64f = |k: &str| -> bridge_return_host::Result<u64> {
        json[k]
            .as_u64()
            .or_else(|| json[k].as_str().and_then(|s| s.parse().ok()))
            .ok_or_else(|| err(format!("missing u64 field {k}")))
    };
    let arr = |k: &str, n: usize| -> bridge_return_host::Result<Vec<u8>> {
        let s = json[k]
            .as_str()
            .ok_or_else(|| err(format!("missing hex field {k}")))?;
        let v = hex::decode(s.strip_prefix("0x").unwrap_or(s))?;
        if v.len() != n {
            return Err(err(format!("{k} length {}, expected {n}", v.len())));
        }
        Ok(v)
    };
    let source_chain_id = u64f("source_chain_id")?;
    let asset: [u8; 20] = arr("asset", 20)?.try_into().unwrap();
    let chain_id_str = source_chain_id.to_string();
    let asset_evm_hex = hex::encode(asset);
    let config = BridgeConfig {
        source_chain_id,
        vault: arr("vault", 20)?.try_into().unwrap(),
        asset,
        token_type: token_type(&chain_id_str, &asset_evm_hex),
        coin_id: coin_id(&chain_id_str, &asset_evm_hex),
        reason_tag: u64f("reason_tag")?,
        lock_domain: arr("lock_domain", 32)?.try_into().unwrap(),
        nullifier_domain: arr("nullifier_domain", 32)?.try_into().unwrap(),
    };
    let recipient: [u8; 20] =
        hex::decode(recipient_hex.strip_prefix("0x").unwrap_or(&recipient_hex))?
            .try_into()
            .map_err(|_| err("recipient must be 20 bytes".to_string()))?;

    let input = bridge_return_host::fixture::build_settlement_fixture_b2(
        config,
        [recipient, recipient],
        [amount, amount],
        [0, 1],
    );
    let pv = &input.public_values;
    let h = |b: &[u8]| format!("0x{}", hex::encode(b));
    let mut leaves = Vec::new();
    let mut lock_refs = Vec::new();
    let mut lock_seeds = Vec::new();
    for i in 0..input.return_leaves.len() {
        let leaf = &input.return_leaves[i];
        let lref = &input.sorted_lock_refs[i];
        let mint = input.witness.bridge_burns[i].token.genesis().transaction();
        leaves.push(serde_json::json!({
            "nullifier": h(&leaf.nullifier),
            "recipient": h(&leaf.recipient),
            "amount": h(&leaf.amount),
            "fee_recipient": h(&leaf.fee_recipient),
            "fee_amount": h(&leaf.fee_amount),
            "deadline": leaf.deadline,
        }));
        lock_refs.push(serde_json::json!({ "nonce": lref.nonce, "digest": h(&lref.digest) }));
        lock_seeds.push(serde_json::json!({
            "amount": amount,
            "unicity_token_id": h(mint.token_id().bytes()),
            "recipient_commitment": h(sha256(&mint.recipient().to_cbor()).data()),
            "nonce": lref.nonce,
        }));
    }
    println!(
        "{}",
        serde_json::to_string_pretty(&serde_json::json!({
            "guest_wire_input": h(&bridge_return_guest::wire::encode_guest_input(&input)),
            "public_values": {
                "domain_tag": h(&pv.domain_tag),
                "config_hash": h(&pv.config_hash),
                "trust_base_hash": h(&pv.trust_base_hash),
                "spent_root_old": h(&pv.spent_root_old),
                "spent_root_new": h(&pv.spent_root_new),
                "return_root": h(&pv.return_root),
                "lock_ref_root": h(&pv.lock_ref_root),
                "batch_size": pv.batch_size,
                "total_amount": h(&pv.total_amount),
            },
            "leaves": leaves,
            "lock_refs": lock_refs,
            "lock_seeds": lock_seeds,
        }))
        .expect("serialize settlement")
    );
    Ok(())
}

// The trust-base hash the vault's `setTrustBaseAllowed` must allow — the SDK
// `canonical_hash` of a trust-base JSON (SHA-256/CBOR, 00 §1).
fn emit_trust_base_hash(path: Option<String>) -> bridge_return_host::Result<()> {
    let path = path.ok_or_else(|| {
        bridge_return_host::HostError::Check("missing trust-base.json".to_string())
    })?;
    let trust_base =
        unicity_token::api::bft::RootTrustBase::from_json(&std::fs::read_to_string(&path)?)
            .map_err(|err| bridge_return_host::HostError::Check(format!("trust base: {err}")))?;
    let hash = bridge_return_sdk_ext::trust::canonical_hash(&trust_base);
    println!("0x{}", hex::encode(hash));
    Ok(())
}

fn precheck_wire(wire: Option<String>) -> bridge_return_host::Result<()> {
    let wire =
        wire.ok_or_else(|| bridge_return_host::HostError::Check("missing wire_hex".to_string()))?;
    let bytes = hex::decode(wire.strip_prefix("0x").unwrap_or(&wire))?;
    let report = bridge_return_host::s1::precheck_wire(&bytes)?;
    println!(
        "{}",
        serde_json::to_string_pretty(&serde_json::json!({
            "batch_size": report.batch_size,
            "total_amount": format!("0x{}", hex::encode(report.total_amount)),
            "public_values_abi": format!("0x{}", hex::encode(&report.public_values_abi)),
            "public_values_digest": format!("0x{}", hex::encode(report.public_values_digest)),
            "wire_input_len": report.wire_input.len(),
        }))
        .expect("serialize precheck report")
    );
    Ok(())
}

#[cfg(feature = "sp1")]
fn require_arg<T>(value: Option<T>, name: &str) -> bridge_return_host::Result<T> {
    value.ok_or_else(|| bridge_return_host::HostError::Check(format!("missing {name}")))
}

#[cfg(feature = "sp1")]
fn decode_hex_arg(value: String, name: &str) -> bridge_return_host::Result<Vec<u8>> {
    hex::decode(value.strip_prefix("0x").unwrap_or(&value))
        .map_err(|err| bridge_return_host::HostError::Check(format!("{name}: {err}")))
}

#[cfg(feature = "sp1")]
fn sp1_execute(elf: Option<PathBuf>, wire: Option<String>) -> bridge_return_host::Result<()> {
    let elf = require_arg(elf, "guest.elf")?;
    let wire = decode_hex_arg(require_arg(wire, "wire_hex")?, "wire_hex")?;
    let execution = bridge_return_host::sp1::execute_elf(&elf, wire)?;
    println!(
        "{}",
        serde_json::to_string_pretty(&serde_json::json!({
            "public_values": format!("0x{}", hex::encode(&execution.public_values)),
            "expected_public_values": format!("0x{}", hex::encode(&execution.expected_public_values)),
            "cycles": execution.cycles,
        }))
        .expect("serialize SP1 execution")
    );
    Ok(())
}

#[cfg(not(feature = "sp1"))]
fn sp1_execute(_elf: Option<PathBuf>, _wire: Option<String>) -> bridge_return_host::Result<()> {
    Err(bridge_return_host::HostError::Check(
        "rebuild bridge-return-host with --features sp1".to_string(),
    ))
}

#[cfg(feature = "sp1")]
fn sp1_mock_groth16(
    elf: Option<PathBuf>,
    wire: Option<String>,
    proof: Option<PathBuf>,
) -> bridge_return_host::Result<()> {
    let elf = require_arg(elf, "guest.elf")?;
    let wire = decode_hex_arg(require_arg(wire, "wire_hex")?, "wire_hex")?;
    let proof = require_arg(proof, "proof.bin")?;
    let info = bridge_return_host::sp1::mock_groth16(&elf, wire, &proof)?;
    print_proof_info(&info);
    Ok(())
}

#[cfg(not(feature = "sp1"))]
fn sp1_mock_groth16(
    _elf: Option<PathBuf>,
    _wire: Option<String>,
    _proof: Option<PathBuf>,
) -> bridge_return_host::Result<()> {
    Err(bridge_return_host::HostError::Check(
        "rebuild bridge-return-host with --features sp1".to_string(),
    ))
}

#[cfg(feature = "sp1")]
fn sp1_real_groth16(
    elf: Option<PathBuf>,
    wire: Option<String>,
    proof: Option<PathBuf>,
) -> bridge_return_host::Result<()> {
    let elf = require_arg(elf, "guest.elf")?;
    let wire = decode_hex_arg(require_arg(wire, "wire_hex")?, "wire_hex")?;
    let proof = require_arg(proof, "proof.bin")?;
    let info = bridge_return_host::sp1::real_groth16(&elf, wire, &proof)?;
    print_proof_info(&info);
    Ok(())
}

#[cfg(not(feature = "sp1"))]
fn sp1_real_groth16(
    _elf: Option<PathBuf>,
    _wire: Option<String>,
    _proof: Option<PathBuf>,
) -> bridge_return_host::Result<()> {
    Err(bridge_return_host::HostError::Check(
        "rebuild bridge-return-host with --features sp1".to_string(),
    ))
}

#[cfg(feature = "sp1")]
fn sp1_vkey(elf: Option<PathBuf>) -> bridge_return_host::Result<()> {
    let elf = require_arg(elf, "guest.elf")?;
    let vkey = bridge_return_host::sp1::program_vkey(&elf)?;
    println!(
        "{}",
        serde_json::to_string_pretty(&serde_json::json!({
            "vkey": vkey,
            "circuit_version": bridge_return_host::sp1::circuit_version(),
        }))
        .expect("serialize SP1 vkey")
    );
    Ok(())
}

#[cfg(not(feature = "sp1"))]
fn sp1_vkey(_elf: Option<PathBuf>) -> bridge_return_host::Result<()> {
    Err(bridge_return_host::HostError::Check(
        "rebuild bridge-return-host with --features sp1".to_string(),
    ))
}

#[cfg(feature = "sp1")]
fn sp1_export(
    elf: Option<PathBuf>,
    proof: Option<PathBuf>,
    out: Option<PathBuf>,
) -> bridge_return_host::Result<()> {
    let elf = require_arg(elf, "guest.elf")?;
    let proof = require_arg(proof, "proof.bin")?;
    let out = require_arg(out, "bundle.json")?;
    let info = bridge_return_host::sp1::export_onchain(&elf, &proof)?;
    let digest = info
        .public_values
        .len()
        .checked_sub(32)
        .map(|start| format!("0x{}", hex::encode(&info.public_values[start..])));
    let bundle = serde_json::json!({
        "proto": bridge_return_core::BRIDGE_PROTO_VERSION,
        "sp1_version": info.sp1_version,
        "circuit_version": bridge_return_host::sp1::circuit_version(),
        "proof_mode": info.proof_mode,
        "vkey": info.vkey_hash,
        "public_values": format!("0x{}", hex::encode(&info.public_values)),
        "public_values_digest": digest,
        "proof_bytes": format!("0x{}", hex::encode(&info.proof_bytes)),
        "proof_bytes_len": info.proof_bytes.len(),
    });
    let mut serialized = serde_json::to_string_pretty(&bundle).expect("serialize SP1 bundle");
    serialized.push('\n');
    std::fs::write(&out, serialized)
        .map_err(|err| bridge_return_host::HostError::Check(format!("write {out:?}: {err}")))?;
    eprintln!("wrote on-chain bundle to {}", out.display());
    print_proof_info(&info);
    Ok(())
}

#[cfg(not(feature = "sp1"))]
fn sp1_export(
    _elf: Option<PathBuf>,
    _proof: Option<PathBuf>,
    _out: Option<PathBuf>,
) -> bridge_return_host::Result<()> {
    Err(bridge_return_host::HostError::Check(
        "rebuild bridge-return-host with --features sp1".to_string(),
    ))
}

#[cfg(feature = "sp1")]
fn sp1_proof_info(proof: Option<PathBuf>) -> bridge_return_host::Result<()> {
    let proof = require_arg(proof, "proof.bin")?;
    let info = bridge_return_host::sp1::proof_info_from_file(&proof)?;
    print_proof_info(&info);
    Ok(())
}

#[cfg(not(feature = "sp1"))]
fn sp1_proof_info(_proof: Option<PathBuf>) -> bridge_return_host::Result<()> {
    Err(bridge_return_host::HostError::Check(
        "rebuild bridge-return-host with --features sp1".to_string(),
    ))
}

#[cfg(feature = "sp1")]
fn print_proof_info(info: &bridge_return_host::sp1::Sp1ProofInfo) {
    println!(
        "{}",
        serde_json::to_string_pretty(&serde_json::json!({
            "proof_mode": info.proof_mode,
            "sp1_version": info.sp1_version,
            "vkey": info.vkey_hash,
            "public_values": format!("0x{}", hex::encode(&info.public_values)),
            "proof_bytes": format!("0x{}", hex::encode(&info.proof_bytes)),
            "proof_bytes_len": info.proof_bytes.len(),
        }))
        .expect("serialize SP1 proof info")
    );
}
