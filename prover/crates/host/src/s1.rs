//! S1 — host witness package + precheck (ZK_BACK3 §10.1).
//!
//! S1 assembles the witness a return batch needs into a [`WitnessPackage`] and
//! runs a [`WitnessPackage::precheck`] that mirrors the SP1 guest relation
//! natively, so the pipeline fails fast *before* the expensive STARK→Groth16
//! prove. The precheck runs the exact guest entry points (`execute_public_output`
//! + `execute_wire`) — not a re-implementation — and additionally asserts the
//! wire encoding round-trips, catching any drift between the in-memory
//! `GuestInput` and the byte payload the prover consumes.
//!
//! Out of scope here (still open): the live witness *fetch* — decoding burned
//! blobs, choosing an anchor root `R*`, and pulling anchored inclusion proofs
//! over the aggregator's `http` API. This module owns the package shape and the
//! precheck gate those services feed into.

use bridge_return_core::{
    burn_transition_id, config_hash, domain_tag, lock_ref_root, nullifier, return_root,
    sum_amounts, BridgeConfig, PublicValues, ReturnLeaf, SourceLockRef, U256,
};
use bridge_return_guest::{
    decode_bridge_back_reason, execute_public_output, execute_wire, wire, BridgeBurnWitness,
    BurnVerification, GuestInput, RelationWitness,
};
use unicity_token::api::StateId;
use bridge_return_sdk_ext::accumulator::{ordered_insert_witnesses, NullifierTree};
use bridge_return_sdk_ext::bridge::{
    bridge_lock_obligations_for_token_certified, decode_bridged_payment_data,
    BridgeConfig as SdkBridgeConfig,
};
use bridge_return_sdk_ext::trust::canonical_hash;
use unicity_token::api::bft::RootTrustBase;
use unicity_token::transaction::{Token, Transaction};

use crate::{HostError, Result};

#[derive(Clone)]
pub struct CertifiedBurnInput {
    pub token: Token,
    pub trust_base: RootTrustBase,
    pub lock_justification_tag: u64,
    pub leaf: ReturnLeaf,
}

/// Full cryptographic verification of a live, aggregator-**certified** burned
/// token — each transition carries its own `UnicityCertificate`, as served by a
/// real aggregator — returning the source bridge-lock obligations as
/// [`SourceLockRef`]s. This verifies the trust-base quorum + chain linkage +
/// owner authorization (not just the structural byte derivations), so it is the
/// S1 entry point for real testnet tokens.
///
/// The anchored batch path ([`WitnessPackage`]/`bridge_burns`) instead amortizes
/// one quorum check across a shared anchor (§11); it applies once the aggregator
/// serves historical inclusion proofs against one root.
pub fn verify_certified_burn(
    token: &Token,
    trust_base: &RootTrustBase,
    config: &BridgeConfig,
    lock_justification_tag: u64,
) -> Result<Vec<SourceLockRef>> {
    let sdk_config = SdkBridgeConfig {
        source_chain_id: config.source_chain_id,
        vault: config.vault,
        asset: config.asset,
        token_type: config.token_type,
        coin_id: config.coin_id,
    };
    let obligations = bridge_lock_obligations_for_token_certified(
        token,
        trust_base,
        lock_justification_tag,
        &sdk_config,
        decode_bridged_payment_data,
    )
    .map_err(|err| HostError::Check(format!("certified burn verification failed: {err:?}")))?;
    Ok(obligations
        .into_iter()
        .map(|o| SourceLockRef {
            nonce: o.nonce,
            digest: o.digest,
        })
        .collect())
}

/// The assembled witness for one return batch — everything the SP1 guest needs
/// to produce a proof. Wraps the [`GuestInput`] that S3 (the prover) consumes;
/// this is the artifact S1 produces and hands downstream.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WitnessPackage {
    input: GuestInput,
}

/// Result of the host precheck: the public values the guest will commit, the
/// matching ABI bytes + digest, and the exact wire payload
/// (`encode_guest_input`) the prover should be handed.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PrecheckReport {
    pub public_values: PublicValues,
    pub public_values_abi: Vec<u8>,
    pub public_values_digest: [u8; 32],
    pub wire_input: Vec<u8>,
    pub batch_size: u32,
    pub total_amount: U256,
}

