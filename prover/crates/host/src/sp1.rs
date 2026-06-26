use std::{fs, path::Path, sync::Arc};

use bridge_return_guest::GuestOutput;
use sp1_sdk::blocking::{Elf, ProveRequest, Prover, ProverClient, SP1Stdin};
use sp1_sdk::{ProvingKey, SP1Proof, SP1ProofWithPublicValues};

use crate::{HostError, Result};

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
    let actual = proof.public_values.to_vec();
    ensure_public_values_match(&actual, &expected)?;
    proof
        .save(proof_path)
        .map_err(|err| HostError::Check(format!("save SP1 proof: {err}")))?;
    Ok(proof_info(&proof))
}

pub fn proof_info_from_file(proof_path: &Path) -> Result<Sp1ProofInfo> {
    let proof = SP1ProofWithPublicValues::load(proof_path)
        .map_err(|err| HostError::Check(format!("load SP1 proof: {err}")))?;
    Ok(proof_info(&proof))
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

fn proof_info(proof: &SP1ProofWithPublicValues) -> Sp1ProofInfo {
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
    }
}
