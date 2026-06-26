use bridge_return_core::{
    burn_transition_id, config_hash, domain_tag, lock_ref_root, nullifier, public_values_abi,
    reason_cbor, return_root, sum_amounts, BridgeBackReason, BridgeConfig, PublicValues,
    ReturnLeaf, SourceLockRef,
};
use bridge_return_guest::{execute, BridgeBurnWitness, GuestInput, RelationWitness};
use num_bigint::BigUint;
use serde_json::json;
use unicity_token::accumulator::{
    ordered_insert_witnesses, NonMembershipTerminal, NonMembershipWitness, NullifierTree,
};
use unicity_token::api::bft::{
    InputRecord, RootTrustBase, RootTrustBaseNodeInfo, ShardId, ShardTreeCertificate,
    UnicityCertificate, UnicitySeal, UnicityTreeCertificate,
};
use unicity_token::api::{CertificationData, InclusionCertificate, InclusionProof, NetworkId};
use unicity_token::bridge::{
    bridge_lock_obligation, BridgeConfig as SdkBridgeConfig, TRON_USDT_LOCK_JUSTIFICATION_TAG,
};
use unicity_token::cbor::{encode_array, encode_byte_string, encode_tag, encode_uint};
use unicity_token::crypto::hash::{sha256, DataHash};
use unicity_token::crypto::signer::{Secp256k1Signer, Signer};
use unicity_token::payment::{Asset, AssetId, PaymentAssetCollection};
use unicity_token::predicate::builtin::{BurnPredicate, SignaturePredicate};
use unicity_token::predicate::unlock::sign_signature_unlock;
use unicity_token::predicate::EncodedPredicate;
use unicity_token::transaction::ids::{TokenSalt, TokenType};
use unicity_token::transaction::{
    CertifiedMintTransaction, CertifiedTransferTransaction, MintTransaction, Minter, Token,
    Transaction, TransferTransaction,
};

fn signer(byte: u8) -> Secp256k1Signer {
    Secp256k1Signer::from_bytes(&[byte; 32]).unwrap()
}

fn signature_lock(signer: &Secp256k1Signer) -> EncodedPredicate {
    SignaturePredicate::new(signer.public_key()).to_encoded()
}

fn trust_base(node: &Secp256k1Signer) -> RootTrustBase {
    RootTrustBase::new(
        0,
        NetworkId::LOCAL,
        0,
        0,
        vec![RootTrustBaseNodeInfo {
            node_id: "NODE".to_string(),
            signing_key: node.public_key(),
            stake: 1,
        }],
        1,
    )
}

fn core_config() -> BridgeConfig {
    BridgeConfig {
        source_chain_id: 728126428,
        vault: h20("00000000000000000000000000000000000000a1"),
        asset: h20("a614f803b6fd780986a42c78ec9c7f77e6ded13c"),
        token_type: h32("fd58cc8c3a8f61465cc6cef34bac939a8df0a2126884f017f0a1054c72a9161e"),
        coin_id: h32("16fb6597bb3233902232a7aa6ee54f41e45014ffc4927ee63e8710823638d20b"),
        reason_tag: 39_050,
        lock_domain: h32("158b847f78b3910a5f5f42820de61abba1bf5ae1fbb29dabfba09118f393f932"),
        nullifier_domain: h32("d4530e4ea58fc8e38f84506e62b421476c3eeec70f4cbebefc32688a510e2d5d"),
    }
}

fn h20(hex: &str) -> [u8; 20] {
    hex::decode(hex).unwrap().try_into().unwrap()
}

fn h32(hex: &str) -> [u8; 32] {
    hex::decode(hex).unwrap().try_into().unwrap()
}

fn sdk_config(config: &BridgeConfig) -> SdkBridgeConfig {
    SdkBridgeConfig {
        source_chain_id: config.source_chain_id,
        vault: config.vault,
        asset: config.asset,
        token_type: config.token_type,
        coin_id: config.coin_id,
    }
}

fn lock_justification(config: &BridgeConfig, amount: u64, nonce: u64) -> Vec<u8> {
    encode_tag(
        TRON_USDT_LOCK_JUSTIFICATION_TAG,
        &encode_array(&[
            &encode_uint(1),
            &encode_uint(config.source_chain_id),
            &encode_byte_string(&config.vault),
            &encode_byte_string(&config.asset),
            &encode_byte_string(&[0x77; 32]),
            &encode_uint(3),
            &encode_uint(amount),
            &encode_uint(nonce),
        ]),
    )
}

