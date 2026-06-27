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

use bridge_return_core::{PublicValues, U256};
use bridge_return_guest::{execute_public_output, execute_wire, wire, GuestInput};

use crate::{HostError, Result};

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

/// Decode a wire payload into a [`WitnessPackage`] and precheck it. Useful as a
/// standalone fail-fast gate over the exact bytes a prover would receive.
pub fn precheck_wire(wire_input: &[u8]) -> Result<PrecheckReport> {
    let input = wire::decode_guest_input(wire_input)
        .map_err(|err| HostError::Check(format!("wire decode: {err:?}")))?;
    WitnessPackage::new(input).precheck()
}
