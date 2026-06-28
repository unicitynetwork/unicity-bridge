//! Sparse Merkle presence accumulator (depth-256) for bridge return nullifiers.
//!
//! Each present key occupies a leaf at depth 256 and the root is framed from
//! depth 0 using empty-subtree default hashes, so **every prefix bit is bound**.
//! This replaces the earlier path-compressed radix tree, whose root was the top
//! branch node and therefore did not bind the bits above it — a key diverging in
//! that compressed prefix had no representable non-membership witness (it failed
//! its own verifier for ~21% of >=2-element trees). A full SMT has a sound,
//! representable non-membership proof for every key at every tree size.
//!
//! Witness shape (unchanged on the wire): `terminal` is always `Empty` (an absent
//! key's slot is empty) and `steps` carries the sibling hash at every depth from
//! the terminal up to the root (deepest first), so `td` (the terminal depth) is
//! `steps[0].depth + 1`, or 0 for the empty tree (no steps). `verify`/`insert`
//! are stateless: they rebuild the root from the witness alone.

use alloc::vec::Vec;

use unicity_token::crypto::hash::sha256;
use unicity_token::Error;

/// Sentinel root of the empty accumulator (matches the vault's initial `spentRoot`).
pub const EMPTY_TREE_ROOT: [u8; 32] = [0u8; 32];

const LEAF_PREFIX: u8 = 0x00;
const NODE_PREFIX: u8 = 0x01;
const PRESENCE_VALUE: u8 = 0x01;
const DEPTH: usize = 256;
const MAX_STEPS: usize = 256;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SmtProofStep {
    depth: u8,
    sibling_hash: [u8; 32],
}

impl SmtProofStep {
    pub fn new(depth: u8, sibling_hash: [u8; 32]) -> Self {
        SmtProofStep {
            depth,
            sibling_hash,
        }
    }

    pub fn depth(&self) -> u8 {
        self.depth
    }