fn bridge_mint(
    config: &BridgeConfig,
    owner: &Secp256k1Signer,
    amount: u64,
    nonce: u64,
) -> MintTransaction {
    let assets = PaymentAssetCollection::create([Asset::new(
        AssetId::new(config.coin_id.to_vec()),
        BigUint::from(amount),
    )])
    .unwrap();
    MintTransaction::create(
        NetworkId::LOCAL,
        signature_lock(owner),
        TokenType::new(config.token_type.to_vec()),
        TokenSalt::from_bytes([0x42; 32]),
        Some(assets.to_cbor()),
        Some(lock_justification(config, amount, nonce)),
    )
    .unwrap()
}

fn bridge_burn(mint: &MintTransaction, reason_bytes: Vec<u8>) -> TransferTransaction {
    TransferTransaction::new(
        mint.calculate_state_hash(),
        mint.recipient().clone(),
        BurnPredicate::new(sha256(&reason_bytes).data().to_vec()).to_encoded(),
        vec![0x99; 32],
        Some(reason_bytes),
    )
}

fn leaf_hash(state_id: &[u8; 32], tx_hash: &DataHash) -> [u8; 32] {
    let mut preimage = vec![0x00];
    preimage.extend_from_slice(state_id);
    preimage.extend_from_slice(tx_hash.data());
    digest(&preimage)
}

fn node_hash(depth: u8, left: &[u8; 32], right: &[u8; 32]) -> [u8; 32] {
    let mut preimage = vec![0x01, depth];
    preimage.extend_from_slice(left);
    preimage.extend_from_slice(right);
    digest(&preimage)
}

fn digest(bytes: &[u8]) -> [u8; 32] {
    sha256(bytes).data().try_into().unwrap()
}

fn bit_at(key: &[u8; 32], depth: usize) -> bool {
    (key[depth / 8] >> (depth % 8)) & 1 == 1
}

fn one_sibling_path(
    state_id: &[u8; 32],
    tx_hash: &DataHash,
    sibling_state_id: &[u8; 32],
    sibling_tx_hash: &DataHash,
) -> (InclusionCertificate, [u8; 32]) {
    let depth = (0..=255)
        .rev()
        .find(|depth| bit_at(state_id, *depth) != bit_at(sibling_state_id, *depth))
        .unwrap();
    let leaf = leaf_hash(state_id, tx_hash);
    let sibling = leaf_hash(sibling_state_id, sibling_tx_hash);
    let root = if bit_at(state_id, depth) {
        node_hash(depth as u8, &sibling, &leaf)
    } else {
        node_hash(depth as u8, &leaf, &sibling)
    };
    let mut raw = vec![0u8; 32];
    raw[depth / 8] |= 1 << (depth % 8);
    raw.extend_from_slice(&sibling);
    (InclusionCertificate::decode(&raw).unwrap(), root)
}

fn empty_shard() -> ShardId {
    ShardId::decode(&[0b1000_0000]).unwrap()
}

fn mk_seal(hash: Vec<u8>, signatures: Vec<(String, Vec<u8>)>) -> UnicitySeal {
    UnicitySeal {
        network_id: NetworkId::LOCAL,
        root_chain_round_number: 0,
        epoch: 0,
        timestamp: 0,
        previous_hash: None,
        hash,
        signatures,
    }
}

fn signed_uc(node: &Secp256k1Signer, root: [u8; 32]) -> UnicityCertificate {
    let input_record = InputRecord {
        round_number: 0,
        epoch: 0,
        previous_hash: None,
        hash: root.to_vec(),
        summary_value: Vec::new(),
        timestamp: 0,
        block_hash: None,
        sum_of_earned_fees: 0,
        executed_transactions_hash: None,
    };
    let mut uc = UnicityCertificate {
        input_record,
        technical_record_hash: None,
        shard_configuration_hash: vec![0u8; 32],
        shard_tree_certificate: ShardTreeCertificate {
            shard: empty_shard(),
            sibling_hash_list: Vec::new(),
        },
        unicity_tree_certificate: UnicityTreeCertificate {
            partition_identifier: 0,
            steps: Vec::new(),
        },
        unicity_seal: mk_seal(vec![0u8; 32], Vec::new()),
    };
    let seal_hash = uc.computed_seal_hash().unwrap().data().to_vec();
    let signature = node
        .sign(&mk_seal(seal_hash.clone(), Vec::new()).calculate_hash())
        .encode()
        .to_vec();
    uc.unicity_seal = mk_seal(seal_hash, vec![("NODE".to_string(), signature)]);
    uc
}

