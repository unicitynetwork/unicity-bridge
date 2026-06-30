//! Guards the frozen Nile-USDT deployment config
//! (`bridge-vectors/deployment/nile-usdt.json`): the Rust core must re-derive the
//! same token_type / coin_id / config_hash, and config_hash must equal the value
//! the deployed vault committed on-chain. If any stack drifts, this fails.
use std::fs;
use std::path::Path;

use bridge_return_core::{coin_id, config_hash, token_type, BridgeConfig};
use serde_json::Value;

// The deployed vault's CONFIG_HASH (UnicityBridgeVault on Nile,
// TD89z57Xksziu3uk24qfjT27bJmeWLgjtk, reason_tag 39048), verified on-chain.
const ONCHAIN_CONFIG_HASH: &str =
    "0x594546ae7e114b8c5674b793234a45f72eca7727aa25b0f605200ebf3cae4b93";

fn unhex(s: &str) -> Vec<u8> {
    hex::decode(s.strip_prefix("0x").unwrap_or(s)).expect("hex")
}
fn h(b: &[u8]) -> String {
    format!("0x{}", hex::encode(b))
}

#[test]
fn frozen_nile_config_is_consistent() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../../bridge-vectors/deployment/nile-usdt.json");
    let doc: Value = serde_json::from_slice(&fs::read(path).expect("read frozen config")).unwrap();
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
        derived, ONCHAIN_CONFIG_HASH,
        "config_hash != deployed vault CONFIG_HASH"
    );
}