    pub fn sibling_hash(&self) -> &[u8; 32] {
        &self.sibling_hash
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NonMembershipTerminal {
    Empty,
    Occupied { key: [u8; 32] },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NonMembershipWitness {
    terminal: NonMembershipTerminal,
    steps: Vec<SmtProofStep>,
}

impl NonMembershipWitness {
    pub fn new(terminal: NonMembershipTerminal, steps: Vec<SmtProofStep>) -> Self {
        NonMembershipWitness { terminal, steps }
    }

    pub fn terminal(&self) -> &NonMembershipTerminal {
        &self.terminal
    }

    pub fn steps(&self) -> &[SmtProofStep] {
        &self.steps
    }
}

/// Verify that `key` is absent from the tree committed by `root`.
pub fn verify_non_member(root: &[u8; 32], key: &[u8; 32], witness: &NonMembershipWitness) -> bool {
    let empties = empty_hashes();
    rebuild_absent_root(key, witness, &empties).is_some_and(|rebuilt| rebuilt == *root)
}

/// Given a verified non-membership `witness` for `key` against `root`, return the
/// root after inserting `key`. `None` if the witness does not prove absence.
pub fn insert(root: &[u8; 32], key: &[u8; 32], witness: &NonMembershipWitness) -> Option<[u8; 32]> {
    let empties = empty_hashes();
    if rebuild_absent_root(key, witness, &empties)? != *root {
        return None;
    }
    if witness.steps.is_empty() {
        // First insert into the empty tree.
        return Some(single_key_hash(key, 0, &empties));
    }
    // Replace the empty terminal slot with `key` and fold back up the same path.
    let td = terminal_depth(&witness.steps)?;
    let terminal = single_key_hash(key, td, &empties);
    fold_steps(key, terminal, &witness.steps)
}

#[derive(Default, Debug, Clone)]
pub struct NullifierTree {
    keys: Vec<[u8; 32]>,
}

impl NullifierTree {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn root(&self) -> [u8; 32] {
        if self.keys.is_empty() {
            return EMPTY_TREE_ROOT;
        }
        subtree_hash(&self.keys, 0, &empty_hashes())
    }

    /// Non-membership witness for `key`; `None` if `key` is already present.
    pub fn non_membership_witness(&self, key: &[u8; 32]) -> Option<NonMembershipWitness> {
        if self.keys.iter().any(|existing| existing == key) {
            return None;
        }
        let empties = empty_hashes();
        let mut steps = Vec::new();
        let mut current: Vec<[u8; 32]> = self.keys.clone();
        let mut depth = 0usize;
        loop {
            if current.is_empty() {
                break; // key's slot is empty at `depth`
            }
            if current.len() == 1 {
                let other = current[0];
                let f = first_differing_bit(key, &other)?; // >= depth, exists since absent
                for d in depth..f {
                    steps.push(SmtProofStep::new(d as u8, empties[d + 1]));
                }
                steps.push(SmtProofStep::new(
                    f as u8,
                    single_key_hash(&other, f + 1, &empties),
                ));
                break;
            }
            // >= 2 keys: branch at `depth`. The sibling subtree is everything on
            // the opposite side of `key`'s bit (possibly empty -> default hash).
            let go_right = bit_at(key, depth);
            let mut mine = Vec::new();
            let mut sibling = Vec::new();
            for k in &current {
                if bit_at(k, depth) == go_right {
                    mine.push(*k);
                } else {
                    sibling.push(*k);
                }
            }
            steps.push(SmtProofStep::new(
                depth as u8,
                subtree_hash(&sibling, depth + 1, &empties),
            ));
            current = mine;
            depth += 1;
        }
        steps.reverse(); // deepest first, for the fold
        Some(NonMembershipWitness::new(
            NonMembershipTerminal::Empty,
            steps,
        ))
    }

    pub fn insert(&mut self, key: [u8; 32]) -> core::result::Result<(), Error> {
        if self.keys.iter().any(|existing| *existing == key) {
            return Err(Error::UnexpectedValue("duplicate accumulator key"));
        }
        self.keys.push(key);
        Ok(())
    }
}

pub fn ordered_insert_witnesses(
    start: &NullifierTree,
    keys: &[[u8; 32]],
) -> core::result::Result<(Vec<NonMembershipWitness>, [u8; 32]), Error> {
    let mut tree = start.clone();
    let mut witnesses = Vec::with_capacity(keys.len());
    for key in keys {
        let witness = tree
            .non_membership_witness(key)
            .ok_or(Error::UnexpectedValue("duplicate accumulator key"))?;
        witnesses.push(witness);
        tree.insert(*key)?;
    }
    Ok((witnesses, tree.root()))
}

// --- internals ---------------------------------------------------------------

/// Rebuild the root of the tree in which `key` is absent, from the witness alone.
fn rebuild_absent_root(
    key: &[u8; 32],
    witness: &NonMembershipWitness,
    empties: &[[u8; 32]; DEPTH + 1],
) -> Option<[u8; 32]> {
    // Only the `Empty` terminal is produced; an absent key's slot is empty.
    if witness.terminal != NonMembershipTerminal::Empty {
        return None;
    }
    if witness.steps.is_empty() {
        return Some(EMPTY_TREE_ROOT); // empty tree
    }
    if witness.steps.len() > MAX_STEPS {
        return None;
    }
    let td = terminal_depth(&witness.steps)?;
    fold_steps(key, empties[td], &witness.steps)
}

/// Terminal depth = one below the deepest (first) step. Requires the steps to
/// descend contiguously to depth 0 (the canonical shape the prover emits).
fn terminal_depth(steps: &[SmtProofStep]) -> Option<usize> {
    let top = steps.first()?.depth as usize;
    for (i, step) in steps.iter().enumerate() {
        if step.depth as usize != top - i {
            return None;
        }
    }
    if steps.last()?.depth != 0 {
        return None;
    }
    Some(top + 1)
}

/// Fold `hash` (at the terminal depth) up to the root using `steps` (deepest first).
fn fold_steps(key: &[u8; 32], mut hash: [u8; 32], steps: &[SmtProofStep]) -> Option<[u8; 32]> {
    let mut previous_depth: u16 = u16::try_from(DEPTH).ok()? + 1;
    for step in steps {
        if u16::from(step.depth) >= previous_depth {
            return None;
        }
        hash = if bit_at(key, step.depth as usize) {
            node_hash(step.depth, &step.sibling_hash, &hash)
        } else {
            node_hash(step.depth, &hash, &step.sibling_hash)
        };
        previous_depth = u16::from(step.depth);
    }
    Some(hash)
}

/// Hash of the subtree holding `keys`, all sharing the prefix down to `depth`.
fn subtree_hash(keys: &[[u8; 32]], depth: usize, empties: &[[u8; 32]; DEPTH + 1]) -> [u8; 32] {
    match keys.len() {
        0 => empties[depth],
        1 => single_key_hash(&keys[0], depth, empties),
        _ => {
            let mut left = Vec::new();
            let mut right = Vec::new();
            for k in keys {
                if bit_at(k, depth) {
                    right.push(*k);
                } else {
                    left.push(*k);
                }
            }
            node_hash(
                depth as u8,
                &subtree_hash(&left, depth + 1, empties),
                &subtree_hash(&right, depth + 1, empties),
            )
        }
    }
}

/// Hash of a subtree containing exactly `key`, occupying the slot from `depth`
/// down to the leaf at depth 256 (single-child path with empty siblings).
fn single_key_hash(key: &[u8; 32], depth: usize, empties: &[[u8; 32]; DEPTH + 1]) -> [u8; 32] {
    let mut hash = leaf_hash(key);
    for level in (depth..DEPTH).rev() {
        let sibling = &empties[level + 1];
        hash = if bit_at(key, level) {
            node_hash(level as u8, sibling, &hash)
        } else {
            node_hash(level as u8, &hash, sibling)
        };
    }
    hash
}

/// Default hashes of empty subtrees by depth: `E[256]` = empty leaf, `E[d]` =
/// `node(d, E[d+1], E[d+1])`. Computed once per call (256 hashes).
fn empty_hashes() -> [[u8; 32]; DEPTH + 1] {
    let mut table = [[0u8; 32]; DEPTH + 1];
    table[DEPTH] = EMPTY_TREE_ROOT;
    for depth in (0..DEPTH).rev() {
        table[depth] = node_hash(depth as u8, &table[depth + 1], &table[depth + 1]);
    }
    table
}

fn first_differing_bit(left: &[u8; 32], right: &[u8; 32]) -> Option<usize> {
    (0..DEPTH).find(|&depth| bit_at(left, depth) != bit_at(right, depth))
}

fn leaf_hash(key: &[u8; 32]) -> [u8; 32] {
    sha256_raw(&[&[LEAF_PREFIX], key, &[PRESENCE_VALUE]])
}

fn node_hash(depth: u8, left: &[u8; 32], right: &[u8; 32]) -> [u8; 32] {
    sha256_raw(&[&[NODE_PREFIX, depth], left, right])
}

fn sha256_raw(parts: &[&[u8]]) -> [u8; 32] {
    let len = parts.iter().map(|part| part.len()).sum();
    let mut buf = Vec::with_capacity(len);
    for part in parts {
        buf.extend_from_slice(part);
    }
    let mut out = [0u8; 32];
    out.copy_from_slice(sha256(&buf).data());
    out
}

fn bit_at(data: &[u8], index: usize) -> bool {
    (data[index / 8] >> (index % 8)) & 1 == 1
}
