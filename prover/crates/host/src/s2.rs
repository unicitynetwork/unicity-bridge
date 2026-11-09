//! S2 — rebuild the nullifier accumulator from on-chain settlement events.
//!
//! The vault advances its replay accumulator once per settled batch and emits:
//!   - `Released { nullifier, recipient, amount, feeRecipient, feeAmount, deadline }`
//!     once per leaf, and
//!   - `BatchFulfilled { spentRootOld, spentRootNew, batchSize, totalAmount }`
//!     once per batch.
//!
//! To settle the **next** batch, a relayer (S4) must reconstruct the set of
//! already-spent nullifiers — in insertion order — so the prover can build
//! non-membership witnesses against the vault's current `spentRoot`. This module
//! replays the event log into the (sparse-Merkle) accumulator, **verifies each
//! batch's root transition against the chain** (so a corrupted/incomplete log is
//! caught), and then produces the witnesses + root transition for a new batch.
//!
//! It is chain-agnostic: the caller fetches and decodes the events (TronGrid /
//! an EVM RPC) into [`SettledBatch`]s; this module owns only the accumulator math.
//! Witnesses come from the same `sdk-ext` SMT the guest verifies against; the replay
//! computes its roots with a tree that hashes as that one does but keeps its subtree
//! hashes, so it stays linear in the nullifiers spent.

use std::collections::{HashMap, HashSet};

use bridge_return_sdk_ext::accumulator::{
    insert as accumulator_insert, ordered_insert_witnesses, NonMembershipWitness, NullifierTree,
    EMPTY_TREE_ROOT,
};
use unicity_token::crypto::hash::sha256;

use crate::{HostError, Result};

/// One settled batch, as read from the vault's events. `nullifiers` are the
/// per-leaf `Released.nullifier` values **in the batch's leaf order**; the roots
/// are the matching `BatchFulfilled` fields, and `txid` the transaction that emitted
/// them, when the source names it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SettledBatch {
    pub nullifiers: Vec<[u8; 32]>,
    pub spent_root_old: [u8; 32],
    pub spent_root_new: [u8; 32],
    pub txid: Option<String>,
}

impl SettledBatch {
    /// The same accumulator step: what one source names its transaction does not change it.
    pub fn same_transition(&self, other: &SettledBatch) -> bool {
        self.nullifiers == other.nullifiers
            && self.spent_root_old == other.spent_root_old
            && self.spent_root_new == other.spent_root_new
    }
}

/// The decoded settlement log a chain watcher hands to S2: the ordered settled
/// batches plus, optionally, the vault's current on-chain `spentRoot` so the
/// caller can assert the rebuilt accumulator actually matches the chain.
#[derive(Debug, Clone, Default)]
pub struct SettledLog {
    pub batches: Vec<SettledBatch>,
    /// The vault's live `spentRoot()`, if the watcher included it.
    pub on_chain_spent_root: Option<[u8; 32]>,
}

/// Parse the settlement-log JSON emitted by a chain watcher (e.g.
/// `relayer.js events`) into a [`SettledLog`]. Shape:
/// `{ "batches": [{ "nullifiers": [hex…], "spent_root_old": hex, "spent_root_new": hex,
///    "txid": string? }], "spent_root": hex? }`. This is the single decoder shared by the `s2-rebuild`
/// CLI and the live service, so both agree on the wire shape.
pub fn parse_settled_log(json: &serde_json::Value) -> Result<SettledLog> {
    let err = |m: String| HostError::Check(m);
    let b32 = |v: &serde_json::Value| -> Result<[u8; 32]> {
        let s = v
            .as_str()
            .ok_or_else(|| err("expected hex string".to_string()))?;
        hex::decode(s.strip_prefix("0x").unwrap_or(s))?
            .try_into()
            .map_err(|_| err("expected 32 bytes".to_string()))
    };
    let b32_arr = |v: &serde_json::Value| -> Result<Vec<[u8; 32]>> {
        v.as_array()
            .ok_or_else(|| err("expected array".to_string()))?
            .iter()
            .map(b32)
            .collect()
    };
    let mut batches = Vec::new();
    for b in json["batches"]
        .as_array()
        .ok_or_else(|| err("settlement log: missing `batches` array".to_string()))?
    {
        let txid = match b.get("txid") {
            None | Some(serde_json::Value::Null) => None,
            Some(serde_json::Value::String(t)) if t.is_empty() => None,
            Some(serde_json::Value::String(t)) => Some(t.clone()),
            Some(_) => return Err(err("settlement log: `txid` must be a string".to_string())),
        };
        batches.push(SettledBatch {
            nullifiers: b32_arr(&b["nullifiers"])?,
            spent_root_old: b32(&b["spent_root_old"])?,
            spent_root_new: b32(&b["spent_root_new"])?,
            txid,
        });
    }
    let on_chain_spent_root = match json.get("spent_root") {
        Some(v) if !v.is_null() => Some(b32(v)?),
        _ => None,
    };
    Ok(SettledLog {
        batches,
        on_chain_spent_root,
    })
}