impl WitnessPackage {
    pub fn new(input: GuestInput) -> Self {
        Self { input }
    }

    pub fn input(&self) -> &GuestInput {
        &self.input
    }

    pub fn into_input(self) -> GuestInput {
        self.input
    }

    /// The byte payload the SP1 guest reads from stdin.
    pub fn wire_input(&self) -> Vec<u8> {
        wire::encode_guest_input(&self.input)
    }

    /// Run the guest relation natively and confirm the wire encoding round-trips.
    /// This mirrors what `sp1-execute` validates at prove time, but with no SP1
    /// dependency, so it is a cheap fail-fast gate that also runs under plain
    /// `cargo test`.
    pub fn precheck(&self) -> Result<PrecheckReport> {
        // 1. Run the exact guest relation in-memory.
        let output = execute_public_output(&self.input)
            .map_err(|err| HostError::Check(format!("precheck relation rejected: {err:?}")))?;

        // 2. The package's committed public values must equal the computed ones,
        //    so the prover and the vault agree on the same x before proving.
        if output.public_values != self.input.public_values {
            return Err(HostError::Check(
                "precheck: committed public values differ from computed".to_string(),
            ));
        }

        // 3. The wire payload must decode and re-execute to the identical output,
        //    catching any encode/decode drift before the prover consumes it.
        let wire_input = self.wire_input();
        let wire_output = execute_wire(&wire_input)
            .map_err(|err| HostError::Check(format!("precheck wire rejected: {err:?}")))?;
        if wire_output != output {
            return Err(HostError::Check(
                "precheck: wire round-trip diverged from in-memory relation".to_string(),
            ));
        }

        Ok(PrecheckReport {
            batch_size: output.public_values.batch_size,
            total_amount: output.public_values.total_amount,
            public_values: output.public_values,
            public_values_abi: output.public_values_abi,
            public_values_digest: output.public_values_digest,
            wire_input,
        })
    }
}

/// Assemble a B=1 [`GuestInput`] for a single **certified** burn (a real live
/// token, each transition self-certified) so it can run through the guest
/// relation. Verifies the token (collecting its source lock ref), builds the
/// fresh-accumulator transition over the single nullifier, and derives the
/// `PublicValues`. The caller supplies the settlement `leaf` (the relation
/// re-checks it against the token's terminal burn).
pub fn build_certified_guest_input(
    config: BridgeConfig,
    token: Token,
    trust_base: RootTrustBase,
    lock_justification_tag: u64,
    leaf: ReturnLeaf,
) -> Result<GuestInput> {
    build_certified_guest_input_batch(
        config,
        vec![CertifiedBurnInput {
            token,
            trust_base,
            lock_justification_tag,
            leaf,
        }],
    )
}

