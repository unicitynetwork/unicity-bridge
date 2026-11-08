//! S2 accumulator-builder: rebuild from the event log, verify against the chain,
//! and produce a valid next-batch transition — the operational multi-batch path.

use bridge_return_host::s2::{next_batch, parse_settled_log, rebuild, SettledBatch};
use bridge_return_sdk_ext::accumulator::{
    insert as accumulator_insert, ordered_insert_witnesses, verify_non_member, NullifierTree,
    EMPTY_TREE_ROOT,
};

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

/// Build the `SettledBatch` event records for a sequence of per-batch nullifier
/// groups, exactly as the chain would emit them (chained roots).
fn settled_log(groups: &[Vec<[u8; 32]>]) -> Vec<SettledBatch> {
    let mut tree = NullifierTree::new();
    let mut old = EMPTY_TREE_ROOT;
    let mut out = Vec::new();
    for g in groups {
        let (_, new) = ordered_insert_witnesses(&tree, g).unwrap();
        out.push(SettledBatch {
            nullifiers: g.clone(),
            spent_root_old: old,
            spent_root_new: new,
            txid: None,
        });
        for n in g {
            tree.insert(*n).unwrap();
        }
        old = new;
    }
    out
}

#[test]
fn rebuilds_multi_batch_and_matches_chain() {
    // Batches of sizes 2, 1, 3 — the size-3 batch and the cross-batch state both
    // witness against >=2-element trees (the case the old radix tree got wrong).
    let groups = vec![
        vec![key(1), key(2)],
        vec![key(3)],
        vec![key(4), key(5), key(6)],
    ];
    let log = settled_log(&groups);
    let acc = rebuild(&log).expect("rebuild");

    assert_eq!(acc.spent_count, 6);
    // Reconstructed root == the last batch's on-chain spentRootNew.
    assert_eq!(acc.spent_root, log.last().unwrap().spent_root_new);
}

#[test]
fn next_batch_witnesses_verify_against_the_rebuilt_root() {
    let log = settled_log(&[vec![key(1), key(2)], vec![key(3)]]);
    let acc = rebuild(&log).expect("rebuild");

    let new = [key(10), key(11)];
    let nb = next_batch(&acc, &new).expect("next batch");
    assert_eq!(nb.spent_root_old, acc.spent_root);

    // Each witness verifies against the prior root, and folding the inserts
    // reproduces spent_root_new — exactly what fulfillBatch will check on-chain.
    let mut running = nb.spent_root_old;
    for (n, w) in new.iter().zip(&nb.witnesses) {
        assert!(verify_non_member(&running, n, w));
        running = accumulator_insert(&running, n, w).expect("insert");
    }
    assert_eq!(running, nb.spent_root_new);
}

#[test]
fn rebuild_rejects_a_tampered_root() {
    let mut log = settled_log(&[vec![key(1), key(2)], vec![key(3)]]);
    log[1].spent_root_new = key(999); // corrupt the recorded transition
    assert!(rebuild(&log).is_err());
}

#[test]
fn rebuild_links_batches_by_root_regardless_of_log_order() {
    let ordered = settled_log(&[vec![key(1)], vec![key(2)], vec![key(3)]]);
    let mut shuffled = ordered.clone();
    shuffled.swap(0, 2);
    let acc = rebuild(&shuffled).expect("rebuild");
    assert_eq!(acc.spent_root, rebuild(&ordered).unwrap().spent_root);
    assert_eq!(acc.spent_count, 3);
}

#[test]
fn rebuild_takes_a_batch_the_chain_log_lacks_from_another_source() {
    let all = settled_log(&[vec![key(1), key(2)], vec![key(3)]]);
    let chain_log_missing_the_first = vec![all[1].clone()];
    assert!(rebuild(&chain_log_missing_the_first).is_err());

    let mut union = chain_log_missing_the_first;
    union.push(all[0].clone());
    let acc = rebuild(&union).expect("rebuild");
    assert_eq!(acc.spent_root, all[1].spent_root_new);
    assert_eq!(acc.spent_count, 3);
}

