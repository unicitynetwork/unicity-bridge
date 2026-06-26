use bridge_return_core::{
    burn_transition_id, config_hash, domain_tag, lock_ref_root, nullifier, public_values_abi,
    reason_cbor, return_root, sum_amounts, BridgeBackReason, BridgeConfig, PublicValues,
    ReturnLeaf, SourceLockRef,
};
use bridge_return_guest::{wire, BridgeBurnWitness, GuestInput, RelationWitness};
use bridge_return_sdk_ext::accumulator::{
    ordered_insert_witnesses, NonMembershipTerminal, NonMembershipWitness, NullifierTree,
};
use bridge_return_sdk_ext::bridge::{
    bridge_lock_obligation, BridgeConfig as SdkBridgeConfig, TRON_USDT_LOCK_JUSTIFICATION_TAG,
};
use bridge_return_sdk_ext::trust::canonical_hash;
use num_bigint::BigUint;
use serde_json::json;
use unicity_token::api::bft::{
    InputRecord, RootTrustBase, RootTrustBaseNodeInfo, ShardId, ShardTreeCertificate,
    UnicityCertificate, UnicitySeal, UnicityTreeCertificate,
};
use unicity_token::api::{CertificationData, InclusionCertificate, InclusionProof, NetworkId};
use unicity_token::cbor::{encode_array, encode_byte_string, encode_tag, encode_uint};
use unicity_token::crypto::hash::{sha256, DataHash};
use unicity_token::crypto::signer::{Secp256k1Signer, Signer};
use unicity_token::payment::{
    Asset, AssetId, PaymentAssetCollection, SplitMintJustification, SplitTokenRequest, TokenSplit,
};
use unicity_token::predicate::builtin::{BurnPredicate, SignaturePredicate};
use unicity_token::predicate::unlock::sign_signature_unlock;
use unicity_token::predicate::EncodedPredicate;
use unicity_token::transaction::ids::{TokenSalt, TokenType};
use unicity_token::transaction::{
    CertifiedMintTransaction, CertifiedTransferTransaction, MintTransaction, Minter, Transaction,
    TransferTransaction,
};

pub struct B1Fixture {
    pub input: GuestInput,
    pub anchor_certificate: UnicityCertificate,
    pub accumulator_witnesses: Vec<NonMembershipWitness>,
}

pub struct SplitFixture {
    pub input: GuestInput,
    pub anchor_certificate: UnicityCertificate,
    pub accumulator_witnesses: Vec<NonMembershipWitness>,
}