/// Assemble a multi-burn [`GuestInput`] for **certified** live tokens (each
/// transition carries its own `UnicityCertificate`). This is the batching
/// counterpart to [`build_certified_guest_input`]: it verifies each token, builds
/// one ordered nullifier-accumulator transition over the submitted leaves, sorts
/// source lock refs by nonce, and derives the batch public values.
///
/// The current guest public values commit to one `trust_base_hash`, so all burns
/// in a certified batch must verify against the same trust base.
pub fn build_certified_guest_input_batch(
    config: BridgeConfig,
    burns: Vec<CertifiedBurnInput>,
) -> Result<GuestInput> {
    if burns.is_empty() {
        return Err(HostError::Check(
            "certified guest input batch is empty".to_string(),
        ));
    }

    let mut sorted_lock_refs = Vec::with_capacity(burns.len());
    let trust_base_hash = canonical_hash(&burns[0].trust_base);
    for burn in &burns {
        let burn_trust_base_hash = canonical_hash(&burn.trust_base);
        if burn_trust_base_hash != trust_base_hash {
            return Err(HostError::Check(
                "certified guest input batch mixes trust bases".to_string(),
            ));
        }
        sorted_lock_refs.extend(verify_certified_burn(
            &burn.token,
            &burn.trust_base,
            &config,
            burn.lock_justification_tag,
        )?);
    }
    sorted_lock_refs.sort_by_key(|r| r.nonce);

    let tree = NullifierTree::new();
    let leaves = burns.iter().map(|burn| burn.leaf).collect::<Vec<_>>();
    let nullifiers = leaves.iter().map(|leaf| leaf.nullifier).collect::<Vec<_>>();
    let (accumulator_witnesses, spent_root_new) = ordered_insert_witnesses(&tree, &nullifiers)
        .map_err(|err| HostError::Check(format!("accumulator witness build failed: {err:?}")))?;
    let public_values = PublicValues {
        domain_tag: domain_tag(),
        config_hash: config_hash(&config),
        trust_base_hash,
        spent_root_old: tree.root(),
        spent_root_new,
        return_root: return_root(&leaves),
        lock_ref_root: lock_ref_root(&sorted_lock_refs)
            .map_err(|err| HostError::Check(format!("lock ref root: {err:?}")))?,
        batch_size: u32::try_from(leaves.len())
            .map_err(|_| HostError::Check("certified batch too large".to_string()))?,
        total_amount: sum_amounts(&leaves),
    };
    let bridge_burns = burns
        .into_iter()
        .map(|burn| BridgeBurnWitness {
            token: burn.token,
            trust_base: burn.trust_base,
            verification: BurnVerification::Certified,
            lock_justification_tag: burn.lock_justification_tag,
        })
        .collect();
    Ok(GuestInput {
        config,
        public_values,
        return_leaves: leaves,
        sorted_lock_refs,
        witness: RelationWitness {
            accumulator_witnesses,
            bridge_burns,
        },
    })
}

/// Build a B=1 certified [`GuestInput`] from the wallet's witness **envelope**
/// (`{tokenCbor, configHash, reasonBytes}` — 02 §2c). This is the service intake
/// path: derive the settlement leaf entirely from the burned token + its
/// `reasonBytes` (no leaf is sent over the wire), then hand off to
/// [`build_certified_guest_input`]. The nullifier is recomputed from the token's
/// terminal burn under `config` (00 §5); the recipient/amount/fee/deadline come
/// from decoding `reasonBytes` (00 §4). The relation re-checks both, so a forged
/// leaf cannot pass.
pub fn build_certified_guest_input_from_envelope(
    config: BridgeConfig,
    trust_base: RootTrustBase,
    lock_justification_tag: u64,
    token: Token,
    reason_bytes: &[u8],
) -> Result<GuestInput> {
    let burn = token
        .transactions()
        .last()
        .ok_or_else(|| HostError::Check("token has no terminal burn".to_string()))?
        .transaction();
    let state_id = StateId::derive(burn.lock_script(), burn.source_state_hash());
    let tx_hash: [u8; 32] = burn
        .calculate_transaction_hash()
        .data()
        .try_into()
        .map_err(|_| HostError::Check("terminal burn tx hash is not 32 bytes".to_string()))?;
    let cfg_hash = config_hash(&config);
    let burn_id = burn_transition_id(state_id.bytes(), &tx_hash);
    let null = nullifier(&cfg_hash, &burn_id);

    let reason = decode_bridge_back_reason(config.reason_tag, reason_bytes)
        .map_err(|err| HostError::Check(format!("reasonBytes decode failed: {err:?}")))?;
    let leaf = ReturnLeaf {
        nullifier: null,
        recipient: reason.recipient,
        amount: reason.amount,
        fee_recipient: reason.fee_recipient,
        fee_amount: reason.fee_amount,
        deadline: reason.deadline,
    };
    build_certified_guest_input(config, token, trust_base, lock_justification_tag, leaf)
}

/// A configured intake context: the deployment [`BridgeConfig`] + the trust base
/// the burned token must verify against + the source lock-justification tag. Built
/// once at service startup; turns a wallet witness envelope into the wire input a
/// prover consumes. Keeps `unicity_token` types off the service's surface.
pub struct EnvelopeIntake {
    config: BridgeConfig,
    trust_base: RootTrustBase,
    lock_justification_tag: u64,
}

