use alloc::vec::Vec;

use crate::{Address, Bytes32, U256};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Val {
    Str(&'static str),
    U32(u32),
    U64(u64),
    U256(U256),
    Addr(Address),
    B32(Bytes32),
}

impl Val {
    fn is_dynamic(self) -> bool {
        matches!(self, Val::Str(_))
    }

    fn word(self) -> [u8; 32] {
        let mut out = [0u8; 32];
        match self {
            Val::U32(value) => out[28..].copy_from_slice(&value.to_be_bytes()),
            Val::U64(value) => out[24..].copy_from_slice(&value.to_be_bytes()),
            Val::U256(value) => out.copy_from_slice(&value),
            Val::Addr(value) => out[12..].copy_from_slice(&value),
            Val::B32(value) => out.copy_from_slice(&value),
            Val::Str(_) => panic!("dynamic value has no static ABI word"),
        }
        out
    }
}

pub fn encode(values: &[Val]) -> Vec<u8> {
    let head_size = 32 * values.len();
    let mut head = Vec::with_capacity(head_size);
    let mut tail = Vec::new();

    for value in values {
        if value.is_dynamic() {
            head.extend_from_slice(&word_u64((head_size + tail.len()) as u64));
            if let Val::Str(text) = value {
                tail.extend_from_slice(&word_u64(text.len() as u64));
                let mut bytes = text.as_bytes().to_vec();
                let rem = bytes.len() % 32;
                if rem != 0 {
                    bytes.resize(bytes.len() + 32 - rem, 0);
                }
                tail.extend_from_slice(&bytes);
            }
        } else {
            head.extend_from_slice(&value.word());
        }
    }

    head.extend_from_slice(&tail);
    head
}

pub fn pack_words(values: &[Val]) -> Vec<u8> {
    let mut out = Vec::with_capacity(32 * values.len());
    for value in values {
        out.extend_from_slice(&value.word());
    }
    out
}

fn word_u64(value: u64) -> [u8; 32] {
    let mut out = [0u8; 32];
    out[24..].copy_from_slice(&value.to_be_bytes());
    out
}
