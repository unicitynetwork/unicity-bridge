use sha2::{Digest, Sha256};
use tiny_keccak::{Hasher, Keccak};

use crate::Bytes32;

pub fn sha256(input: &[u8]) -> Bytes32 {
    Sha256::digest(input).into()
}

pub fn keccak256(input: &[u8]) -> Bytes32 {
    let mut hasher = Keccak::v256();
    let mut out = [0u8; 32];
    hasher.update(input);
    hasher.finalize(&mut out);
    out
}
