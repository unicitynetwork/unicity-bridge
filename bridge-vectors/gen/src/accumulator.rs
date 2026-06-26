//! Reference nullifier accumulator for vector generation.
//!
//! Plain radix sparse Merkle tree, LSB-first, SHA-256 leaf/node prefixes
//! matching `state-transition-sdk-rust::accumulator`.

use crate::hash::sha256;

pub const EMPTY_TREE_ROOT: [u8; 32] = [0u8; 32];

const LEAF_PREFIX: u8 = 0x00;
const NODE_PREFIX: u8 = 0x01;
const PRESENCE_VALUE: u8 = 0x01;

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
        build_node(&self.keys, 0).map_or(EMPTY_TREE_ROOT, |node| node.hash())
    }

    pub fn witness(&self, key: &[u8; 32]) -> Option<Witness> {
        let node = build_node(&self.keys, 0)?;
        Some(node.witness(key))
    }

    pub fn insert(&mut self, key: [u8; 32]) {
        assert!(!self.keys.iter().any(|existing| *existing == key));
        self.keys.push(key);
    }
}

enum Node {
    Empty,
    Leaf {
        key: [u8; 32],
        hash: [u8; 32],
    },
    Branch {
        depth: u8,
        left: Box<Node>,
        right: Box<Node>,
        hash: [u8; 32],
    },
}

impl Node {
    fn hash(&self) -> [u8; 32] {
        match self {
            Node::Empty => EMPTY_TREE_ROOT,
            Node::Leaf { hash, .. } | Node::Branch { hash, .. } => *hash,
        }
    }

    fn witness(&self, key: &[u8; 32]) -> Witness {
        let mut steps = Vec::new();
        let terminal = self.collect(key, &mut steps);
        steps.reverse();
        Witness { terminal, steps }
    }

    fn collect(&self, key: &[u8; 32], steps: &mut Vec<Step>) -> Terminal {
        match self {
            Node::Empty => Terminal::Empty,
            Node::Leaf { key, .. } => Terminal::Occupied(*key),
            Node::Branch {
                depth, left, right, ..
            } => {
                let (next, sibling) = if bit_at(key, *depth) {
                    (right, left)
                } else {
                    (left, right)
                };
                let terminal = next.collect(key, steps);
                steps.push(Step {
                    depth: *depth,
                    sibling_hash: sibling.hash(),
                });
                terminal
            }
        }
    }
}

fn build_node(keys: &[[u8; 32]], start_bit: u16) -> Option<Node> {
    if keys.is_empty() {
        return Some(Node::Empty);
    }
    if keys.len() == 1 {
        let key = keys[0];
        return Some(Node::Leaf {
            key,
            hash: leaf_hash(&key),
        });
    }

    let mut depth = start_bit;
    let bifurcation = loop {
        if depth > 255 {
            return None;
        }
        let first = bit_at(&keys[0], depth as u8);
        if keys.iter().any(|key| bit_at(key, depth as u8) != first) {
            break depth as u8;
        }
        depth += 1;
    };

    let mut left = Vec::new();
    let mut right = Vec::new();
    for key in keys {
        if bit_at(key, bifurcation) {
            right.push(*key);
        } else {
            left.push(*key);
        }
    }

    let left = Box::new(build_node(&left, u16::from(bifurcation) + 1)?);
    let right = Box::new(build_node(&right, u16::from(bifurcation) + 1)?);
    let hash = node_hash(bifurcation, &left.hash(), &right.hash());
    Some(Node::Branch {
        depth: bifurcation,
        left,
        right,
        hash,
    })
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

fn bit_at(key: &[u8; 32], depth: u8) -> bool {
    (key[depth as usize / 8] >> (depth % 8)) & 1 == 1
}
