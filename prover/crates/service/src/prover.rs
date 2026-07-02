use bridge_return_host::s1;

use crate::config::{ProveMode, ServiceConfig};

#[derive(Clone)]
pub struct Prover {
    config: ServiceConfig,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProofBundle {
    pub mode: String,
    pub public_values: Vec<u8>,
    pub proof_bytes: Vec<u8>,
    pub vkey_hash: Option<String>,
}

impl Prover {
    pub fn new(config: ServiceConfig) -> Self {
        Self { config }
    }

    pub async fn prove(
        &self,
        batch_id: String,
        wire_input: Vec<u8>,
    ) -> Result<ProofBundle, ProverError> {
        match self.config.prove_mode {
            ProveMode::PrecheckOnly => {
                let report = tokio::task::spawn_blocking(move || s1::precheck_wire(&wire_input))
                    .await
                    .map_err(|err| ProverError::Join(err.to_string()))??;
                Ok(ProofBundle {
                    mode: "precheck_only".to_string(),
                    public_values: report.public_values_abi,
                    proof_bytes: Vec::new(),
                    vkey_hash: None,
                })
            }
            ProveMode::Sp1Groth16 => self.prove_sp1(batch_id, wire_input).await,
        }
    }

    #[cfg(feature = "sp1")]
    async fn prove_sp1(
        &self,
        batch_id: String,
        wire_input: Vec<u8>,
    ) -> Result<ProofBundle, ProverError> {
        use std::fs;

        let elf = self
            .config
            .elf_path
            .clone()
            .ok_or(ProverError::MissingSp1Elf)?;
        let proof_path = proof_path(&self.config.proof_dir, &batch_id);
        let proof_dir = self.config.proof_dir.clone();
        let info = tokio::task::spawn_blocking(move || {
            fs::create_dir_all(&proof_dir)?;
            bridge_return_host::sp1::real_groth16(&elf, wire_input, &proof_path)
        })
        .await
        .map_err(|err| ProverError::Join(err.to_string()))??;
        Ok(ProofBundle {
            mode: info.proof_mode.to_string(),
            public_values: info.public_values,
            proof_bytes: info.proof_bytes,
            vkey_hash: info.vkey_hash,
        })
    }

    #[cfg(not(feature = "sp1"))]
    async fn prove_sp1(
        &self,
        _batch_id: String,
        _wire_input: Vec<u8>,
    ) -> Result<ProofBundle, ProverError> {
        Err(ProverError::Sp1FeatureDisabled)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ProverError {
    #[error("{0}")]
    Host(#[from] bridge_return_host::HostError),
    #[error("prover task join failed: {0}")]
    Join(String),
    #[error("SP1 proving requested but bridge-return-service was built without --features sp1")]
    Sp1FeatureDisabled,
    #[error("SP1_GUEST_ELF must be set for sp1_groth16 mode")]
    MissingSp1Elf,
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
}

#[cfg(feature = "sp1")]
fn proof_path(root: &std::path::Path, batch_id: &str) -> std::path::PathBuf {
    root.join(format!("{batch_id}.bin"))
}
