//! S1 aggregator HTTP client (ZK_BACK3 §10.1). Compiled only with `--features
//! http`. The live test is `#[ignore]` — it needs the gateway env + network.
#![cfg(feature = "http")]

use bridge_return_host::s1::aggregator;

#[test]
fn client_from_env_errors_without_gateway() {
    // `cargo test` does not load `.env`, so UNICITY_GATEWAY is unset here and the
    // builder must surface a clear error rather than panic.
    if std::env::var("UNICITY_GATEWAY").is_ok() {
        return; // someone exported it; skip rather than assert the opposite
    }
    assert!(aggregator::client_from_env().is_err());
}

/// Live fetch against the configured gateway. Run with the repo-root `.env`
/// exported, e.g.:
///   set -a; . ../../../.env; set +a
///   cargo test -p bridge-return-host --features http -- --ignored live_fetch
#[test]
#[ignore = "live network: needs UNICITY_GATEWAY (+ API key) and a live aggregator"]
fn live_fetch_terminal_proof() {
    use std::fs;
    use std::path::Path;
    use unicity_token::transaction::Token;

    let client = aggregator::client_from_env().expect("UNICITY_GATEWAY must be set");
    let blob = fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/bridge-back-live-sample.json"),
    )
    .expect("read sample");
    let json: serde_json::Value = serde_json::from_str(&blob).unwrap();
    let cbor = hex::decode(json["burnedTokenCbor"].as_str().unwrap()).unwrap();
    let token = Token::from_cbor(&cbor).unwrap();

    let proof = aggregator::fetch_terminal_inclusion_proof(&client, &token)
        .expect("fetch terminal inclusion proof");
    assert!(proof.inclusion_certificate.is_some());
}
