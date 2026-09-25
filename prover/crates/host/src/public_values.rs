use bridge_return_core::{Bytes32, PublicValues};

pub fn public_values_from_abi(bytes: &[u8]) -> Option<PublicValues> {
    if bytes.len() < 9 * 32 {
        return None;
    }
    let word = |i: usize| -> Bytes32 { bytes[i * 32..(i + 1) * 32].try_into().unwrap() };
    let size = word(7);
    if size[..28].iter().any(|b| *b != 0) {
        return None;
    }
    Some(PublicValues {
        domain_tag: word(0),
        config_hash: word(1),
        trust_base_hash: word(2),
        spent_root_old: word(3),
        spent_root_new: word(4),
        return_root: word(5),
        lock_ref_root: word(6),
        batch_size: u32::from_be_bytes(size[28..].try_into().unwrap()),
        total_amount: word(8),
    })
}