fn proof(
    transaction: &impl Transaction,
    owner: &Secp256k1Signer,
    certificate: InclusionCertificate,
    uc: UnicityCertificate,
) -> InclusionProof {
    let tx_hash = transaction.calculate_transaction_hash();
    let unlock = sign_signature_unlock(owner, transaction.source_state_hash(), &tx_hash);
    InclusionProof {
        certification_data: Some(CertificationData::new(
            transaction.lock_script().clone(),
            transaction.source_state_hash().clone(),
            tx_hash,
            unlock,
        )),
        inclusion_certificate: Some(certificate),
        unicity_certificate: uc,
    }
}

#[test]
fn guest_executes_direct_bridge_burn_b1() {
    let config = core_config();
    let amount = 1_000_000;
    let nonce = 7;
    let owner = signer(0x22);
    let node = signer(0x11);
    let trust_base = trust_base(&node);

    let mint = bridge_mint(&config, &owner, amount, nonce);
    let reason = BridgeBackReason {
        version: 1,
        recipient: [0xB2; 20],
        amount: bridge_return_core::u256_from_u64(amount),
        fee_recipient: [0xC3; 20],
        fee_amount: bridge_return_core::u256_from_u64(1_000),
        deadline: 1_900_000_000,
    };
    let reason_bytes = reason_cbor(&config, &reason);
    let burn = bridge_burn(&mint, reason_bytes);

    let mint_state_id =
        unicity_token::api::StateId::derive(mint.lock_script(), mint.source_state_hash());
    let burn_state_id =
        unicity_token::api::StateId::derive(burn.lock_script(), burn.source_state_hash());
    let (mint_path, root) = one_sibling_path(
        mint_state_id.bytes(),
        &mint.calculate_transaction_hash(),
        burn_state_id.bytes(),
        &burn.calculate_transaction_hash(),
    );
    let (burn_path, burn_root) = one_sibling_path(
        burn_state_id.bytes(),
        &burn.calculate_transaction_hash(),
        mint_state_id.bytes(),
        &mint.calculate_transaction_hash(),
    );
    assert_eq!(root, burn_root);
    let anchor = signed_uc(&node, root);

    let minter = Minter::signer(mint.token_id()).unwrap();
    let token = Token::new(
        CertifiedMintTransaction::new(
            mint.clone(),
            proof(&mint, &minter, mint_path, anchor.clone()),
        ),
        vec![CertifiedTransferTransaction::new(
            burn.clone(),
            proof(&burn, &owner, burn_path, anchor.clone()),
        )],
    );

    let cfg_hash = config_hash(&config);
    let burn_tx_hash: [u8; 32] = burn.calculate_transaction_hash().data().try_into().unwrap();
    let burn_id = burn_transition_id(burn_state_id.bytes(), &burn_tx_hash);
    let leaf = ReturnLeaf {
        nullifier: nullifier(&cfg_hash, &burn_id),
        recipient: reason.recipient,
        amount: reason.amount,
        fee_recipient: reason.fee_recipient,
        fee_amount: reason.fee_amount,
        deadline: reason.deadline,
    };
    let tree = NullifierTree::new();
    let (accumulator_witnesses, spent_root_new) =
        ordered_insert_witnesses(&tree, &[leaf.nullifier]).unwrap();
    let obligation = bridge_lock_obligation(
        token.genesis(),
        TRON_USDT_LOCK_JUSTIFICATION_TAG,
        &sdk_config(&config),
        PaymentAssetCollection::from_cbor_bytes,
    )
    .unwrap();
    let lock_refs = vec![SourceLockRef {
        nonce: obligation.nonce,
        digest: obligation.digest,
    }];
    let leaves = vec![leaf];
    let public_values = PublicValues {
        domain_tag: domain_tag(),
        config_hash: cfg_hash,
        trust_base_hash: trust_base.canonical_hash(),
        spent_root_old: tree.root(),
        spent_root_new,
        return_root: return_root(&leaves),
        lock_ref_root: lock_ref_root(&lock_refs).unwrap(),
        batch_size: 1,
        total_amount: sum_amounts(&leaves),
    };
    let input = GuestInput {
        config,
        public_values,
        return_leaves: leaves,
        sorted_lock_refs: lock_refs,
        witness: RelationWitness {
            accumulator_witnesses: accumulator_witnesses.clone(),
            bridge_burns: vec![BridgeBurnWitness {
                token,
                trust_base,
                anchor_certificate: anchor.clone(),
                lock_justification_tag: TRON_USDT_LOCK_JUSTIFICATION_TAG,
            }],
        },
    };

    if std::env::var_os("BRIDGE_PRINT_B1_TOKEN_VECTOR").is_some() {
        println!(
            "{}",
            serde_json::to_string_pretty(&fixture_json(&input, &anchor, &accumulator_witnesses,))
                .unwrap()
        );
    }

    assert_eq!(execute(&input), Ok(public_values));
}

