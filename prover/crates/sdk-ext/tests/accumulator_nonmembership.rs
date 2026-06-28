//! Regression for a known E2 accumulator bug.
//!
//! `non_membership_witness` produces witnesses that fail their own
//! `verify_non_member` for trees with **>= 2 elements** (~21% of random
//! 2-element trees). Root cause: the tree is a *path-compressed* radix tree whose
//! root is the top branch node (at the first bifurcation depth) and therefore
//! does NOT bind the prefix bits above it. A key that diverges from the present
//! keys *within that compressed prefix* has no representable witness (the step
//! depth would have to sit above the branch, which `rebuild_absent_root`
//! correctly rejects) — so the witness builder and verifier are inconsistent.
//!
//! Impact: non-membership is only sound for <=1-element trees, so the relation is
//! correct for a single burn and for the *second* burn in a batch (witness vs a
//! 1-element tree) — M3 (B=1) and M4 (B=2) are unaffected — but it breaks the
//! **third+ burn in a batch (B>=3)** and **multi-batch continuity** (a later
//! batch's burn is witnessed against the >=2-element accumulator of prior burns).
//!
//! Fix: rework the accumulator into a proper sparse Merkle tree whose root is
//! framed from depth 0 (empty-subtree-hash compression), so every prefix bit is
//! bound and divergence at any depth is provable. That changes root hashes (the
//! M3/M4 test vaults' spentRoot would differ — acceptable, they are throwaway).
//!
//! `#[ignore]`d so CI stays green; remove the attribute once the accumulator is
//! reworked and this must pass.
use bridge_return_sdk_ext::accumulator::{
    ordered_insert_witnesses, verify_non_member, NullifierTree,
};

// Deterministic, well-spread pseudo-random 32-byte keys (not crypto).
fn key(mut v: u64) -> [u8; 32] {
    let mut b = [0u8; 32];
    for byte in b.iter_mut() {
        v = v
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        *byte = (v >> 33) as u8;
    }
    b
}

#[test]
#[ignore = "known bug: non-membership unsound for >=2-element trees; needs SMT rework"]
fn non_membership_self_verifies_for_two_element_trees() {
    let mut failures = 0;
    let trials = 500u64;
    for s in 0..trials {
        let mut t = NullifierTree::new();
        t.insert(key(s * 4)).unwrap();
        t.insert(key(s * 4 + 1)).unwrap();
        let absent = key(s * 4 + 2);
        let w = t.non_membership_witness(&absent).expect("witness");
        if !verify_non_member(&t.root(), &absent, &w) {
            failures += 1;
        }
    }
    assert_eq!(
        failures, 0,
        "{failures}/{trials} non-membership witnesses failed self-verify"
    );
}

#[test]
#[ignore = "known bug: ordered_insert_witnesses unsound past the 2nd key; needs SMT rework"]
fn ordered_insert_witnesses_all_verify_for_b3() {
    // The 3rd key is witnessed against a 2-element tree — the broken case.
    let keys = [key(1), key(2), key(3)];
    let tree = NullifierTree::new();
    let (witnesses, _root) = ordered_insert_witnesses(&tree, &keys).unwrap();
    // Re-derive each intermediate root and check the witness verifies against it.
    let mut running = NullifierTree::new();
    for (k, w) in keys.iter().zip(&witnesses) {
        assert!(
            verify_non_member(&running.root(), k, w),
            "witness failed against the intermediate root it was built on"
        );
        running.insert(*k).unwrap();
    }
}
