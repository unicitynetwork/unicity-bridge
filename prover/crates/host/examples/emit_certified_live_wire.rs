//! Emit the guest wire payload for a live, aggregator-*certified* burned token,
//! so it can be fed to `sp1-execute` and run through the relation in the zkVM.
//!
//!   cargo run -p bridge-return-host --example emit_certified_live_wire -- \
//!     ../bridge-plugin-tron-usdt/demo/.bridge-back-state.json ../bft-trustbase.testnet2.json
use std::{env, fs};

use bridge_return_core::{u256_from_u64, BridgeConfig, ReturnLeaf};
use bridge_return_guest::wire;
use bridge_return_host::s1::build_certified_guest_input;
use bridge_return_sdk_ext::bridge::TRON_USDT_LOCK_JUSTIFICATION_TAG;
use serde_json::Value;
use unicity_token::api::bft::RootTrustBase;
use unicity_token::transaction::Token;

fn unhex(s: &str) -> Vec<u8> {
    hex::decode(s.strip_prefix("0x").unwrap_or(s)).expect("valid hex")
}
fn n(v: &Value, k: &str) -> Vec<u8> {
    unhex(v.get(k).and_then(Value::as_str).expect("hex field"))
}
fn u64f(v: &Value, k: &str) -> u64 {
    let x = &v[k];
    x.as_u64()
        .or_else(|| x.as_str().and_then(|s| s.parse().ok()))
        .expect("u64")
}

fn main() {
    let blob = env::args()
        .nth(1)
        .unwrap_or_else(|| "../bridge-plugin-tron-usdt/demo/.bridge-back-state.json".to_string());
    let tb = env::args()
        .nth(2)
        .unwrap_or_else(|| "../bft-trustbase.testnet2.json".to_string());
    let json: Value = serde_json::from_str(&fs::read_to_string(blob).expect("blob")).unwrap();
    let trust_base =
        RootTrustBase::from_json(&fs::read_to_string(tb).expect("trust base")).unwrap();

    let pc = &json["proverConfig"];
    let config = BridgeConfig {
        source_chain_id: u64f(pc, "sourceChainId"),
        vault: n(pc, "vault").try_into().unwrap(),
        asset: n(pc, "asset").try_into().unwrap(),
        token_type: n(pc, "tokenType").try_into().unwrap(),
        coin_id: n(pc, "coinId").try_into().unwrap(),
        reason_tag: u64f(pc, "reasonTag"),
        lock_domain: n(pc, "lockDomain").try_into().unwrap(),
        nullifier_domain: n(pc, "nullifierDomain").try_into().unwrap(),
    };
    let rl = &json["returnLeaf"];
    let leaf = ReturnLeaf {
        nullifier: n(rl, "nullifier").try_into().unwrap(),
        recipient: n(rl, "recipient").try_into().unwrap(),
        amount: u256_from_u64(u64f(rl, "amount")),
        fee_recipient: n(rl, "feeRecipient").try_into().unwrap(),
        fee_amount: u256_from_u64(u64f(rl, "feeAmount")),
        deadline: u64f(rl, "deadline"),
    };
    let token = Token::from_cbor(&unhex(json["burnedTokenCbor"].as_str().unwrap())).unwrap();

    let input = build_certified_guest_input(
        config,
        token,
        trust_base,
        TRON_USDT_LOCK_JUSTIFICATION_TAG,
        leaf,
    )
    .expect("build certified guest input");
    println!("0x{}", hex::encode(wire::encode_guest_input(&input)));
}