pub fn build_b1_direct_bridge_fixture() -> B1Fixture {
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
    let token = unicity_token::transaction::Token::new(
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
        trust_base_hash: canonical_hash(&trust_base),
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

    B1Fixture {
        input,
        anchor_certificate: anchor,
        accumulator_witnesses,
    }
}

pub fn b1_fixture_json(fixture: &B1Fixture) -> serde_json::Value {
    fixture_json(
        "B=1 direct bridge-lock token burned to BridgeBackReason; full guest relation execute vector",
        &fixture.input,
        &fixture.anchor_certificate,
        &fixture.accumulator_witnesses,
    )
}

pub fn build_split_bridge_fixture() -> SplitFixture {
    let config = core_config();
    let source_amount = 1_000_000;
    let output_amount = 600_000;
    let nonce = 7;
    let owner = signer(0x22);
    let output_owner = signer(0x33);
    let change_owner = signer(0x44);
    let node = signer(0x11);
    let trust_base = trust_base(&node);

    let source_mint = bridge_mint_with_salt(&config, &owner, source_amount, nonce, [0x42; 32]);
    let source_token_for_split = token_with_single_mint(&source_mint, &node);
    let output_assets = PaymentAssetCollection::create([Asset::new(
        AssetId::new(config.coin_id.to_vec()),
        BigUint::from(output_amount),
    )])
    .unwrap();
    let change_assets = PaymentAssetCollection::create([Asset::new(
        AssetId::new(config.coin_id.to_vec()),
        BigUint::from(source_amount - output_amount),
    )])
    .unwrap();
    let split = TokenSplit::split_unchecked(
        &source_token_for_split,
        PaymentAssetCollection::from_cbor_bytes,
        vec![
            SplitTokenRequest::create(
                signature_lock(&output_owner),
                output_assets,
                TokenType::new(config.token_type.to_vec()),
                TokenSalt::from_bytes([0x51; 32]),
            ),
            SplitTokenRequest::create(
                signature_lock(&change_owner),
                change_assets,
                TokenType::new(config.token_type.to_vec()),
                TokenSalt::from_bytes([0x52; 32]),
            ),
        ],
        Some([0x55; 32]),
    )
    .unwrap();
    let split_output = &split.tokens[0];

    let source_mint_state_id = unicity_token::api::StateId::derive(
        source_mint.lock_script(),
        source_mint.source_state_hash(),
    );
    let split_burn = split.burn.transaction.clone();
    let split_burn_state_id = unicity_token::api::StateId::derive(
        split_burn.lock_script(),
        split_burn.source_state_hash(),
    );
    let (source_mint_path, source_root) = one_sibling_path(
        source_mint_state_id.bytes(),
        &source_mint.calculate_transaction_hash(),
        split_burn_state_id.bytes(),
        &split_burn.calculate_transaction_hash(),
    );
    let (split_burn_path, split_root) = one_sibling_path(
        split_burn_state_id.bytes(),
        &split_burn.calculate_transaction_hash(),
        source_mint_state_id.bytes(),
        &source_mint.calculate_transaction_hash(),
    );
    assert_eq!(source_root, split_root);
    let source_anchor = signed_uc(&node, source_root);
    let source_minter = Minter::signer(source_mint.token_id()).unwrap();
    let burned_source = unicity_token::transaction::Token::new(
        CertifiedMintTransaction::new(
            source_mint.clone(),
            proof(
                &source_mint,
                &source_minter,
                source_mint_path,
                source_anchor.clone(),
            ),
        ),
        vec![CertifiedTransferTransaction::new(
            split_burn.clone(),
            proof(&split_burn, &owner, split_burn_path, source_anchor),
        )],
    );

    let split_justification =
        SplitMintJustification::create(burned_source, split_output.proofs.clone()).unwrap();
    let output_mint = MintTransaction::create(
        split_output.network_id,
        split_output.recipient.clone(),
        split_output.token_type.clone(),
        split_output.salt.clone(),
        Some(split_output.assets.to_cbor()),
        Some(split_justification.to_cbor()),
    )
    .unwrap();
    let reason = BridgeBackReason {
        version: 1,
        recipient: [0xB2; 20],
        amount: bridge_return_core::u256_from_u64(output_amount),
        fee_recipient: [0xC3; 20],
        fee_amount: bridge_return_core::u256_from_u64(1_000),
        deadline: 1_900_000_000,
    };
    let reason_bytes = reason_cbor(&config, &reason);
    let return_burn = bridge_burn(&output_mint, reason_bytes);

    let output_mint_state_id = unicity_token::api::StateId::derive(
        output_mint.lock_script(),
        output_mint.source_state_hash(),
    );
    let return_burn_state_id = unicity_token::api::StateId::derive(
        return_burn.lock_script(),
        return_burn.source_state_hash(),
    );
    let (output_mint_path, output_root) = one_sibling_path(
        output_mint_state_id.bytes(),
        &output_mint.calculate_transaction_hash(),
        return_burn_state_id.bytes(),
        &return_burn.calculate_transaction_hash(),
    );
    let (return_burn_path, return_root_hash) = one_sibling_path(
        return_burn_state_id.bytes(),
        &return_burn.calculate_transaction_hash(),
        output_mint_state_id.bytes(),
        &output_mint.calculate_transaction_hash(),
    );
    assert_eq!(output_root, return_root_hash);
    let anchor = signed_uc(&node, output_root);

    let output_minter = Minter::signer(output_mint.token_id()).unwrap();
    let token = unicity_token::transaction::Token::new(
        CertifiedMintTransaction::new(
            output_mint.clone(),
            proof(
                &output_mint,
                &output_minter,
                output_mint_path,
                anchor.clone(),
            ),
        ),
        vec![CertifiedTransferTransaction::new(
            return_burn.clone(),
            proof(
                &return_burn,
                &output_owner,
                return_burn_path,
                anchor.clone(),
            ),
        )],
    );

    let cfg_hash = config_hash(&config);
    let burn_tx_hash: [u8; 32] = return_burn
        .calculate_transaction_hash()
        .data()
        .try_into()
        .unwrap();
    let burn_id = burn_transition_id(return_burn_state_id.bytes(), &burn_tx_hash);
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
        token
            .genesis()
            .transaction()
            .justification()
            .and_then(|_| {
                SplitMintJustification::from_cbor(token.genesis().transaction().justification()?)
                    .ok()
            })
            .expect("split justification")
            .token()
            .genesis(),
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
        trust_base_hash: canonical_hash(&trust_base),
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

    SplitFixture {
        input,
        anchor_certificate: anchor,
        accumulator_witnesses,
    }
}

pub fn split_fixture_json(fixture: &SplitFixture) -> serde_json::Value {
    fixture_json(
        "B=1 split bridge token burned to BridgeBackReason; exercises recursive source-lock extraction",
        &fixture.input,
        &fixture.anchor_certificate,
        &fixture.accumulator_witnesses,
    )
}

fn fixture_json(
    description: &str,
    input: &GuestInput,
    anchor_certificate: &UnicityCertificate,
    accumulator_witnesses: &[NonMembershipWitness],
) -> serde_json::Value {
    let burn = &input.witness.bridge_burns[0];
    json!({
        "description": description,
        "in": {
            "guest_wire_input": hex0(&wire::encode_guest_input(input)),
            "token_cbor": hex0(&burn.token.to_cbor()),
            "trust_base": trust_base_json(&burn.trust_base),
            "anchor_certificate_cbor": hex0(&anchor_certificate.to_cbor()),
            "lock_justification_tag": burn.lock_justification_tag,
            "leaves": input.return_leaves.iter().map(return_leaf_json).collect::<Vec<_>>(),
            "lock_refs": input.sorted_lock_refs.iter().map(lock_ref_json).collect::<Vec<_>>(),
            "accumulator_witnesses": accumulator_witnesses.iter().map(witness_json).collect::<Vec<_>>(),
        },
        "out": public_values_json(&input.public_values),
    })
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
    bridge_mint_with_salt(config, owner, amount, nonce, [0x42; 32])
}

fn bridge_mint_with_salt(
    config: &BridgeConfig,
    owner: &Secp256k1Signer,
    amount: u64,
    nonce: u64,
    salt: [u8; 32],
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
        TokenSalt::from_bytes(salt),
        Some(assets.to_cbor()),
        Some(lock_justification(config, amount, nonce)),
    )
    .unwrap()
}

fn token_with_single_mint(
    mint: &MintTransaction,
    node: &Secp256k1Signer,
) -> unicity_token::transaction::Token {
    let minter = Minter::signer(mint.token_id()).unwrap();
    let state_id =
        unicity_token::api::StateId::derive(mint.lock_script(), mint.source_state_hash());
    let (path, root) = one_sibling_path(
        state_id.bytes(),
        &mint.calculate_transaction_hash(),
        &[0xFF; 32],
        &sha256(&[0xEE]),
    );
    unicity_token::transaction::Token::new(
        CertifiedMintTransaction::new(
            mint.clone(),
            proof(mint, &minter, path, signed_uc(node, root)),
        ),
        Vec::new(),
    )
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
