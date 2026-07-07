//! Minimal deterministic (canonical) CBOR encoder — only the item types the
//! bridge structures use: definite-length, minimal-length integer arguments.
//! `H(...)` (the SDK's hash convention) is SHA-256 over a CBOR array of fields.

use crate::hash::sha256;

/// major type 0: unsigned integer.
pub fn uint(n: u64) -> Vec<u8> {
    head(0, n)
}

/// major type 2: byte string.
pub fn bytes(b: &[u8]) -> Vec<u8> {
    let mut v = head(2, b.len() as u64);
    v.extend_from_slice(b);
    v
}

/// major type 3: text string.
pub fn text(s: &str) -> Vec<u8> {
    let mut v = head(3, s.len() as u64);
    v.extend_from_slice(s.as_bytes());
    v
}

/// major type 4: array header (followed by `len` items).
pub fn array_header(len: u64) -> Vec<u8> {
    head(4, len)
}

/// major type 6: semantic tag.
pub fn tag(t: u64) -> Vec<u8> {
    head(6, t)
}

/// `H(fields...) = SHA-256( CBOR-array(fields) )` (00 §8 / appendix convention).
pub fn h_array(items: &[Vec<u8>]) -> [u8; 32] {
    let mut buf = array_header(items.len() as u64);
    for it in items {
        buf.extend_from_slice(it);
    }
    sha256(&buf)
}

/// `major<<5 | argument`, minimal-length per canonical CBOR.
fn head(major: u8, arg: u64) -> Vec<u8> {
    let m = major << 5;
    if arg < 24 {
        vec![m | (arg as u8)]
    } else if arg <= 0xff {
        vec![m | 24, arg as u8]
    } else if arg <= 0xffff {
        let mut v = vec![m | 25];
        v.extend_from_slice(&(arg as u16).to_be_bytes());
        v
    } else if arg <= 0xffff_ffff {
        let mut v = vec![m | 26];
        v.extend_from_slice(&(arg as u32).to_be_bytes());
        v
    } else {
        let mut v = vec![m | 27];
        v.extend_from_slice(&arg.to_be_bytes());
        v
    }
}
