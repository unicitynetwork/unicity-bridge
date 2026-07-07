//! Reference generator for the Unicity bridge cross-stack conformance vectors.
//! `BRIDGE_PROTO_VERSION = 1`. See ../../interop.md.
//!
//! Implemented (unambiguous hash / ABI / CBOR derivations): `config`, `lock`,
//! `reason`, `nullifier`, `public`. Stubbed (need the SDK SMT / token relation):
//! `accumulator`, `token` — those directories carry a README explaining the
//! dependency and are filled in at M2 once the Rust SDK extensions (E1–E3) land.
//!
//! Run: `cargo run` (in this directory). Writes one coherent example per group
//! into ../<group>/*.json, all threaded through a single example deployment.

mod abi;
mod accumulator;
mod cbor;
mod derive;
mod hash;
mod json;

use derive::{Config, LockRecord, LockRef, PublicValues, ReasonParams, ReturnLeaf};
use json::{hex, Obj};
use std::fs;
use std::path::Path;

fn main() {
    hash::self_test();

    let base = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .to_path_buf();
    let version = fs::read_to_string(base.join("VERSION")).unwrap_or_default();
    println!(
        "bridge-vectors generator — BRIDGE_PROTO_VERSION={}",
        version.trim()
    );

    // ---- one example deployment, threaded through every group ----------------
    let chain_id_str = "728126428"; // Tron mainnet (0x2b6653dc)
    let asset_evm_hex = "a614f803b6fd780986a42c78ec9c7f77e6ded13c"; // USDT-TRON (EVM form)
    let cfg = Config {
        source_chain_id: 728126428,
        vault: h20("00000000000000000000000000000000000000a1"),
        asset: h20(asset_evm_hex),
        token_type: derive::token_type(chain_id_str, asset_evm_hex),
        coin_id: derive::coin_id(chain_id_str, asset_evm_hex),
        reason_tag: 39048,
        lock_domain: hash::sha256(b"UNICITY_BR_LOCK"),
        nullifier_domain: hash::sha256(b"UNICITY_BR_NUL"),
    };
    let cfg_hash = derive::config_hash(&cfg);

    // ---- config --------------------------------------------------------------
    write(&base, "config", "config-00.json", Obj::new()
        .str("description", "config -> tokenType/coinId + configHash (00 §2); configHash = keccak256(abi.encode(...))")
        .obj("in", Obj::new()
            .num("source_chain_id", cfg.source_chain_id)
            .str("chain_id_str", chain_id_str)
            .str("asset_evm_hex", asset_evm_hex)
            .hex("vault", &cfg.vault)
            .hex("asset", &cfg.asset)
            .num("reason_tag", cfg.reason_tag)
            .hex("lock_domain", &cfg.lock_domain)
            .hex("nullifier_domain", &cfg.nullifier_domain))
        .obj("out", Obj::new()
            .hex("token_type", &cfg.token_type)
            .hex("coin_id", &cfg.coin_id)
            .hex("config_hash", &cfg_hash))
        .render());

    // ---- lock ----------------------------------------------------------------
    let recipient_cbor = vec![0xa2u8, 0x01, 0x58, 0x21, 0x03]; // placeholder predicate CBOR
    let lock = LockRecord {
        nonce: 7,
        amount: u256(1_000_000), // 1 USDT (6 decimals)
        unicity_token_id: hash::sha256(b"example-unicity-token-id"),
        recipient_commitment: derive::recipient_commitment(&recipient_cbor),
    };
    let lock_digest = derive::lock_digest(&cfg, &lock);
    write(&base, "lock", "lock-00.json", Obj::new()
        .str("description", "LockRecord -> recipientCommitment + lockDigest (00 §3); lockDigest = keccak256(abi.encode(...))")
        .obj("in", Obj::new()
            .num("nonce", lock.nonce)
            .hex("amount", &lock.amount)
            .hex("unicity_token_id", &lock.unicity_token_id)
            .hex("recipient_cbor", &recipient_cbor)
            .hex("config_hash", &cfg_hash))
        .obj("out", Obj::new()
            .hex("recipient_commitment", &lock.recipient_commitment)
            .hex("lock_digest", &lock_digest))
        .render());

    // ---- reason --------------------------------------------------------------
    let reason = ReasonParams {
        version: 1,
        recipient: h20("00000000000000000000000000000000000000b2"),
        amount: u256(1_000_000),
        fee_recipient: h20("00000000000000000000000000000000000000c3"),
        fee_amount: u256(2_000),
        deadline: 1_900_000_000,
    };
    let reason_cbor = derive::reason_cbor(&cfg, &reason);
    let reason_hash = hash::sha256(&reason_cbor); // = H(reasonBytes), bound by BurnPredicate(H(reasonBytes)) (00 §4)
    write(&base, "reason", "reason-00.json", Obj::new()
        .str("description", "BridgeBackReason fields -> canonical CBOR (reasonBytes, in burn aux data) + reasonHash bound by BurnPredicate(H(reasonBytes)) (00 §4); PROVISIONAL, confirm vs SDK at M0")
        .obj("in", Obj::new()
            .num("version", reason.version)
            .num("source_chain_id", cfg.source_chain_id)
            .hex("vault", &cfg.vault)
            .hex("asset", &cfg.asset)
            .hex("token_type", &cfg.token_type)
            .hex("coin_id", &cfg.coin_id)
            .hex("recipient", &reason.recipient)
            .hex("amount", &reason.amount)
            .hex("fee_recipient", &reason.fee_recipient)
            .hex("fee_amount", &reason.fee_amount)
            .num("deadline", reason.deadline)
            .num("reason_tag", cfg.reason_tag))
        .obj("out", Obj::new()
            .hex("reason_cbor", &reason_cbor)
            .hex("reason_hash", &reason_hash))
        .render());

    // ---- nullifier -----------------------------------------------------------
    let state_id = hash::sha256(b"example-burn-state-id");
    let tx_hash = hash::sha256(b"example-burn-tx-hash");
    let bt_id = derive::burn_transition_id(&state_id, &tx_hash);
    let nullifier_a = derive::nullifier(&cfg_hash, &bt_id);
    write(&base, "nullifier", "nullifier-00.json", Obj::new()
        .str("description", "(stateId, txHash, configHash) -> burnTransitionId, nullifier (00 §5); H = SHA-256 over CBOR array")
        .obj("in", Obj::new()
            .hex("state_id", &state_id)
            .hex("tx_hash", &tx_hash)
            .hex("config_hash", &cfg_hash))
        .obj("out", Obj::new()
            .hex("burn_transition_id", &bt_id)
            .hex("nullifier", &nullifier_a))
        .render());

    // ---- public (two leaves, two lock refs) ----------------------------------
    let bt_b =
        derive::burn_transition_id(&hash::sha256(b"burn-state-b"), &hash::sha256(b"burn-tx-b"));
    let nullifier_b = derive::nullifier(&cfg_hash, &bt_b);
    let leaves = vec![
        ReturnLeaf {
            nullifier: nullifier_a,
            recipient: reason.recipient,
            amount: u256(400_000),
            fee_recipient: reason.fee_recipient,
            fee_amount: u256(1_000),
            deadline: reason.deadline,
        },
        ReturnLeaf {
            nullifier: nullifier_b,
            recipient: h20("00000000000000000000000000000000000000d4"),
            amount: u256(600_000),
            fee_recipient: h20("0000000000000000000000000000000000000000"),
            fee_amount: u256(0),
            deadline: 0,
        },
    ];
    let refs = vec![
        // intentionally unsorted to exercise the sort-by-nonce rule
        LockRef {
            nonce: 7,
            digest: lock_digest,
        },
        LockRef {
            nonce: 3,
            digest: hash::sha256(b"another-lock-digest"),
        },
    ];
    let return_root = derive::return_root(&leaves);
    let lock_ref_root = derive::lock_ref_root(&refs);
    let pv = PublicValues {
        domain_tag: derive::domain_tag(),
        config_hash: cfg_hash,
        trust_base_hash: hash::sha256(b"example-root-trust-base"),
        spent_root_old: [0u8; 32], // EMPTY_TREE_ROOT placeholder (00 §6)
        spent_root_new: hash::sha256(b"example-new-spent-root"),
        return_root,
        lock_ref_root,
        batch_size: 2,
        total_amount: u256(1_000_000),
    };
    let pv_abi = derive::public_values_abi(&pv);

    let leaf_json = |l: &ReturnLeaf| {
        Obj::new()
            .hex("nullifier", &l.nullifier)
            .hex("recipient", &l.recipient)
            .hex("amount", &l.amount)
            .hex("fee_recipient", &l.fee_recipient)
            .hex("fee_amount", &l.fee_amount)
            .num("deadline", l.deadline)
            .render_inline()
    };
    let ref_json = |r: &LockRef| {
        Obj::new()
            .num("nonce", r.nonce)
            .hex("digest", &r.digest)
            .render_inline()
    };
    let leaves_arr = format!("[{}, {}]", leaf_json(&leaves[0]), leaf_json(&leaves[1]));
    let refs_arr = format!("[{}, {}]", ref_json(&refs[0]), ref_json(&refs[1]));

    write(&base, "public", "public-00.json", Obj::new()
        .str("description", "ReturnLeaf[]/SourceLockRef[] -> returnRoot/lockRefRoot (keccak over fixed-width words) + PublicValues ABI (00 §7)")
        .obj("in", Obj::new()
            .raw("leaves", leaves_arr)
            .raw("lock_refs", refs_arr)
            .hex("trust_base_hash", &pv.trust_base_hash)
            .hex("spent_root_old", &pv.spent_root_old)
            .hex("spent_root_new", &pv.spent_root_new)
            .num("batch_size", pv.batch_size as u64)
            .hex("total_amount", &pv.total_amount))
        .obj("out", Obj::new()
            .hex("domain_tag", &pv.domain_tag)
            .hex("return_root", &return_root)
            .hex("lock_ref_root", &lock_ref_root)
            .hex("config_hash", &cfg_hash)
            .hex("public_values_abi", &pv_abi))
        .render());

    // ---- accumulator --------------------------------------------------------
    let nullifiers = vec![nullifier_a, nullifier_b, hash::sha256(b"burn-nullifier-c")];
    let mut tree = accumulator::Tree::new();
    let mut roots = Vec::new();
    let mut witnesses = Vec::new();
    for n in &nullifiers {
        let against_root = tree.root();
        let witness = tree.witness(n).expect("canonical witness");
        tree.insert(*n);
        let new_root = tree.root();
        roots.push(new_root);
        witnesses.push(witness_json(n, &against_root, &new_root, &witness));
    }
    let nullifiers_arr = format!(
        "[{}]",
        nullifiers
            .iter()
            .map(|n| format!("\"0x{}\"", hex(n)))
            .collect::<Vec<_>>()
            .join(", ")
    );
    let roots_arr = format!(
        "[{}]",
        roots
            .iter()
            .map(|r| format!("\"0x{}\"", hex(r)))
            .collect::<Vec<_>>()
            .join(", ")
    );
    write(&base, "accumulator", "accumulator-00.json", Obj::new()
        .str("description", "Ordered nullifier accumulator inserts (00 §6); each witness is valid against the intermediate root after prior inserts")
        .obj("in", Obj::new()
            .hex("empty_root", &accumulator::EMPTY_TREE_ROOT)
            .raw("nullifiers", nullifiers_arr))
        .obj("out", Obj::new()
            .raw("roots", roots_arr)
            .raw("witnesses", format!("[{}]", witnesses.join(", "))))
        .render());

    println!("done — implemented groups written; token/ is still a stub (see its README).");
}

