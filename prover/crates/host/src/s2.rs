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
//! an EVM RPC) into [`SettledBatch`]s; this module owns only the accumulator math
//! and reuses the same `sdk-ext` SMT the guest verifies against.

use bridge_return_sdk_ext::accumulator::{
    insert as accumulator_insert, ordered_insert_witnesses, NonMembershipWitness, NullifierTree,
    EMPTY_TREE_ROOT,
};

use crate::{HostError, Result};

/// One settled batch, as read from the vault's events. `nullifiers` are the
/// per-leaf `Released.nullifier` values **in the batch's leaf order**; the roots
/// are the matching `BatchFulfilled` fields.
#[derive(Debug, Clone)]
pub struct SettledBatch {
    pub nullifiers: Vec<[u8; 32]>,
    pub spent_root_old: [u8; 32],
    pub spent_root_new: [u8; 32],
}

/// The reconstructed accumulator after replaying the event log.
#[derive(Debug, Clone)]
pub struct RebuiltAccumulator {
    pub tree: NullifierTree,
    /// Current root — equals the vault's on-chain `spentRoot`.
    pub spent_root: [u8; 32],
    /// Total nullifiers spent so far (across all batches).
    pub spent_count: usize,
}

/// Replay `batches` (in chain order) into the accumulator, checking that each
/// batch's `spentRootOld` chains from the previous root and its `spentRootNew`
/// matches the root the accumulator computes. A mismatch means the event log is
/// out of order, incomplete, or tampered — the relayer must not settle on top of
/// an unverified state.
pub fn rebuild(batches: &[SettledBatch]) -> Result<RebuiltAccumulator> {
    let mut tree = NullifierTree::new();
    let mut root = EMPTY_TREE_ROOT;
    let mut spent_count = 0usize;

    for (i, batch) in batches.iter().enumerate() {
        if batch.spent_root_old != root {
            return Err(HostError::Check(format!(
                "S2 batch {i}: spentRootOld 0x{} does not chain from current root 0x{}",
                hex::encode(batch.spent_root_old),
                hex::encode(root),
            )));
        }
        if batch.nullifiers.is_empty() {
            return Err(HostError::Check(format!("S2 batch {i}: empty batch")));
        }
        // Recompute the transition the same way the circuit did.
        let (_, new_root) = ordered_insert_witnesses(&tree, &batch.nullifiers)
            .map_err(|e| HostError::Check(format!("S2 batch {i}: insert failed: {e:?}")))?;
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
        root = new_root;
        spent_count += batch.nullifiers.len();
    }

    Ok(RebuiltAccumulator {
        tree,
        spent_root: root,
        spent_count,
    })
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