fn fixture_json(
    input: &GuestInput,
    anchor: &UnicityCertificate,
    accumulator_witnesses: &[NonMembershipWitness],
) -> serde_json::Value {
    let burn = &input.witness.bridge_burns[0];
    json!({
        "description": "B=1 direct bridge-lock token burned to BridgeBackReason; full guest relation execute vector",
        "in": {
            "token_cbor": hex0(&burn.token.to_cbor()),
            "trust_base": trust_base_json(&burn.trust_base),
            "anchor_certificate_cbor": hex0(&anchor.to_cbor()),
            "lock_justification_tag": burn.lock_justification_tag,
            "leaves": input.return_leaves.iter().map(return_leaf_json).collect::<Vec<_>>(),
            "lock_refs": input.sorted_lock_refs.iter().map(lock_ref_json).collect::<Vec<_>>(),
            "accumulator_witnesses": accumulator_witnesses.iter().map(witness_json).collect::<Vec<_>>(),
        },
        "out": public_values_json(&input.public_values),
    })
}

fn trust_base_json(trust_base: &RootTrustBase) -> serde_json::Value {
    json!({
        "version": trust_base.version,
        "network_id": trust_base.network_id.id(),
        "epoch": trust_base.epoch,
        "epoch_start_round": trust_base.epoch_start_round,
        "quorum_threshold": trust_base.quorum_threshold,
        "root_nodes": trust_base.root_nodes.iter().map(|node| json!({
            "node_id": node.node_id,
            "signing_key": hex0(node.signing_key.as_bytes()),
            "stake": node.stake,
        })).collect::<Vec<_>>(),
    })
}

fn return_leaf_json(leaf: &ReturnLeaf) -> serde_json::Value {
    json!({
        "nullifier": hex0(&leaf.nullifier),
        "recipient": hex0(&leaf.recipient),
        "amount": hex0(&leaf.amount),
        "fee_recipient": hex0(&leaf.fee_recipient),
        "fee_amount": hex0(&leaf.fee_amount),
        "deadline": leaf.deadline,
    })
}

fn lock_ref_json(lock_ref: &SourceLockRef) -> serde_json::Value {
    json!({
        "nonce": lock_ref.nonce,
        "digest": hex0(&lock_ref.digest),
    })
}

fn witness_json(witness: &NonMembershipWitness) -> serde_json::Value {
    let terminal = match witness.terminal() {
        NonMembershipTerminal::Empty => json!({ "kind": "empty" }),
        NonMembershipTerminal::Occupied { key } => json!({
            "kind": "occupied",
            "key": hex0(key),
        }),
    };
    json!({
        "terminal": terminal,
        "steps": witness.steps().iter().map(|step| json!({
            "depth": step.depth(),
            "sibling_hash": hex0(step.sibling_hash()),
        })).collect::<Vec<_>>(),
    })
}

fn public_values_json(public_values: &PublicValues) -> serde_json::Value {
    json!({
        "domain_tag": hex0(&public_values.domain_tag),
        "config_hash": hex0(&public_values.config_hash),
        "trust_base_hash": hex0(&public_values.trust_base_hash),
        "spent_root_old": hex0(&public_values.spent_root_old),
        "spent_root_new": hex0(&public_values.spent_root_new),
        "return_root": hex0(&public_values.return_root),
        "lock_ref_root": hex0(&public_values.lock_ref_root),
        "batch_size": public_values.batch_size,
        "total_amount": hex0(&public_values.total_amount),
        "public_values_abi": hex0(&public_values_abi(public_values)),
    })
}

fn hex0(bytes: &[u8]) -> String {
    format!("0x{}", hex::encode(bytes))
}
