//! Minimal Solidity `abi.encode` for the static types the vault recomputes, plus
//! the single dynamic `string` domain literal used inside `configHash`/`lockDigest`
//! (ZK_BACK3 §2.3, §3). `pack_words` is the non-framed concatenation used for the
//! "fixed-width ABI encodings" of `returnRoot`/`lockRefRoot` (ZK_BACK3 §5.2).
//!
//! NOTE (M0): the leading domain is encoded as a dynamic ABI `string` to match
//! ZK_BACK3's `abi.encode("unicity-...:v1", ...)`. If the contract team prefers a
//! `bytes32` domain constant (all-static encoding, simpler to recompute), change
//! it here and in the contracts together and bump BRIDGE_PROTO_VERSION.

#[derive(Clone)]
pub enum Val {
    /// Dynamic ABI string (the domain literal).
    Str(&'static str),
    U32(u32),
    U64(u64),
    U256([u8; 32]),
    Addr([u8; 20]),
    B32([u8; 32]),
}

impl Val {
    fn is_dynamic(&self) -> bool {
        matches!(self, Val::Str(_))
    }

    /// The 32-byte head word for a static value (Solidity left-pads numerics and
    /// addresses; `bytes32` occupies the whole word).
    fn word(&self) -> [u8; 32] {
        let mut w = [0u8; 32];
        match self {
            Val::U32(n) => w[28..].copy_from_slice(&n.to_be_bytes()),
            Val::U64(n) => w[24..].copy_from_slice(&n.to_be_bytes()),
            Val::U256(b) => w.copy_from_slice(b),
            Val::Addr(a) => w[12..].copy_from_slice(a),
            Val::B32(b) => w.copy_from_slice(b),
            Val::Str(_) => panic!("dynamic value has no head word"),
        }
        w
    }
}

/// Solidity `abi.encode(vals...)` with proper head/tail framing.
pub fn encode(vals: &[Val]) -> Vec<u8> {
    let head_size = 32 * vals.len();
    let mut head = Vec::with_capacity(head_size);
    let mut tail = Vec::new();
    for v in vals {
        if v.is_dynamic() {
            let offset = head_size + tail.len();
            head.extend_from_slice(&word_u64(offset as u64));
            if let Val::Str(s) = v {
                tail.extend_from_slice(&word_u64(s.len() as u64));
                let mut data = s.as_bytes().to_vec();
                let rem = data.len() % 32;
                if rem != 0 {
                    data.resize(data.len() + (32 - rem), 0);
                }
                tail.extend_from_slice(&data);
            }
        } else {
            head.extend_from_slice(&v.word());
        }
    }
    head.extend_from_slice(&tail);
    head
}

/// Concatenated fixed-width 32-byte words, no array framing. The on-chain vault
/// recomputes `returnRoot`/`lockRefRoot` as `keccak256` over exactly this layout.
pub fn pack_words(vals: &[Val]) -> Vec<u8> {
    let mut out = Vec::with_capacity(32 * vals.len());
    for v in vals {
        out.extend_from_slice(&v.word());
    }
    out
}

fn word_u64(n: u64) -> [u8; 32] {
    let mut w = [0u8; 32];
    w[24..].copy_from_slice(&n.to_be_bytes());
    w
}