/// The reconstructed accumulator after replaying the event log.
#[derive(Debug, Clone)]
pub struct RebuiltAccumulator {
    pub tree: NullifierTree,
    /// Current root — equals the vault's on-chain `spentRoot`.
    pub spent_root: [u8; 32],
    /// Total nullifiers spent so far (across all batches).
    pub spent_count: usize,
    /// The transaction that spent each nullifier, for those whose batch a source named one for.
    pub txids: HashMap<[u8; 32], String>,
    /// The transitions replayed, in chain order, one per distinct batch, each with the
    /// first txid any copy of it names.
    pub batches: Vec<SettledBatch>,
    spent: HashSet<[u8; 32]>,
}

impl RebuiltAccumulator {
    /// Whether the log spent `nullifier`.
    pub fn is_spent(&self, nullifier: &[u8; 32]) -> bool {
        self.spent.contains(nullifier)
    }
}

/// Replay `batches` into the accumulator by following the root links from the
/// empty tree: each step takes the batch whose `spentRootOld` is the current
/// root and checks that its `spentRootNew` is what the accumulator computes.
/// The input may come from more than one source and in any order; a duplicate of
/// the same transition is applied once, and takes the first txid any copy names. A
/// batch no reached root continues, or two different batches continuing the same root,
/// mean the log is incomplete or tampered — the relayer must not settle on top of an
/// unverified state.
pub fn rebuild(batches: &[SettledBatch]) -> Result<RebuiltAccumulator> {
    let mut tree = NullifierTree::new();
    let mut hashed = HashedTree::new();
    let mut spent = HashSet::new();
    let mut root = EMPTY_TREE_ROOT;
    let mut spent_count = 0usize;
    let mut txids = HashMap::new();
    let mut replayed = Vec::new();
    let mut used = vec![false; batches.len()];

    while let Some(i) = next_link(batches, &used, &root)? {
        let batch = &batches[i];
        if batch.nullifiers.is_empty() {
            return Err(HostError::Check(format!("S2 batch {i}: empty batch")));
        }
        for nullifier in &batch.nullifiers {
            if !spent.insert(*nullifier) || !hashed.insert(*nullifier) {
                return Err(HostError::Check(format!(
                    "S2 batch {i}: nullifier 0x{} is spent twice",
                    hex::encode(nullifier),
                )));
            }
        }
        let new_root = hashed.root();
        if new_root != batch.spent_root_new {
            return Err(HostError::Check(format!(
                "S2 batch {i}: recomputed spentRootNew 0x{} != chain 0x{}",
                hex::encode(new_root),
                hex::encode(batch.spent_root_new),
            )));
        }
        for nullifier in &batch.nullifiers {
            tree.insert(*nullifier).map_err(|e| {
                HostError::Check(format!("S2 batch {i}: duplicate nullifier replayed: {e:?}"))
            })?;
        }
        let mut txid = None;
        for (j, other) in batches.iter().enumerate() {
            if other.same_transition(batch) {
                used[j] = true;
                txid = txid.or(other.txid.as_ref());
            }
        }
        if let Some(txid) = txid {
            for nullifier in &batch.nullifiers {
                txids.insert(*nullifier, txid.clone());
            }
        }
        replayed.push(SettledBatch {
            txid: txid.cloned(),
            ..batch.clone()
        });
        root = new_root;
        spent_count += batch.nullifiers.len();
    }

    if let Some(i) = used.iter().position(|u| !u) {
        return Err(HostError::Check(format!(
            "S2 batch {i}: spentRootOld 0x{} does not chain from any settled root (rebuild stopped at 0x{})",
            hex::encode(batches[i].spent_root_old),
            hex::encode(root),
        )));
    }

    Ok(RebuiltAccumulator {
        tree,
        spent_root: root,
        spent_count,
        txids,
        batches: replayed,
        spent,
    })
}