// ---- helpers ---------------------------------------------------------------

fn u256(n: u128) -> [u8; 32] {
    let mut w = [0u8; 32];
    w[16..].copy_from_slice(&n.to_be_bytes());
    w
}

fn h20(s: &str) -> [u8; 20] {
    let v = unhex(s);
    let mut a = [0u8; 20];
    a.copy_from_slice(&v);
    a
}

fn unhex(s: &str) -> Vec<u8> {
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).expect("valid hex"))
        .collect()
}

fn write(base: &Path, group: &str, file: &str, content: String) {
    let dir = base.join(group);
    fs::create_dir_all(&dir).expect("create group dir");
    fs::write(dir.join(file), content).expect("write vector");
    println!("  wrote {group}/{file}");
}

fn witness_json(
    nullifier: &[u8; 32],
    against_root: &[u8; 32],
    new_root: &[u8; 32],
    witness: &accumulator::Witness,
) -> String {
    let terminal = match witness.terminal {
        accumulator::Terminal::Empty => Obj::new().str("kind", "empty").render_inline(),
        accumulator::Terminal::Occupied(key) => Obj::new()
            .str("kind", "occupied")
            .hex("key", &key)
            .render_inline(),
    };
    let steps = witness
        .steps
        .iter()
        .map(|s| {
            Obj::new()
                .num("depth", s.depth as u64)
                .hex("sibling_hash", &s.sibling_hash)
                .render_inline()
        })
        .collect::<Vec<_>>()
        .join(", ");
    Obj::new()
        .hex("nullifier", nullifier)
        .hex("against_root", against_root)
        .hex("new_root", new_root)
        .raw("terminal", terminal)
        .raw("steps", format!("[{steps}]"))
        .render_inline()
}
