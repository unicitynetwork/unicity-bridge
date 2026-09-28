//! Guards the frozen Nile-USDT deployment configs
//! (`deployments/nile/nile-usdt.json` = v1, `nile-usdt-v2.json` = v2): the Rust
//! core must re-derive the same token_type / coin_id / config_hash, and
//! config_hash must equal the value the deployed vault committed on-chain. If
//! any stack drifts, this fails.
use std::fs;
use std::path::Path;

use bridge_return_core::{coin_id, config_hash, token_type, BridgeConfig};
use serde_json::Value;

// The deployed vaults' CONFIG_HASH (UnicityBridgeVault on Nile, reason_tag
// 39048), each read back on-chain after deployment.
// v1: TTKKLyhnRRQ7XV5vsRarV8xWWEvF9225mY (2026-07-03, vkey 0x00c34ae0…)
const ONCHAIN_CONFIG_HASH_V1: &str =
    "0x7f376b16b3bff3455f375e7cf30b9d29d2a14332912f0ffb69d78e1b31d5193f";
// v2: TBKJ84417jdxo6j92TxQuYpZdRZGaeZVrv (2026-09-21, vkey 0x0039a542…)
const ONCHAIN_CONFIG_HASH_V2: &str =
    "0xfa77a13a6fb24658fa75377b0eef7cf3e92f8caede9ea127afe21be7a036cb1b";

fn unhex(s: &str) -> Vec<u8> {
    hex::decode(s.strip_prefix("0x").unwrap_or(s)).expect("hex")
}
fn h(b: &[u8]) -> String {
    format!("0x{}", hex::encode(b))
}

#[test]
fn frozen_nile_config_v1_is_consistent() {
    check_frozen("nile-usdt.json", ONCHAIN_CONFIG_HASH_V1);
}

#[test]
fn frozen_nile_config_v2_is_consistent() {
    check_frozen("nile-usdt-v2.json", ONCHAIN_CONFIG_HASH_V2);
}

#[test]
fn v2_differs_from_v1_only_by_vault_and_vkey() {
    let v1 = read_doc("nile-usdt.json");
    let v2 = read_doc("nile-usdt-v2.json");
    for k in ["asset", "coin_id", "token_type", "reason_tag", "lock_domain", "nullifier_domain", "source_chain_id"] {
        assert_eq!(v1["config"][k], v2["config"][k], "{k} must not change on redeploy");
    }
    assert_ne!(v1["config"]["vault"], v2["config"]["vault"]);
    assert_ne!(v1["config"]["config_hash"], v2["config"]["config_hash"]);
    assert_ne!(v1["deployment"]["vkey"], v2["deployment"]["vkey"]);
    assert_eq!(v2["proto"], 2);
}

fn read_doc(file: &str) -> Value {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../../deployments/nile")
        .join(file);
    serde_json::from_slice(&fs::read(path).expect("read frozen config")).unwrap()
}

fn check_frozen(file: &str, onchain_config_hash: &str) {
    let doc = read_doc(file);
    let c = &doc["config"];

    let cfg = BridgeConfig {
        source_chain_id: c["source_chain_id"].as_u64().unwrap(),
        vault: unhex(c["vault"].as_str().unwrap()).try_into().unwrap(),
        asset: unhex(c["asset"].as_str().unwrap()).try_into().unwrap(),
        token_type: unhex(c["token_type"].as_str().unwrap()).try_into().unwrap(),
        coin_id: unhex(c["coin_id"].as_str().unwrap()).try_into().unwrap(),
        reason_tag: c["reason_tag"].as_u64().unwrap(),
        lock_domain: unhex(c["lock_domain"].as_str().unwrap())
            .try_into()
            .unwrap(),
        nullifier_domain: unhex(c["nullifier_domain"].as_str().unwrap())
            .try_into()
            .unwrap(),
    };

    // token_type / coin_id re-derive from (chain_id_str, asset_evm_hex).
    let chain_id_str = doc["chain_id_str"].as_str().unwrap();
    let asset_evm_hex = doc["asset_evm_hex"].as_str().unwrap();
    assert_eq!(
        h(&token_type(chain_id_str, asset_evm_hex)),
        c["token_type"].as_str().unwrap()
    );
    assert_eq!(
        h(&coin_id(chain_id_str, asset_evm_hex)),
        c["coin_id"].as_str().unwrap()
    );

    // config_hash re-derives AND equals both the recorded value and the on-chain
    // vault CONFIG_HASH — the cross-stack freeze (Rust prover == Solidity vault).
    let derived = h(&config_hash(&cfg));
    assert_eq!(
        derived,
        c["config_hash"].as_str().unwrap(),
        "config_hash drift"
    );
    assert_eq!(
        derived, onchain_config_hash,
        "config_hash != deployed vault CONFIG_HASH"
    );
}