fn next_link(batches: &[SettledBatch], used: &[bool], root: &[u8; 32]) -> Result<Option<usize>> {
    let mut found: Option<usize> = None;
    for (i, batch) in batches.iter().enumerate() {
        if used[i] || batch.spent_root_old != *root {
            continue;
        }
        match found {
            None => found = Some(i),
            Some(f) if batches[f].same_transition(batch) => {}
            Some(f) => {
                return Err(HostError::Check(format!(
                    "S2 batches {f} and {i} both continue root 0x{} with different contents",
                    hex::encode(root),
                )))
            }
        }
    }
    Ok(found)
}

/// Rebuild from a [`SettledLog`] and, when the watcher reported the vault's live
/// `spentRoot`, assert the reconstructed root matches it. A divergence means the
/// event log is incomplete/tampered or came from a different accumulator version
/// — settling on top of it would revert on-chain (`vault: stale root`), so fail
/// loudly *before* spending a ~7-minute proof.
pub fn rebuild_verified(log: &SettledLog) -> Result<RebuiltAccumulator> {
    let acc = rebuild(&log.batches)?;
    if let Some(on_chain) = log.on_chain_spent_root {
        if acc.spent_root != on_chain {
            return Err(HostError::Check(format!(
                "accumulator diverged from chain: rebuilt spentRoot 0x{} != on-chain 0x{} \
                 ({} batch(es), {} nullifier(s) replayed) — event log incomplete or stale",
                hex::encode(acc.spent_root),
                hex::encode(on_chain),
                log.batches.len(),
                acc.spent_count,
            )));
        }
    }
    Ok(acc)
}

/// The accumulator inputs a prover needs to settle a new batch on top of a
/// rebuilt accumulator: `spent_root_old` (= the vault's current `spentRoot`), one
/// non-membership witness per new nullifier (against that root, in order), and
/// the resulting `spent_root_new`.
#[derive(Debug, Clone)]
pub struct NextBatch {
    pub spent_root_old: [u8; 32],
    pub witnesses: Vec<NonMembershipWitness>,
    pub spent_root_new: [u8; 32],
}

/// Build the accumulator transition for a new batch of `new_nullifiers` on top of
/// `acc`. Rejects any nullifier already spent (double-spend) before building.
pub fn next_batch(acc: &RebuiltAccumulator, new_nullifiers: &[[u8; 32]]) -> Result<NextBatch> {
    if new_nullifiers.is_empty() {
        return Err(HostError::Check("S2 next batch: empty".to_string()));
    }
    let spent_root_old = acc.tree.root();
    let (witnesses, spent_root_new) = ordered_insert_witnesses(&acc.tree, new_nullifiers)
        .map_err(|e| HostError::Check(format!("S2 next batch: {e:?} (already spent?)")))?;
    // Defensive: replay the transition exactly as the guest will — each witness is
    // checked against the *running* root (after the prior inserts in this batch),
    // and the fold must reproduce spent_root_new. Catches any witness/root bug.
    let mut running = spent_root_old;
    for (w, n) in witnesses.iter().zip(new_nullifiers) {
        running = accumulator_insert(&running, n, w).ok_or_else(|| {
            HostError::Check("S2 next batch: produced a witness that does not verify".to_string())
        })?;
    }
    if running != spent_root_new {
        return Err(HostError::Check(
            "S2 next batch: folded root != spent_root_new".to_string(),
        ));
    }
    Ok(NextBatch {
        spent_root_old,
        witnesses,
        spent_root_new,
    })
}