#[test]
fn rebuild_applies_an_identical_duplicate_once() {
    let all = settled_log(&[vec![key(1)], vec![key(2)]]);
    let with_duplicate = vec![all[0].clone(), all[1].clone(), all[1].clone()];
    let acc = rebuild(&with_duplicate).expect("rebuild");
    assert_eq!(acc.spent_count, 2);
    assert_eq!(acc.spent_root, all[1].spent_root_new);
}

#[test]
fn rebuild_names_the_transaction_that_spent_each_nullifier() {
    let mut log = settled_log(&[vec![key(1), key(2)], vec![key(3)]]);
    log[0].txid = Some("0xaa".to_string());
    let acc = rebuild(&log).expect("rebuild");
    assert_eq!(acc.txids.get(&key(1)).map(String::as_str), Some("0xaa"));
    assert_eq!(acc.txids.get(&key(2)).map(String::as_str), Some("0xaa"));
    assert_eq!(acc.txids.get(&key(3)), None);
}

#[test]
fn a_copy_without_a_txid_is_the_same_batch_and_the_named_txid_is_kept() {
    let chain = {
        let mut log = settled_log(&[vec![key(1)], vec![key(2)]]);
        log[1].txid = Some("0xbb".to_string());
        log
    };
    let journal_copy = settled_log(&[vec![key(1)], vec![key(2)]]).remove(1);
    for batches in [
        vec![chain[0].clone(), journal_copy.clone(), chain[1].clone()],
        vec![chain[0].clone(), chain[1].clone(), journal_copy],
    ] {
        let acc = rebuild(&batches).expect("rebuild");
        assert_eq!(acc.spent_count, 2);
        assert_eq!(acc.txids.get(&key(2)).map(String::as_str), Some("0xbb"));
        assert_eq!(acc.batches, chain);
    }
}

#[test]
fn rebuild_keeps_each_transition_it_replayed_in_chain_order() {
    let log = settled_log(&[vec![key(1), key(2)], vec![key(3)]]);
    let acc = rebuild(&[log[1].clone(), log[0].clone()]).expect("rebuild");
    assert_eq!(acc.batches, log);
    let other = settled_log(&[vec![key(1), key(2), key(4)]]).remove(0);
    assert!(acc.batches.iter().any(|b| b.same_transition(&log[0])));
    assert!(!acc.batches.iter().any(|b| b.same_transition(&other)));
}

#[test]
fn the_settlement_log_names_each_batch_txid_when_the_watcher_gives_one() {
    let root = |b: u8| format!("0x{}", hex::encode([b; 32]));
    let json = serde_json::json!({
        "batches": [
            { "nullifiers": [root(1)], "spent_root_old": root(0), "spent_root_new": root(2), "txid": "0xabc" },
            { "nullifiers": [root(3)], "spent_root_old": root(2), "spent_root_new": root(4) },
        ],
    });
    let log = parse_settled_log(&json).unwrap();
    assert_eq!(log.batches[0].txid.as_deref(), Some("0xabc"));
    assert_eq!(log.batches[1].txid, None);
    let mut bad = json.clone();
    bad["batches"][0]["txid"] = serde_json::json!(7);
    assert!(parse_settled_log(&bad).is_err());
}

#[test]
fn rebuild_rejects_a_batch_no_root_reaches() {
    let mut log = settled_log(&[vec![key(1)], vec![key(2)]]);
    let mut orphan = log[1].clone();
    orphan.spent_root_old = key(99);
    log.push(orphan);
    let err = rebuild(&log).unwrap_err().to_string();
    assert!(err.contains("does not chain"), "{err}");
}

#[test]
fn rebuild_rejects_two_different_batches_from_one_root() {
    let mut log = settled_log(&[vec![key(1)], vec![key(2)]]);
    let mut rival = log[1].clone();
    rival.nullifiers = vec![key(3)];
    log.push(rival);
    let err = rebuild(&log).unwrap_err().to_string();
    assert!(err.contains("both continue root"), "{err}");
}

#[test]
fn next_batch_rejects_a_double_spend() {
    let log = settled_log(&[vec![key(1), key(2)]]);
    let acc = rebuild(&log).expect("rebuild");
    // key(1) was already spent in batch 0.
    assert!(next_batch(&acc, &[key(1)]).is_err());
}
