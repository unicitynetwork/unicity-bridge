//! S1 verifies a REAL aggregator-certified bridge-back token against the REAL
//! testnet2 trust base — full quorum + chain-linkage + owner-auth verification,
//! not a synthetic fixture. The sample is a frozen `npm run e2e:back` blob
//! (`bridge-plugin-tron-usdt/demo/.bridge-back-state.json`, gitignored live) and
//! the matching trust base; together they verify deterministically forever,
//! independent of current network state.
use std::fs;
use std::path::Path;

use bridge_return_core::BridgeConfig;
use bridge_return_host::s1::verify_certified_burn;
use bridge_return_sdk_ext::bridge::TRON_USDT_LOCK_JUSTIFICATION_TAG;
use serde_json::Value;
use unicity_token::api::bft::RootTrustBase;
use unicity_token::transaction::Token;

fn data(name: &str) -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/data")
        .join(name);
    fs::read_to_string(path).expect("read fixture")
}

fn unhex(s: &str) -> Vec<u8> {
    hex::decode(s.strip_prefix("0x").unwrap_or(s)).expect("valid hex")
}

fn n(v: &Value, k: &str) -> Vec<u8> {
    unhex(v.get(k).and_then(Value::as_str).expect("hex field"))
}

fn config_from(pc: &Value) -> BridgeConfig {
    let u64f = |k: &str| -> u64 {
        let v = &pc[k];
        v.as_u64()
            .or_else(|| v.as_str().and_then(|s| s.parse().ok()))
            .expect("u64 field")
    };
    BridgeConfig {
        source_chain_id: u64f("sourceChainId"),
        vault: n(pc, "vault").try_into().unwrap(),
        asset: n(pc, "asset").try_into().unwrap(),
        token_type: n(pc, "tokenType").try_into().unwrap(),
        coin_id: n(pc, "coinId").try_into().unwrap(),
        reason_tag: u64f("reasonTag"),
        lock_domain: n(pc, "lockDomain").try_into().unwrap(),
        nullifier_domain: n(pc, "nullifierDomain").try_into().unwrap(),
    }
}

#[test]
fn s1_verifies_live_certified_token() {
    let json: Value = serde_json::from_str(&data("bridge-back-live-sample.json")).unwrap();
    let trust_base = RootTrustBase::from_json(&data("trustbase.testnet2.json")).unwrap();
    let config = config_from(&json["proverConfig"]);
    let token = Token::from_cbor(&unhex(json["burnedTokenCbor"].as_str().unwrap())).unwrap();

    let refs = verify_certified_burn(
        &token,
        &trust_base,
        &config,
        TRON_USDT_LOCK_JUSTIFICATION_TAG,
    )
    .expect("live token verifies against the testnet2 trust base");

    // v1: exactly one source lock, matching the values the TS wallet recorded.
    let nonce = &json["lock"]["nonce"];
    let expected_nonce = nonce
        .as_u64()
        .or_else(|| nonce.as_str().and_then(|s| s.parse().ok()))
        .expect("nonce");
    assert_eq!(refs.len(), 1);
    assert_eq!(refs[0].nonce, expected_nonce);
    assert_eq!(refs[0].digest.to_vec(), n(&json["lock"], "lockDigest"));
}

#[test]
fn s1_rejects_unsatisfiable_trust_base() {
    // The quorum is actually enforced: a trust base whose threshold cannot be met
    // by the certificate's signatures rejects the very same live token.
    // (A single-byte token flip is *not* a reliable negative — it can land in one
    // redundant signature and still meet quorum, which is correct behavior.)
    let json: Value = serde_json::from_str(&data("bridge-back-live-sample.json")).unwrap();
    let config = config_from(&json["proverConfig"]);
    let token = Token::from_cbor(&unhex(json["burnedTokenCbor"].as_str().unwrap())).unwrap();

    let mut tb_json: Value = serde_json::from_str(&data("trustbase.testnet2.json")).unwrap();
    tb_json["quorumThreshold"] = Value::from(1000u64); // exceeds total stake

    let rejected = match RootTrustBase::from_json(&tb_json.to_string()) {
        Err(_) => true, // rejected at parse/validate
        Ok(trust_base) => verify_certified_burn(
            &token,
            &trust_base,
            &config,
            TRON_USDT_LOCK_JUSTIFICATION_TAG,
        )
        .is_err(),
    };
    assert!(rejected, "unsatisfiable quorum must reject the token");
}