/// The accumulator's root, kept current one insert at a time. It hashes exactly as the
/// sdk-ext `NullifierTree` the guest's witnesses come from, but keeps every subtree's
/// hash, so an insert rehashes the new key's path and the key it displaces rather than
/// every key in the tree. `NullifierTree` recomputes the whole tree for each root and
/// each witness, which made replaying the log quadratic in the nullifiers spent.
struct HashedTree {
    top: Subtree,
    empties: Box<[[u8; 32]; DEPTH + 1]>,
}

/// A subtree at some depth: no key, one key (hashed from its leaf up to this depth), or
/// two or more keys split on this depth's bit.
enum Subtree {
    Empty,
    One {
        key: [u8; 32],
        hash: [u8; 32],
    },
    Split {
        left: Box<Subtree>,
        right: Box<Subtree>,
        hash: [u8; 32],
    },
}

const DEPTH: usize = 256;
const LEAF_PREFIX: u8 = 0x00;
const NODE_PREFIX: u8 = 0x01;
const PRESENCE_VALUE: u8 = 0x01;

impl HashedTree {
    fn new() -> Self {
        let mut empties = Box::new([[0u8; 32]; DEPTH + 1]);
        empties[DEPTH] = EMPTY_TREE_ROOT;
        for depth in (0..DEPTH).rev() {
            empties[depth] = node_hash(depth, &empties[depth + 1], &empties[depth + 1]);
        }
        Self {
            top: Subtree::Empty,
            empties,
        }
    }

    fn root(&self) -> [u8; 32] {
        match &self.top {
            Subtree::Empty => EMPTY_TREE_ROOT,
            Subtree::One { hash, .. } | Subtree::Split { hash, .. } => *hash,
        }
    }

    /// Add `key`; `false`, with the tree unchanged, if it is already there.
    fn insert(&mut self, key: [u8; 32]) -> bool {
        self.top.insert(key, 0, &self.empties)
    }
}

impl Subtree {
    fn hash(&self, depth: usize, empties: &[[u8; 32]; DEPTH + 1]) -> [u8; 32] {
        match self {
            Subtree::Empty => empties[depth],
            Subtree::One { hash, .. } | Subtree::Split { hash, .. } => *hash,
        }
    }

    fn insert(&mut self, key: [u8; 32], depth: usize, empties: &[[u8; 32]; DEPTH + 1]) -> bool {
        match self {
            Subtree::Empty => {
                *self = Subtree::One {
                    key,
                    hash: single_key_hash(&key, depth, empties),
                };
            }
            Subtree::One { key: other, .. } => {
                let other = *other;
                // Both keys took the same path down to `depth`, so they first differ at or
                // below it.
                let Some(fork) = (depth..DEPTH).find(|&d| bit_at(&key, d) != bit_at(&other, d))
                else {
                    return false;
                };
                let one = |key: [u8; 32]| Subtree::One {
                    key,
                    hash: single_key_hash(&key, fork + 1, empties),
                };
                let mut node = if bit_at(&key, fork) {
                    split(fork, one(other), one(key), empties)
                } else {
                    split(fork, one(key), one(other), empties)
                };
                for d in (depth..fork).rev() {
                    node = if bit_at(&key, d) {
                        split(d, Subtree::Empty, node, empties)
                    } else {
                        split(d, node, Subtree::Empty, empties)
                    };
                }
                *self = node;
            }
            Subtree::Split { left, right, hash } => {
                let side = if bit_at(&key, depth) {
                    &mut **right
                } else {
                    &mut **left
                };
                if !side.insert(key, depth + 1, empties) {
                    return false;
                }
                *hash = node_hash(
                    depth,
                    &left.hash(depth + 1, empties),
                    &right.hash(depth + 1, empties),
                );
            }
        }
        true
    }
}

