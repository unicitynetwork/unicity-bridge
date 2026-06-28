//! Correctness suite for the sparse-Merkle nullifier accumulator.
//!
//! Originally a regression for a real bug: the previous path-compressed radix
//! tree's root did not bind prefix bits above the top branch, so non-membership
//! witnesses failed their own verifier for ~21% of >=2-element trees (breaking
//! B>=3 batches and multi-batch continuity). The accumulator is now a depth-256
//! sparse Merkle tree framed from depth 0, and these tests assert the properties
//! that were broken: every absent key has a self-verifying witness at every tree
//! size, and `insert(root, key, witness)` agrees with rebuilding the tree.
use bridge_return_sdk_ext::accumulator::{
    insert, ordered_insert_witnesses, verify_non_member, NullifierTree, EMPTY_TREE_ROOT,
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
fn non_membership_self_verifies_for_two_element_trees() {
    let mut failures = 0;
    let trials = 300u64;
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
fn ordered_insert_witnesses_all_verify_for_b3() {
    // The 3rd key is witnessed against a 2-element tree — the previously broken case.
    let keys = [key(1), key(2), key(3)];
    let tree = NullifierTree::new();
    let (witnesses, _root) = ordered_insert_witnesses(&tree, &keys).unwrap();
    let mut running = NullifierTree::new();
    for (k, w) in keys.iter().zip(&witnesses) {
        assert!(
            verify_non_member(&running.root(), k, w),
            "witness failed against the intermediate root it was built on"
        );
        running.insert(*k).unwrap();
    }
}

#[test]
fn witness_and_insert_agree_across_tree_sizes() {
    // For trees of 0..=6 present keys, every absent key must have a self-verifying
    // witness, and insert-via-witness must equal the rebuilt tree's root.
    for size in 0u64..=6 {
        for seed in 0u64..40 {
            let mut tree = NullifierTree::new();
            for i in 0..size {
                tree.insert(key(seed * 16 + i)).unwrap();
            }
            let root = tree.root();
            let absent = key(seed * 16 + 100);
            let w = tree
                .non_membership_witness(&absent)
                .expect("absent witness");
            assert!(
                verify_non_member(&root, &absent, &w),
                "size {size} seed {seed}: non-membership self-verify failed"
            );
            // insert via witness == rebuild from scratch
            let via_witness = insert(&root, &absent, &w).expect("insert");
            let mut rebuilt = tree.clone();
            rebuilt.insert(absent).unwrap();
            assert_eq!(
                via_witness,
                rebuilt.root(),
                "size {size} seed {seed}: insert-via-witness != rebuilt root"
            );
        }
    }
}

#[test]
fn present_key_has_no_witness() {
    let mut tree = NullifierTree::new();
    tree.insert(key(7)).unwrap();
    tree.insert(key(8)).unwrap();
    assert!(tree.non_membership_witness(&key(7)).is_none());
    assert!(
        tree.insert(key(7)).is_err(),
        "duplicate insert must be rejected"
    );
}

#[test]
fn empty_tree_first_insert() {
    let tree = NullifierTree::new();
    assert_eq!(tree.root(), EMPTY_TREE_ROOT);
    let k = key(42);
    let w = tree.non_membership_witness(&k).expect("witness");
    assert!(verify_non_member(&EMPTY_TREE_ROOT, &k, &w));
    let after = insert(&EMPTY_TREE_ROOT, &k, &w).expect("insert");
    let mut t2 = NullifierTree::new();
    t2.insert(k).unwrap();
    assert_eq!(after, t2.root());
    assert_ne!(after, EMPTY_TREE_ROOT);
}