impl EnvelopeIntake {
    /// Load from a frozen deployment-config JSON (the `config` block of
    /// `deployments/<net>/<asset>.json`, or a bare config object) + a trust
    /// base JSON. `lock_justification_tag` is the source chain's lock tag (Tron
    /// USDT = `1330002`).
    pub fn from_json(
        deployment_json: &str,
        trust_base_json: &str,
        lock_justification_tag: u64,
    ) -> Result<Self> {
        let doc: serde_json::Value = serde_json::from_str(deployment_json)
            .map_err(|e| HostError::Check(format!("deployment config json: {e}")))?;
        let c = doc.get("config").unwrap_or(&doc);
        let config = bridge_config_from_json(c)?;
        let trust_base = RootTrustBase::from_json(trust_base_json)
            .map_err(|e| HostError::Check(format!("trust base json: {e}")))?;
        Ok(Self {
            config,
            trust_base,
            lock_justification_tag,
        })
    }

    /// The deployment `config_hash` this intake binds (cross-check vs the envelope).
    pub fn config_hash(&self) -> [u8; 32] {
        config_hash(&self.config)
    }

    /// Build the certified guest **wire input** from the wallet envelope
    /// (`tokenCbor` + `reasonBytes`). Fully verifies the burned token against the
    /// trust base and derives the settlement leaf (00 §4/§5).
    pub fn build_wire_input(&self, token_cbor: &[u8], reason_bytes: &[u8]) -> Result<Vec<u8>> {
        let token = Token::from_cbor(token_cbor)
            .map_err(|e| HostError::Check(format!("token decode: {e}")))?;
        let input = build_certified_guest_input_from_envelope(
            self.config,
            self.trust_base.clone(),
            self.lock_justification_tag,
            token,
            reason_bytes,
        )?;
        Ok(wire::encode_guest_input(&input))
    }
}

fn bridge_config_from_json(c: &serde_json::Value) -> Result<BridgeConfig> {
    Ok(BridgeConfig {
        source_chain_id: json_u64(c, "source_chain_id")?,
        vault: json_addr(c, "vault")?,
        asset: json_addr(c, "asset")?,
        token_type: json_b32(c, "token_type")?,
        coin_id: json_b32(c, "coin_id")?,
        reason_tag: json_u64(c, "reason_tag")?,
        lock_domain: json_b32(c, "lock_domain")?,
        nullifier_domain: json_b32(c, "nullifier_domain")?,
    })
}

fn json_u64(c: &serde_json::Value, key: &str) -> Result<u64> {
    c[key]
        .as_u64()
        .ok_or_else(|| HostError::Check(format!("deployment config: missing/invalid u64 `{key}`")))
}

fn json_hex<const N: usize>(c: &serde_json::Value, key: &str) -> Result<[u8; N]> {
    let s = c[key]
        .as_str()
        .ok_or_else(|| HostError::Check(format!("deployment config: missing string `{key}`")))?;
    let bytes = hex::decode(s.strip_prefix("0x").unwrap_or(s))
        .map_err(|e| HostError::Check(format!("deployment config `{key}` hex: {e}")))?;
    bytes
        .try_into()
        .map_err(|_| HostError::Check(format!("deployment config `{key}` wrong length")))
}

fn json_addr(c: &serde_json::Value, key: &str) -> Result<[u8; 20]> {
    json_hex::<20>(c, key)
}
fn json_b32(c: &serde_json::Value, key: &str) -> Result<[u8; 32]> {
    json_hex::<32>(c, key)
}

/// Decode a wire payload into a [`WitnessPackage`] and precheck it. Useful as a
/// standalone fail-fast gate over the exact bytes a prover would receive.
pub fn precheck_wire(wire_input: &[u8]) -> Result<PrecheckReport> {
    let input = wire::decode_guest_input(wire_input)
        .map_err(|err| HostError::Check(format!("wire decode: {err:?}")))?;
    WitnessPackage::new(input).precheck()
}

