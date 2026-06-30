//! S1 host precheck tests (ZK_BACK3 §10.1). The precheck mirrors the SP1 guest
//! relation natively and round-trips the wire encoding, so a witness package that
//! prechecks clean is safe to hand to the prover. These assert it accepts the
//! valid B=1 / split / B=2 fixtures and rejects a tampered package.
use bridge_return_guest::wire;
use bridge_return_host::fixture::{
    build_b1_direct_bridge_fixture, build_b2_direct_bridge_fixture, build_split_bridge_fixture,
};
use bridge_return_host::s1::{
    build_certified_guest_input_batch, precheck_wire, CertifiedBurnInput, WitnessPackage,
};

#[test]
fn precheck_accepts_b1() {
    let input = build_b1_direct_bridge_fixture().input;
    let expected = input.public_values;
    let wire_input = wire::encode_guest_input(&input);
    let report = WitnessPackage::new(input).precheck().expect("b1 precheck");
    assert_eq!(report.batch_size, 1);
    assert_eq!(report.public_values, expected);
    // The package reports the exact bytes the prover should be handed.
    assert_eq!(report.wire_input, wire_input);
    assert_eq!(report.total_amount, expected.total_amount);
}

#[test]
fn precheck_accepts_split() {
    let input = build_split_bridge_fixture().input;
    let expected = input.public_values;
    let report = WitnessPackage::new(input)
        .precheck()
        .expect("split precheck");
    assert_eq!(report.batch_size, 1);
    assert_eq!(report.public_values, expected);
}

#[test]
fn precheck_accepts_b2() {
    let input = build_b2_direct_bridge_fixture();
    let expected = input.public_values;
    let report = WitnessPackage::new(input).precheck().expect("b2 precheck");
    assert_eq!(report.batch_size, 2);
    assert_eq!(report.public_values, expected);
}

#[test]
fn precheck_wire_matches_in_memory() {
    let input = build_b2_direct_bridge_fixture();
    let expected = input.public_values;
    let wire_input = wire::encode_guest_input(&input);
    let report = precheck_wire(&wire_input).expect("wire precheck");
    assert_eq!(report.batch_size, 2);
    assert_eq!(report.public_values, expected);
}

#[test]
fn precheck_rejects_tampered_public_values() {
    // Flip a byte of the committed total: the relation recomputes it from the
    // leaves, so execute() rejects before any wire round-trip.
    let mut input = build_b1_direct_bridge_fixture().input;
    input.public_values.total_amount[0] ^= 0x01;
    assert!(WitnessPackage::new(input).precheck().is_err());
}

#[test]
fn precheck_rejects_truncated_wire() {
    let input = build_b1_direct_bridge_fixture().input;
    let mut wire_input = wire::encode_guest_input(&input);
    wire_input.truncate(wire_input.len() - 1);
    assert!(precheck_wire(&wire_input).is_err());
}

#[test]
fn certified_batch_builder_accepts_b2() {
    let anchored = build_b2_direct_bridge_fixture();
    let burns = anchored
        .witness
        .bridge_burns
        .iter()
        .zip(anchored.return_leaves.iter())
        .map(|(burn, leaf)| CertifiedBurnInput {
            token: burn.token.clone(),
            trust_base: burn.trust_base.clone(),
            lock_justification_tag: burn.lock_justification_tag,
            leaf: *leaf,
        })
        .collect();

    let certified = build_certified_guest_input_batch(anchored.config, burns)
        .expect("certified B=2 guest input");
    let report = WitnessPackage::new(certified)
        .precheck()
        .expect("certified B=2 precheck");
    assert_eq!(report.batch_size, 2);
    assert_eq!(
        report.public_values.return_root,
        anchored.public_values.return_root
    );
    assert_eq!(
        report.public_values.total_amount,
        anchored.public_values.total_amount
    );
}
