use bridge_return_core::{public_values_abi, public_values_digest, PublicValues};
use bridge_return_host::public_values::public_values_from_abi;

fn sample() -> PublicValues {
    PublicValues {
        domain_tag: [0xd; 32],
        config_hash: [0xc; 32],
        trust_base_hash: [1; 32],
        spent_root_old: [2; 32],
        spent_root_new: [3; 32],
        return_root: [4; 32],
        lock_ref_root: [5; 32],
        batch_size: 7,
        total_amount: [9; 32],
    }
}

#[test]
fn decodes_what_the_guest_encodes() {
    let values = sample();
    assert_eq!(
        public_values_from_abi(&public_values_abi(&values)),
        Some(values)
    );
}

#[test]
fn tolerates_the_digest_the_relayer_appends() {
    let values = sample();
    let mut bytes = public_values_abi(&values);
    bytes.extend_from_slice(&public_values_digest(&values));
    assert_eq!(public_values_from_abi(&bytes), Some(values));
}

#[test]
fn refuses_short_input_and_an_oversized_batch_size() {
    let mut bytes = public_values_abi(&sample());
    assert_eq!(public_values_from_abi(&bytes[..bytes.len() - 1]), None);
    bytes[7 * 32] = 1;
    assert_eq!(public_values_from_abi(&bytes), None);
}
