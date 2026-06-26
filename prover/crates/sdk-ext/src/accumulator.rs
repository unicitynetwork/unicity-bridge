//! Plain radix sparse-Merkle presence accumulator for bridge return nullifiers.

use alloc::boxed::Box;
use alloc::vec::Vec;

use unicity_token::crypto::hash::sha256;
use unicity_token::Error;

pub const EMPTY_TREE_ROOT: [u8; 32] = [0u8; 32];

const LEAF_PREFIX: u8 = 0x00;
const NODE_PREFIX: u8 = 0x01;
const PRESENCE_VALUE: u8 = 0x01;
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

pub fn verify_non_member(root: &[u8; 32], key: &[u8; 32], witness: &NonMembershipWitness) -> bool {
    rebuild_absent_root(key, witness).is_some_and(|rebuilt| rebuilt == *root)
}

pub fn insert(root: &[u8; 32], key: &[u8; 32], witness: &NonMembershipWitness) -> Option<[u8; 32]> {
    if !verify_non_member(root, key, witness) {
        return None;
    }

    let inserted = leaf_hash(key);
    let hash = match witness.terminal {
        NonMembershipTerminal::Empty => inserted,
        NonMembershipTerminal::Occupied { key: other } => {
            if other == *key {
                return None;
            }
            let depth = first_differing_bit(key, &other)?;
            let other_hash = leaf_hash(&other);
            let first_step_depth = witness.steps.first().map(SmtProofStep::depth);
            if first_step_depth.is_some_and(|step_depth| step_depth >= depth) {
                return None;
            }
            if bit_at(key, depth as usize) {
                node_hash(depth, &other_hash, &inserted)
            } else {
                node_hash(depth, &inserted, &other_hash)
            }
        }
    };

    fold_steps(key, hash, &witness.steps)
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
        build_root(&self.keys)
    }

    pub fn non_membership_witness(&self, key: &[u8; 32]) -> Option<NonMembershipWitness> {
        let root = build_node(&self.keys, 0)?;
        Some(root.non_membership_witness(key))
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

#[derive(Debug)]
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

    fn non_membership_witness(&self, key: &[u8; 32]) -> NonMembershipWitness {
        let mut steps = Vec::new();
        let terminal = self.collect_non_membership(key, &mut steps);
        steps.reverse();
        NonMembershipWitness { terminal, steps }
    }

    fn collect_non_membership(
        &self,
        key: &[u8; 32],
        steps: &mut Vec<SmtProofStep>,
    ) -> NonMembershipTerminal {
        match self {
            Node::Empty => NonMembershipTerminal::Empty,
            Node::Leaf { key: existing, .. } => NonMembershipTerminal::Occupied { key: *existing },
            Node::Branch {
                depth, left, right, ..
            } => {
                let (next, sibling) = if bit_at(key, *depth as usize) {
                    (right, left)
                } else {
                    (left, right)
                };
                let terminal = next.collect_non_membership(key, steps);
                steps.push(SmtProofStep::new(*depth, sibling.hash()));
                terminal
            }
        }
    }
}

fn build_root(keys: &[[u8; 32]]) -> [u8; 32] {
    build_node(keys, 0).map_or(EMPTY_TREE_ROOT, |node| node.hash())
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
        let first = bit_at(&keys[0], depth as usize);
        if keys.iter().any(|key| bit_at(key, depth as usize) != first) {
            break depth as u8;
        }
        depth += 1;
    };

    let mut left = Vec::new();
    let mut right = Vec::new();
    for key in keys {
        if bit_at(key, bifurcation as usize) {
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

fn rebuild_absent_root(key: &[u8; 32], witness: &NonMembershipWitness) -> Option<[u8; 32]> {
    if witness.steps.len() > MAX_STEPS {
        return None;
    }

    let first_step_depth = witness.steps.first().map(SmtProofStep::depth);
    let terminal_hash = match witness.terminal {
        NonMembershipTerminal::Empty => EMPTY_TREE_ROOT,
        NonMembershipTerminal::Occupied { key: other } => {
            if other == *key {
                return None;
            }
            let divergence = first_differing_bit(key, &other)?;
            if first_step_depth.is_some_and(|depth| depth >= divergence) {
                return None;
            }
            leaf_hash(&other)
        }
    };
    fold_steps(key, terminal_hash, &witness.steps)
}

fn fold_steps(key: &[u8; 32], mut hash: [u8; 32], steps: &[SmtProofStep]) -> Option<[u8; 32]> {
    let mut previous_depth: u16 = 256;
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

fn first_differing_bit(left: &[u8; 32], right: &[u8; 32]) -> Option<u8> {
    (0..=255)
        .find(|&depth| bit_at(left, depth) != bit_at(right, depth))
        .map(|depth| depth as u8)
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