fn split(depth: usize, left: Subtree, right: Subtree, empties: &[[u8; 32]; DEPTH + 1]) -> Subtree {
    let hash = node_hash(
        depth,
        &left.hash(depth + 1, empties),
        &right.hash(depth + 1, empties),
    );
    Subtree::Split {
        left: Box::new(left),
        right: Box::new(right),
        hash,
    }
}

/// The hash of a subtree at `depth` holding only `key`: its leaf, folded up with the
/// empty sibling at every level.
fn single_key_hash(key: &[u8; 32], depth: usize, empties: &[[u8; 32]; DEPTH + 1]) -> [u8; 32] {
    let mut leaf = [0u8; 34];
    leaf[0] = LEAF_PREFIX;
    leaf[1..33].copy_from_slice(key);
    leaf[33] = PRESENCE_VALUE;
    let mut hash = digest(&leaf);
    for level in (depth..DEPTH).rev() {
        let sibling = &empties[level + 1];
        hash = if bit_at(key, level) {
            node_hash(level, sibling, &hash)
        } else {
            node_hash(level, &hash, sibling)
        };
    }
    hash
}

fn node_hash(depth: usize, left: &[u8; 32], right: &[u8; 32]) -> [u8; 32] {
    let mut node = [0u8; 66];
    node[0] = NODE_PREFIX;
    node[1] = depth as u8;
    node[2..34].copy_from_slice(left);
    node[34..].copy_from_slice(right);
    digest(&node)
}

fn digest(bytes: &[u8]) -> [u8; 32] {
    let mut out = [0u8; 32];
    out.copy_from_slice(sha256(bytes).data());
    out
}

fn bit_at(data: &[u8; 32], index: usize) -> bool {
    (data[index / 8] >> (index % 8)) & 1 == 1
}

#[cfg(test)]
mod tests {
    use super::*;

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

    fn with_bit(mut key: [u8; 32], index: usize) -> [u8; 32] {
        key[index / 8] ^= 1 << (index % 8);
        key
    }

    /// Insert `keys` one at a time into both trees and compare the roots after each of the
    /// first `every_up_to` and at the end. The sdk tree rehashes every key for each root.
    fn same_roots(keys: &[[u8; 32]], every_up_to: usize) {
        let mut hashed = HashedTree::new();
        let mut reference = NullifierTree::new();
        assert_eq!(hashed.root(), reference.root());
        for (i, k) in keys.iter().enumerate() {
            assert!(hashed.insert(*k), "key {i} refused");
            reference.insert(*k).unwrap();
            if i < every_up_to || i + 1 == keys.len() {
                assert_eq!(hashed.root(), reference.root(), "root after key {i}");
            }
        }
    }

    #[test]
    fn the_root_is_the_sdk_trees() {
        same_roots(&(1..=300).map(key).collect::<Vec<_>>(), 24);
    }

    #[test]
    fn keys_that_part_deep_or_at_the_top_hash_as_the_sdk_tree_does() {
        let base = key(7);
        // Keys that differ only in the last bit, only in the first, and a run of keys that
        // each part from the previous one a level further down, inserted in both orders.
        let mut keys = vec![base, with_bit(base, 255), with_bit(base, 0)];
        keys.extend((1..12).map(|d| with_bit(with_bit(base, 0), d)));
        keys.extend((240..255).map(|d| with_bit(base, d)));
        keys.push([0u8; 32]);
        keys.push([0xff; 32]);
        same_roots(&keys, keys.len());
        keys.reverse();
        same_roots(&keys, keys.len());
    }

    #[test]
    fn a_key_already_there_is_refused_and_changes_nothing() {
        let mut hashed = HashedTree::new();
        assert!(hashed.insert(key(1)));
        assert!(hashed.insert(key(2)));
        let root = hashed.root();
        assert!(!hashed.insert(key(1)));
        assert!(!hashed.insert(key(2)));
        assert_eq!(hashed.root(), root);
    }
}
