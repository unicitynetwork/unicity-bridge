//! Cross-stack closure check for the live bridge-back e2e.
//!
//! Decodes the burned-token blob produced by the TS wallet demo
//! (`bridge-plugin-tron-usdt/demo/bridge-back-e2e.ts` → `.bridge-back-state.json`)
//! and verifies the Rust prover derives the SAME values the TS side recorded:
//!   - `bridge_lock_obligation` (E3) reproduces the TS `lockDigest` + nonce (00 §3);
//!   - `config_hash` matches (00 §2);
//!   - the nullifier derived from the decoded terminal burn matches (00 §5).
//!
//! This proves the two stacks are byte-compatible on a *real* aggregator-certified
//! token (not a synthetic fixture). Run:
//!   cargo run -p bridge-return-host --example cross_check_live -- \
//!     ../bridge-plugin-tron-usdt/demo/.bridge-back-state.json
use std::{env, fs, process};

use bridge_return_core::{burn_transition_id, config_hash, nullifier, BridgeConfig};
use bridge_return_sdk_ext::bridge::{
    bridge_lock_obligation, BridgeConfig as SdkBridgeConfig, TRON_USDT_LOCK_JUSTIFICATION_TAG,
};
use serde_json::Value;
use unicity_token::api::bft::RootTrustBase;
use unicity_token::api::StateId;
use unicity_token::payment::PaymentAssetCollection;
use unicity_token::transaction::{Token, Transaction};

fn unhex(s: &str) -> Vec<u8> {
    hex::decode(s.strip_prefix("0x").unwrap_or(s)).expect("valid hex")
}
fn n32(s: &str) -> [u8; 32] {
    unhex(s).try_into().expect("32 bytes")
}
fn n20(s: &str) -> [u8; 20] {
    unhex(s).try_into().expect("20 bytes")
}
fn s(v: &Value, k: &str) -> String {
    v.get(k)
        .and_then(Value::as_str)
        .unwrap_or_else(|| panic!("missing field {k}"))
        .to_string()
}

fn check(label: &str, got: &[u8], want: &[u8]) -> bool {
    let ok = got == want;
    println!(
        "  [{}] {label}\n      rust: 0x{}\n      ts  : 0x{}",
        if ok { "PASS" } else { "FAIL" },
        hex::encode(got),
        hex::encode(want),
    );
    ok
}

fn main() {
    let path = env::args()
        .nth(1)
        .unwrap_or_else(|| "../bridge-plugin-tron-usdt/demo/.bridge-back-state.json".to_string());
    let json: Value = serde_json::from_str(&fs::read_to_string(&path).expect("read state file"))
        .expect("parse state file");

    // Trust base for full quorum verification (arg 2, env, or repo-root default).
    let trustbase_path = env::args()
        .nth(2)
        .or_else(|| env::var("UNICITY_TRUSTBASE").ok())
        .unwrap_or_else(|| "../bft-trustbase.testnet2.json".to_string());
    let trust_base =
        RootTrustBase::from_json(&fs::read_to_string(&trustbase_path).expect("read trust base"))
            .expect("parse trust base");

    let pc = json.get("proverConfig").expect("proverConfig");
    let lock = json.get("lock").expect("lock");

    let config = BridgeConfig {
        source_chain_id: s(pc, "sourceChainId").parse().unwrap(),
        vault: n20(&s(pc, "vault")),
        asset: n20(&s(pc, "asset")),
        token_type: n32(&s(pc, "tokenType")),
        coin_id: n32(&s(pc, "coinId")),
        reason_tag: s(pc, "reasonTag").parse().unwrap(),
        lock_domain: n32(&s(pc, "lockDomain")),
        nullifier_domain: n32(&s(pc, "nullifierDomain")),
    };
    let sdk_config = SdkBridgeConfig {
        source_chain_id: config.source_chain_id,
        vault: config.vault,
        asset: config.asset,
        token_type: config.token_type,
        coin_id: config.coin_id,
    };

    let token =
        Token::from_cbor(&unhex(&s(&json, "burnedTokenCbor"))).expect("decode burned token");
    println!(
        "Decoded TS burned token: {} transition(s).\n",
        token.transactions().len()
    );

    let mut ok = true;

    // Full cryptographic verification of the live token against the testnet2
    // trust base (quorum + chain linkage + owner auth via certified mode), then
    // the verified bridge-lock obligation. This is the S1 entry path for real
    // tokens — stronger than the structural-only derivations below.
    match bridge_return_host::s1::verify_certified_burn(
        &token,
        &trust_base,
        &config,
        TRON_USDT_LOCK_JUSTIFICATION_TAG,
    ) {
        Ok(refs) => {
            let digest_ok = refs.len() == 1 && refs[0].digest == n32(&s(lock, "lockDigest"));
            ok &= digest_ok;
            println!(
                "  [{}] live token VERIFIED vs testnet2 trust base (quorum {} of {} nodes), certified mode\n      verified lockDigest: 0x{}",
                if digest_ok { "PASS" } else { "FAIL" },
                trust_base.quorum_threshold,
                trust_base.root_nodes.len(),
                refs.first().map(|r| hex::encode(r.digest)).unwrap_or_default(),
            );
        }
        Err(err) => {
            ok = false;
            println!("  [FAIL] live token verification: {err}");
        }
    }

    // E3: structural backing obligation (lockDigest + nonce).
    let obligation = bridge_lock_obligation(
        token.genesis(),
        TRON_USDT_LOCK_JUSTIFICATION_TAG,
        &sdk_config,
        PaymentAssetCollection::from_cbor_bytes,
    )
    .expect("bridge_lock_obligation");
    ok &= obligation.nonce == s(lock, "nonce").parse::<u64>().unwrap();
    println!(
        "  [{}] obligation.nonce  rust={} ts={}",
        if obligation.nonce == s(lock, "nonce").parse::<u64>().unwrap() {
            "PASS"
        } else {
            "FAIL"
        },
        obligation.nonce,
        s(lock, "nonce"),
    );
    ok &= check(
        "lockDigest (00 §3)",
        &obligation.digest,
        &n32(&s(lock, "lockDigest")),
    );

    // config_hash (00 §2).
    let cfg_hash = config_hash(&config);
    ok &= check(
        "configHash (00 §2)",
        &cfg_hash,
        &n32(&s(&json, "configHash")),
    );

    // nullifier from the decoded terminal burn (00 §5).
    let burn = token
        .transactions()
        .last()
        .expect("terminal burn")
        .transaction();
    let state_id = StateId::derive(burn.lock_script(), burn.source_state_hash());
    let tx_hash: [u8; 32] = burn.calculate_transaction_hash().data().try_into().unwrap();
    ok &= check(
        "burnStateId",
        state_id.bytes(),
        &n32(&s(&json, "burnStateId")),
    );
    ok &= check("burnTxHash", &tx_hash, &n32(&s(&json, "burnTxHash")));
    let burn_id = burn_transition_id(state_id.bytes(), &tx_hash);
    ok &= check(
        "burnTransitionId (00 §5)",
        &burn_id,
        &n32(&s(&json, "burnTransitionId")),
    );
    ok &= check(
        "nullifier (00 §5)",
        &nullifier(&cfg_hash, &burn_id),
        &n32(&s(&json, "nullifier")),
    );

    println!();
    if ok {
        println!("✔ cross-stack closure: Rust prover agrees with the TS wallet on the live token.");
    } else {
        eprintln!("✘ MISMATCH — the stacks disagree on at least one value.");
        process::exit(1);
    }
}
