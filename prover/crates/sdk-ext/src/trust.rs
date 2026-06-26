use alloc::vec::Vec;

use unicity_token::api::bft::RootTrustBase;
use unicity_token::cbor::{encode_array, encode_byte_string, encode_text_string, encode_uint};
use unicity_token::crypto::hash::sha256;

/// SHA-256 over the canonical CBOR trust-base document committed by
/// `PublicValues.trustBaseHash`.
pub fn canonical_hash(trust_base: &RootTrustBase) -> [u8; 32] {
    let nodes = trust_base
        .root_nodes
        .iter()
        .map(|node| {
            encode_array(&[
                &encode_text_string(&node.node_id),
                &encode_byte_string(node.signing_key.as_bytes()),
                &encode_uint(node.stake),
            ])
        })
        .collect::<Vec<_>>();
    let node_refs = nodes.iter().map(Vec::as_slice).collect::<Vec<_>>();
    let encoded_nodes = encode_array(&node_refs);
    let encoded = encode_array(&[
        &encode_uint(trust_base.version),
        &encode_uint(trust_base.network_id.id().into()),
        &encode_uint(trust_base.epoch),
        &encode_uint(trust_base.epoch_start_round),
        &encoded_nodes,
        &encode_uint(trust_base.quorum_threshold),
    ]);
    let mut out = [0u8; 32];
    out.copy_from_slice(sha256(&encoded).data());
    out
}
