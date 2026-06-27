//! B=2 batch relation tests (M4). The guest relation is batch-shaped; these
//! assert a two-burn batch executes and that the order-coupled invariants
//! (accumulator witness order, sorted-unique lock refs, batch size, value
//! conservation) are enforced — the most bug-prone part of the return relation
//! (ZK_BACK3 §7.4).
use bridge_return_guest::execute;
use bridge_return_host::fixture::{build_b2_direct_bridge_fixture, build_b2_shared_anchor_fixture};

#[test]
fn guest_executes_b2_batch() {
    let input = build_b2_direct_bridge_fixture();
    assert_eq!(input.public_values.batch_size, 2);
    assert_eq!(input.return_leaves.len(), 2);
    assert_eq!(input.sorted_lock_refs.len(), 2);
    // the two nullifiers are distinct (independent tokens)
    assert_ne!(
        input.return_leaves[0].nullifier,
        input.return_leaves[1].nullifier
    );
    assert_eq!(execute(&input), Ok(input.public_values));
}

#[test]
fn b2_rejects_swapped_accumulator_witnesses() {
    // witness[1] is valid only against the root AFTER inserting nullifier[0];
    // swapping them must fail (the order-coupling in ZK_BACK3 §6/§7.4).
    let mut input = build_b2_direct_bridge_fixture();
    input.witness.accumulator_witnesses.swap(0, 1);
    assert!(execute(&input).is_err());
}

#[test]
fn b2_rejects_unsorted_lock_refs() {
    let mut input = build_b2_direct_bridge_fixture();
    input.sorted_lock_refs.swap(0, 1); // now nonce-descending
    assert!(execute(&input).is_err());
}

#[test]
fn b2_rejects_wrong_batch_size() {
    let mut input = build_b2_direct_bridge_fixture();
    input.public_values.batch_size = 1;
    assert!(execute(&input).is_err());
}

#[test]
fn b2_rejects_swapped_leaves() {
    // Swapping the settlement leaves (without re-deriving the public roots)
    // breaks both the return-root commitment and the per-burn nullifier binding.
    let mut input = build_b2_direct_bridge_fixture();
    input.return_leaves.swap(0, 1);
    assert!(execute(&input).is_err());
}

#[test]
fn b2_rejects_dropped_burn_witness() {
    let mut input = build_b2_direct_bridge_fixture();
    input.witness.bridge_burns.pop();
    assert!(execute(&input).is_err());
}

#[test]
fn b2_shared_anchor_executes() {
    // The §11 one-quorum-check shape: all four transitions of the two tokens are
    // leaves of a single SMT, so one shared UC* anchors the whole batch.
    let input = build_b2_shared_anchor_fixture();
    assert_eq!(input.public_values.batch_size, 2);
    assert_eq!(execute(&input), Ok(input.public_values));
}

#[test]
fn b2_shared_anchor_uses_one_anchor() {
    // Both burns reference the byte-identical anchor certificate — one BFT-quorum
    // certificate covers the batch (vs distinct per-token anchors otherwise).
    let shared = build_b2_shared_anchor_fixture();
    let a0 = shared.witness.bridge_burns[0].anchor_certificate.to_cbor();
    let a1 = shared.witness.bridge_burns[1].anchor_certificate.to_cbor();
    assert_eq!(a0, a1, "shared-anchor batch must carry one UC*");

    // Sanity: the per-anchor batch instead carries two distinct anchors.
    let per_burn = build_b2_direct_bridge_fixture();
    assert_ne!(
        per_burn.witness.bridge_burns[0]
            .anchor_certificate
            .to_cbor(),
        per_burn.witness.bridge_burns[1]
            .anchor_certificate
            .to_cbor(),
    );
}

#[test]
fn b2_shared_anchor_matches_per_anchor_public_values() {
    // Anchoring shape is a witness detail; the committed public values (the x the
    // vault verifies) are identical to the per-anchor batch.
    assert_eq!(
        build_b2_shared_anchor_fixture().public_values,
        build_b2_direct_bridge_fixture().public_values,
    );
}
