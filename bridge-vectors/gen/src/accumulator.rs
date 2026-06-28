//! Reference nullifier accumulator for vector generation.
//!
//! Independent implementation of the depth-256 sparse Merkle tree in
//! `prover/crates/sdk-ext/src/accumulator.rs` (LSB-first bits, SHA-256 leaf/node
//! prefixes). The root is framed from depth 0 with empty-subtree default hashes
//! so every prefix bit is bound and non-membership is provable at any tree size.
//! The two implementations must agree byte-for-byte (the cross-stack rule).

use crate::hash::sha256;

pub const EMPTY_TREE_ROOT: [u8; 32] = [0u8; 32];

const LEAF_PREFIX: u8 = 0x00;
const NODE_PREFIX: u8 = 0x01;
const PRESENCE_VALUE: u8 = 0x01;
const DEPTH: usize = 256;

#[derive(Clone)]
pub enum Terminal {
    Empty,
    Occupied([u8; 32]),
}

#[derive(Clone)]
pub struct Step {
    pub depth: u8,
    pub sibling_hash: [u8; 32],
}

#[derive(Clone)]
pub struct Witness {
    pub terminal: Terminal,
    pub steps: Vec<Step>,
}

#[derive(Default, Clone)]
pub struct Tree {
    keys: Vec<[u8; 32]>,
}

impl Tree {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn root(&self) -> [u8; 32] {
        if self.keys.is_empty() {
            return EMPTY_TREE_ROOT;
        }
        subtree_hash(&self.keys, 0, &empty_hashes())
    }

    pub fn witness(&self, key: &[u8; 32]) -> Option<Witness> {
        if self.keys.iter().any(|existing| existing == key) {
            return None;
        }
        let empties = empty_hashes();
        let mut steps = Vec::new();
        let mut current: Vec<[u8; 32]> = self.keys.clone();
        let mut depth = 0usize;
        loop {
            if current.is_empty() {
                break;
            }
            if current.len() == 1 {
                let other = current[0];
                let f = first_differing_bit(key, &other).expect("absent key differs");
                for d in depth..f {
                    steps.push(Step {
                        depth: d as u8,
                        sibling_hash: empties[d + 1],
                    });
                }
                steps.push(Step {
                    depth: f as u8,
                    sibling_hash: single_key_hash(&other, f + 1, &empties),
                });
                break;
            }
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
            steps.push(Step {
                depth: depth as u8,
                sibling_hash: subtree_hash(&sibling, depth + 1, &empties),
            });
            current = mine;
            depth += 1;
        }
        steps.reverse();
        Some(Witness {
            terminal: Terminal::Empty,
            steps,
        })
    }

    pub fn insert(&mut self, key: [u8; 32]) {
        assert!(!self.keys.iter().any(|existing| *existing == key));
        self.keys.push(key);
    }
}

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
    sha256(&concat(&[&[LEAF_PREFIX], key, &[PRESENCE_VALUE]]))
}

fn node_hash(depth: u8, left: &[u8; 32], right: &[u8; 32]) -> [u8; 32] {
    sha256(&concat(&[&[NODE_PREFIX, depth], left, right]))
}

fn concat(parts: &[&[u8]]) -> Vec<u8> {
    let len = parts.iter().map(|part| part.len()).sum();
    let mut out = Vec::with_capacity(len);
    for part in parts {
        out.extend_from_slice(part);
    }
    out
}

fn bit_at(key: &[u8; 32], depth: usize) -> bool {
    (key[depth / 8] >> (depth % 8)) & 1 == 1
}
