use alloc::vec::Vec;

use crate::{hash, Bytes32};

pub fn uint(value: u64) -> Vec<u8> {
    major(0, value)
}

pub fn bytes(value: &[u8]) -> Vec<u8> {
    let mut out = major(2, value.len() as u64);
    out.extend_from_slice(value);
    out
}

pub fn text(value: &str) -> Vec<u8> {
    let mut out = major(3, value.len() as u64);
    out.extend_from_slice(value.as_bytes());
    out
}

pub fn array_header(len: u64) -> Vec<u8> {
    major(4, len)
}

pub fn tag(value: u64) -> Vec<u8> {
    major(6, value)
}

pub fn h_array(items: &[Vec<u8>]) -> Bytes32 {
    let mut encoded = array_header(items.len() as u64);
    for item in items {
        encoded.extend_from_slice(item);
    }
    hash::sha256(&encoded)
}

fn major(major: u8, value: u64) -> Vec<u8> {
    let prefix = major << 5;
    match value {
        0..=23 => alloc::vec![prefix | value as u8],
        24..=0xff => alloc::vec![prefix | 24, value as u8],
        0x100..=0xffff => {
            let mut out = alloc::vec![prefix | 25];
            out.extend_from_slice(&(value as u16).to_be_bytes());
            out
        }
        0x1_0000..=0xffff_ffff => {
            let mut out = alloc::vec![prefix | 26];
            out.extend_from_slice(&(value as u32).to_be_bytes());
            out
        }
        _ => {
            let mut out = alloc::vec![prefix | 27];
            out.extend_from_slice(&value.to_be_bytes());
            out
        }
    }
}
