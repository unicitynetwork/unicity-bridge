//! Live S1 anchored fetch (ZK_BACK3 §2.1): take a real aggregator token, re-fetch
//! each transition's inclusion proof against the CURRENT root, and confirm they
//! all resolve to one shared root certified by a single `UC*` (anchored mode).
//! Run from prover/ with the repo .env exported and --features http:
//!   set -a; . ../.env; set +a
//!   cargo run -p bridge-return-host --features http --example s1_anchored_live
#[cfg(not(feature = "http"))]
fn main() {
    eprintln!("rebuild with --features http");
}

#[cfg(feature = "http")]
fn main() {
    use std::fs;
    use std::path::Path;

    use bridge_return_host::s1::aggregator;
    use serde_json::Value;
    use unicity_token::api::bft::RootTrustBase;
    use unicity_token::api::StateId;
    use unicity_token::client::AggregatorClient;
    use unicity_token::transaction::{Token, Transaction};

    let base = Path::new(env!("CARGO_MANIFEST_DIR"));
    let blob = fs::read_to_string(base.join("tests/data/bridge-back-live-sample.json"))
        .expect("read live sample");
    let json: Value = serde_json::from_str(&blob).unwrap();
    let token = Token::from_cbor(&hex::decode(json["burnedTokenCbor"].as_str().unwrap()).unwrap())
        .expect("decode token");
    let trust_base = RootTrustBase::from_json(
        &fs::read_to_string(base.join("tests/data/trustbase.testnet2.json")).unwrap(),
    )
    .unwrap();
    let client = aggregator::client_from_env().expect("UNICITY_GATEWAY set");

    // Show that re-fetching each transition now resolves to one current root.
    let mint = token.genesis().transaction();
    let mut sids = vec![StateId::derive(
        mint.lock_script(),
        mint.source_state_hash(),
    )];
    for t in token.transactions() {
        let tx = t.transaction();
        sids.push(StateId::derive(tx.lock_script(), tx.source_state_hash()));
    }
    println!(
        "re-fetching {} transition inclusion proof(s) ...",
        sids.len()
    );
    for (i, sid) in sids.iter().enumerate() {
        let p = client.get_inclusion_proof(sid).expect("inclusion proof");
        let ir = &p.unicity_certificate.input_record;
        println!(
            "  transition {i}: round {} root 0x{}",
            ir.round_number,
            hex::encode(&ir.hash)
        );
    }

    match aggregator::fetch_anchored_token(&client, &token, &trust_base) {
        Ok((anchored, uc)) => println!(
            "\n✅ anchored: {} transition(s) verify against ONE shared root 0x{} (UC* round {}) under the testnet2 trust base.",
            anchored.transactions().len() + 1,
            hex::encode(&uc.input_record.hash),
            uc.input_record.round_number,
        ),
        Err(e) => println!(
            "\n⚠️  anchored re-fetch did NOT converge to one root: {e:?}\n\
             The gateway's `get_inclusion_proof.v2` takes only a stateId and serves the\n\
             *current* root (rounds above differ + advance), with no target-root/snapshot\n\
             param, so a multi-transition token's proofs can't be pinned to one shared\n\
             root via this API. Anchored batching (§11/§2.1) needs an aggregator endpoint\n\
             that returns inclusion against a specified root. Until then, live tokens are\n\
             proven in CERTIFIED mode (each transition's own cert) — see s1_live.rs.",
        ),
    }
}
