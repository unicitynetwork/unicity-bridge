use bridge_return_host::s1;

use crate::{
    config::{ProveMode, ServiceConfig},
    domain::guest::{GuestKeyError, VaultGuestKey},
};

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

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum GuestCheck {
    NotProving,
    Unpinned { vkey: String },
    Admitted { vkey: String },
}

impl Prover {
    pub fn new(config: ServiceConfig) -> Self {
        Self { config }
    }

    pub async fn check_guest(&self) -> Result<GuestCheck, ProverError> {
        if self.config.prove_mode == ProveMode::PrecheckOnly {
            return Ok(GuestCheck::NotProving);
        }
        let elf = self
            .config
            .elf_path
            .clone()
            .ok_or(ProverError::MissingSp1Elf)?;
        let vault = self.vault_guest_key()?;
        let vkey = program_vkey(elf).await?;
        match vault {
            None => Ok(GuestCheck::Unpinned { vkey }),
            Some(key) => {
                key.admit(&vkey)?;
                Ok(GuestCheck::Admitted { vkey })
            }
        }
    }

    fn vault_guest_key(&self) -> Result<Option<VaultGuestKey>, GuestKeyError> {
        let Some(path) = &self.config.deployment_config_path else {
            return Ok(None);
        };
        let record = std::fs::read_to_string(path)
            .map_err(|err| GuestKeyError::Record(format!("read {}: {err}", path.display())))?;
        VaultGuestKey::from_deployment(&record)
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
    #[error("{0}")]
    Guest(#[from] GuestKeyError),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
}

#[cfg(feature = "sp1")]
fn proof_path(root: &std::path::Path, batch_id: &str) -> std::path::PathBuf {
    root.join(format!("{batch_id}.bin"))
}

#[cfg(feature = "sp1")]
async fn program_vkey(elf: std::path::PathBuf) -> Result<String, ProverError> {
    Ok(
        tokio::task::spawn_blocking(move || bridge_return_host::sp1::program_vkey(&elf))
            .await
            .map_err(|err| ProverError::Join(err.to_string()))??,
    )
}

#[cfg(not(feature = "sp1"))]
async fn program_vkey(_elf: std::path::PathBuf) -> Result<String, ProverError> {
    Err(ProverError::Sp1FeatureDisabled)
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    fn proving(elf: Option<&str>, deployment: Option<PathBuf>) -> Prover {
        Prover::new(ServiceConfig {
            prove_mode: ProveMode::Sp1Groth16,
            elf_path: elf.map(PathBuf::from),
            deployment_config_path: deployment,
            ..ServiceConfig::default()
        })
    }

    fn deployment(name: &str) -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../../deployments")
            .join(name)
    }

    #[tokio::test]
    async fn a_service_that_only_prechecks_needs_no_guest() {
        let prover = Prover::new(ServiceConfig::default());

        assert_eq!(prover.check_guest().await.unwrap(), GuestCheck::NotProving);
    }

    #[tokio::test]
    async fn proving_without_a_guest_elf_is_refused_at_the_check() {
        let refused = proving(None, None).check_guest().await.unwrap_err();

        assert!(matches!(refused, ProverError::MissingSp1Elf));
    }

    #[tokio::test]
    async fn a_deployment_record_that_cannot_be_read_is_refused() {
        let refused = proving(Some("guest"), Some(deployment("missing.json")))
            .check_guest()
            .await
            .unwrap_err();

        assert!(matches!(
            refused,
            ProverError::Guest(GuestKeyError::Record(_))
        ));
    }

    #[cfg(not(feature = "sp1"))]
    #[tokio::test]
    async fn a_build_without_sp1_cannot_derive_the_guest_key_and_says_so() {
        let refused = proving(Some("guest"), Some(deployment("sepolia/sepolia-usdc.json")))
            .check_guest()
            .await
            .unwrap_err();

        assert!(matches!(refused, ProverError::Sp1FeatureDisabled));
    }

    #[cfg(feature = "sp1")]
    mod with_sp1 {
        use super::*;

        const PINNED_GUEST: &str = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../guest-elf/bridge-return-sp1-guest"
        );
        const VAULT_KEY: &str =
            "0x0039a5424014e57caf45d3451053e6c014547837ae09c9eb724aa569389b90d5";

        #[tokio::test]
        async fn the_pinned_guest_is_the_program_the_sepolia_and_nile_v2_vaults_hold() {
            for record in ["sepolia/sepolia-usdc.json", "nile/nile-usdt-v2.json"] {
                let check = proving(Some(PINNED_GUEST), Some(deployment(record)))
                    .check_guest()
                    .await;

                assert_eq!(
                    check.unwrap(),
                    GuestCheck::Admitted {
                        vkey: VAULT_KEY.to_string()
                    }
                );
            }
        }

        #[tokio::test]
        async fn the_pinned_guest_is_refused_for_a_vault_that_holds_another_key() {
            let refused = proving(Some(PINNED_GUEST), Some(deployment("nile/nile-usdt.json")))
                .check_guest()
                .await
                .unwrap_err();

            assert!(matches!(
                refused,
                ProverError::Guest(GuestKeyError::Mismatch { .. })
            ));
        }

        #[tokio::test]
        async fn without_a_deployment_record_the_guest_key_is_reported_unpinned() {
            let check = proving(Some(PINNED_GUEST), None).check_guest().await;

            assert_eq!(
                check.unwrap(),
                GuestCheck::Unpinned {
                    vkey: VAULT_KEY.to_string()
                }
            );
        }
    }
}