/// Live aggregator/gateway fetch for S1 (ZK_BACK3 §10.1). Talks to the Unicity
/// gateway over the SDK's blocking JSON-RPC client to pull the inclusion proofs
/// a witness needs. Enabled by the `http` feature (pulls a TLS stack).
#[cfg(feature = "http")]
pub mod aggregator {
    use bridge_return_sdk_ext::verify::verify_token_anchored;
    use unicity_token::api::bft::{RootTrustBase, UnicityCertificate};
    use unicity_token::api::{InclusionProof, StateId};
    use unicity_token::client::{AggregatorClient, HttpAggregatorClient};
    use unicity_token::transaction::{
        CertifiedMintTransaction, CertifiedTransferTransaction, Token, Transaction,
    };

    use crate::{HostError, Result};

    /// Build an aggregator client from the environment: `UNICITY_GATEWAY` (the
    /// base URL, required) and `UNICITY_API_KEY` (optional, sent as the API key).
    pub fn client_from_env() -> Result<HttpAggregatorClient> {
        let url = std::env::var("UNICITY_GATEWAY")
            .map_err(|_| HostError::Check("UNICITY_GATEWAY is not set".to_string()))?;
        let mut client = HttpAggregatorClient::try_new(url)
            .map_err(|err| HostError::Check(format!("aggregator URL: {err}")))?;
        if let Ok(key) = std::env::var("UNICITY_API_KEY") {
            if !key.is_empty() {
                client = client.with_api_key(key);
            }
        }
        Ok(client)
    }

    /// Fetch the inclusion proof for one transition's state id.
    pub fn fetch_inclusion_proof(
        client: &HttpAggregatorClient,
        state_id: &StateId,
    ) -> Result<InclusionProof> {
        client
            .get_inclusion_proof(state_id)
            .map_err(|err| HostError::Check(format!("get_inclusion_proof: {err}")))
    }

    /// Fetch the inclusion proof for a token's terminal (burn) transition — the
    /// state the return relation settles against.
    pub fn fetch_terminal_inclusion_proof(
        client: &HttpAggregatorClient,
        token: &Token,
    ) -> Result<InclusionProof> {
        let terminal = token
            .transactions()
            .last()
            .ok_or_else(|| HostError::Check("token has no terminal transition".to_string()))?
            .transaction();
        let state_id = StateId::derive(terminal.lock_script(), terminal.source_state_hash());
        fetch_inclusion_proof(client, &state_id)
    }

    /// Re-fetch every transition's inclusion proof against the aggregator's
    /// **current** root and rebuild the token so all transitions share that one
    /// root, returning the rebuilt token + the shared anchor `UC*`. This is what
    /// enables anchored mode (one BFT-quorum check per batch, §11): re-fetching a
    /// historical transition yields a fresh inclusion path — with all sibling
    /// hashes — to the latest root (ZK_BACK3 §2.1), instead of the per-transition
    /// certificate the token was minted with. Verifies the rebuild in anchored
    /// mode before returning. Retries if a round advances mid-fetch (the proofs
    /// must all resolve to one root).
    pub fn fetch_anchored_token(
        client: &HttpAggregatorClient,
        token: &Token,
        trust_base: &RootTrustBase,
    ) -> Result<(Token, UnicityCertificate)> {
        for attempt in 0..6 {
            let mint = token.genesis().transaction();
            let mint_sid = StateId::derive(mint.lock_script(), mint.source_state_hash());
            let mint_proof = fetch_inclusion_proof(client, &mint_sid)?;
            let anchor = mint_proof.unicity_certificate.clone();
            let genesis = CertifiedMintTransaction::new(mint.clone(), mint_proof);
            let mut transfers = Vec::new();
            for t in token.transactions() {
                let tx = t.transaction();
                let sid = StateId::derive(tx.lock_script(), tx.source_state_hash());
                transfers.push(CertifiedTransferTransaction::new(
                    tx.clone(),
                    fetch_inclusion_proof(client, &sid)?,
                ));
            }
            let rebuilt = Token::new(genesis, transfers);
            match verify_token_anchored(&rebuilt, trust_base, &anchor) {
                Ok(()) => return Ok((rebuilt, anchor)),
                Err(err) => {
                    if attempt == 5 {
                        return Err(HostError::Check(format!(
                            "anchored re-fetch failed after retries (root skew?): {err:?}"
                        )));
                    }
                    std::thread::sleep(std::time::Duration::from_millis(2000));
                }
            }
        }
        unreachable!()
    }
}
