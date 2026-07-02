use std::{fs, path::Path, sync::Arc};

use bridge_return_guest::GuestOutput;
use sp1_sdk::blocking::{Elf, ProveRequest, Prover, ProverClient, SP1Stdin};
use sp1_sdk::{HashableKey, ProvingKey, SP1Proof, SP1ProofWithPublicValues};

use crate::{HostError, Result};

/// SP1 release circuit version embedded in this `sp1-sdk` build (e.g. `v6.1.0`).
/// The on-chain Groth16 verifier bytecode and the downloaded circuit/key archive
/// are pinned to this string; the published bundle records it for provenance.
pub fn circuit_version() -> &'static str {
    sp1_sdk::SP1_CIRCUIT_VERSION
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Sp1Execution {
    pub public_values: Vec<u8>,
    pub expected_public_values: Vec<u8>,
    pub cycles: Option<u64>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Sp1ProofInfo {
    pub proof_mode: &'static str,
    pub public_values: Vec<u8>,
    pub proof_bytes: Vec<u8>,
    pub sp1_version: String,
    /// The program verifying-key hash (`bytes32`, "0x"-prefixed) the on-chain
    /// `verifyProof(programVKey, …)` binds to. `None` when derived from a saved
    /// proof file alone, which does not carry the vkey — recover it from the ELF
    /// via [`program_vkey`] / [`export_onchain`].
    pub vkey_hash: Option<String>,
}

pub fn execute_elf(elf_path: &Path, wire_input: Vec<u8>) -> Result<Sp1Execution> {
    let elf = load_elf(elf_path)?;
    let expected = expected_output(&wire_input)?;
    let client = ProverClient::builder().cpu().build();
    let (public_values, report) = client
        .execute(elf, stdin(wire_input))
        .run()
        .map_err(|err| HostError::Check(format!("SP1 execute failed: {err}")))?;
    let actual = public_values.to_vec();
    ensure_public_values_match(&actual, &expected)?;
    Ok(Sp1Execution {
        public_values: actual,
        expected_public_values: expected_public_values(&expected),
        cycles: Some(report.total_instruction_count()),
    })
}

pub fn mock_groth16(
    elf_path: &Path,
    wire_input: Vec<u8>,
    proof_path: &Path,
) -> Result<Sp1ProofInfo> {
    let elf = load_elf(elf_path)?;
    let expected = expected_output(&wire_input)?;
    let client = ProverClient::builder().mock().build();
    let pk = client
        .setup(elf)
        .map_err(|err| HostError::Check(format!("SP1 setup failed: {err}")))?;
    let proof = client
        .prove(&pk, stdin(wire_input))
        .groth16()
        .run()
        .map_err(|err| HostError::Check(format!("SP1 mock groth16 failed: {err}")))?;
    client
        .verify(&proof, pk.verifying_key(), None)
        .map_err(|err| HostError::Check(format!("SP1 mock proof verify failed: {err}")))?;
    let vkey_hash = pk.verifying_key().bytes32();
    let actual = proof.public_values.to_vec();
    ensure_public_values_match(&actual, &expected)?;
    proof
        .save(proof_path)
        .map_err(|err| HostError::Check(format!("save SP1 proof: {err}")))?;
    Ok(proof_info(&proof, Some(vkey_hash)))
}

/// Real CPU Groth16 proof. The complete STARK → recursion → shrink → wrap →
/// BN254 Groth16 pipeline runs natively on local CPU. In release circuit mode,
/// SP1 downloads its pre-generated circuit and proving key on first use; no
/// Docker, prover network, GPU, or local Groth16 setup is required.
/// Otherwise identical to {mock_groth16}: it sets up the program key, proves in
/// `groth16` mode, verifies the proof against the program vkey, checks the
/// committed public values equal the guest precheck, and saves.
pub fn real_groth16(
    elf_path: &Path,
    wire_input: Vec<u8>,
    proof_path: &Path,
) -> Result<Sp1ProofInfo> {
    ensure_release_circuit_mode()?;
    // Install SP1's tracing subscriber so RUST_LOG stage logs (and any OOM/abort
    // reason) are visible — the standalone host CLI otherwise initializes no
    // subscriber. `setup_logger()` panics if a global default is already set
    // (e.g. bridge-return-service installs its own at startup) — catch that
    // specific case; logging already works there, nothing to do.
    let _ = std::panic::catch_unwind(sp1_sdk::utils::setup_logger);
    let elf = load_elf(elf_path)?;
    let expected = expected_output(&wire_input)?;
    let client = ProverClient::builder().cpu().build();
    let pk = client
        .setup(elf)
        .map_err(|err| HostError::Check(format!("SP1 setup failed: {err}")))?;
    let proof = client
        .prove(&pk, stdin(wire_input))
        .groth16()
        .run()
        .map_err(|err| HostError::Check(format!("SP1 groth16 prove failed: {err:?}")))?;
    client
        .verify(&proof, pk.verifying_key(), None)
        .map_err(|err| HostError::Check(format!("SP1 groth16 proof verify failed: {err}")))?;
    let vkey_hash = pk.verifying_key().bytes32();
    let actual = proof.public_values.to_vec();
    ensure_public_values_match(&actual, &expected)?;
    proof
        .save(proof_path)
        .map_err(|err| HostError::Check(format!("save SP1 proof: {err}")))?;
    Ok(proof_info(&proof, Some(vkey_hash)))
}

/// Derive the program verifying-key hash (`bytes32`) from the guest ELF alone.
/// This is the `programVKey` the source-chain vault stores and passes to
/// `verifyProof`; it is deterministic in the ELF and independent of any input.
/// Only the cheap program key setup runs here — not the full proving pipeline.
pub fn program_vkey(elf_path: &Path) -> Result<String> {
    let elf = load_elf(elf_path)?;
    let client = ProverClient::builder().cpu().build();
    let pk = client
        .setup(elf)
        .map_err(|err| HostError::Check(format!("SP1 setup failed: {err}")))?;
    Ok(pk.verifying_key().bytes32())
}

/// Assemble the publishable on-chain verification bundle from a guest ELF and a
/// previously saved proof. Re-derives `programVKey` from the ELF, re-verifies the
/// saved proof against it, and returns the proof info with `vkey_hash` populated.
/// The caller serializes the `(programVKey, publicValues, proofBytes)` triple the
/// vault's `verifyProof` consumes.
pub fn export_onchain(elf_path: &Path, proof_path: &Path) -> Result<Sp1ProofInfo> {
    let elf = load_elf(elf_path)?;
    let client = ProverClient::builder().cpu().build();
    let pk = client
        .setup(elf)
        .map_err(|err| HostError::Check(format!("SP1 setup failed: {err}")))?;
    let proof = SP1ProofWithPublicValues::load(proof_path)
        .map_err(|err| HostError::Check(format!("load SP1 proof: {err}")))?;
    client
        .verify(&proof, pk.verifying_key(), None)
        .map_err(|err| HostError::Check(format!("saved proof fails verification: {err}")))?;
    Ok(proof_info(&proof, Some(pk.verifying_key().bytes32())))
}

fn ensure_release_circuit_mode() -> Result<()> {
    if std::env::var("SP1_CIRCUIT_MODE").as_deref() == Ok("dev") {
        return Err(HostError::Check(
            "SP1_CIRCUIT_MODE=dev would use a private artifact bucket and fall back to local Groth16 key generation; unset it or set SP1_CIRCUIT_MODE=release"
                .to_string(),
        ));
    }
    Ok(())
}

pub fn proof_info_from_file(proof_path: &Path) -> Result<Sp1ProofInfo> {
    let proof = SP1ProofWithPublicValues::load(proof_path)
        .map_err(|err| HostError::Check(format!("load SP1 proof: {err}")))?;
    Ok(proof_info(&proof, None))
}

fn load_elf(path: &Path) -> Result<Elf> {
    let bytes = fs::read(path)?;
    Ok(Elf::Dynamic(Arc::<[u8]>::from(bytes)))
}

fn stdin(wire_input: Vec<u8>) -> SP1Stdin {
    let mut stdin = SP1Stdin::new();
    stdin.write_vec(wire_input);
    stdin
}

fn expected_output(wire_input: &[u8]) -> Result<GuestOutput> {
    bridge_return_guest::execute_wire(wire_input)
        .map_err(|err| HostError::Check(format!("guest wire precheck failed: {err:?}")))
}

fn expected_public_values(output: &GuestOutput) -> Vec<u8> {
    let mut expected = output.public_values_abi.clone();
    expected.extend_from_slice(&output.public_values_digest);
    expected
}

fn ensure_public_values_match(actual: &[u8], expected: &GuestOutput) -> Result<()> {
    let expected = expected_public_values(expected);
    if actual != expected {
        return Err(HostError::Check(format!(
            "SP1 public values mismatch: actual=0x{} expected=0x{}",
            hex::encode(actual),
            hex::encode(expected)
        )));
    }
    Ok(())
}

fn proof_info(proof: &SP1ProofWithPublicValues, vkey_hash: Option<String>) -> Sp1ProofInfo {
    Sp1ProofInfo {
        proof_mode: match &proof.proof {
            SP1Proof::Core(_) => "core",
            SP1Proof::Compressed(_) => "compressed",
            SP1Proof::Plonk(_) => "plonk",
            SP1Proof::Groth16(_) => "groth16",
        },
        public_values: proof.public_values.to_vec(),
        proof_bytes: proof.bytes(),
        sp1_version: proof.sp1_version.clone(),
        vkey_hash,
    }
}
