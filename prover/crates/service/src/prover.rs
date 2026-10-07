use std::{
    fs,
    os::unix::process::ExitStatusExt,
    path::{Path, PathBuf},
    process::Stdio,
};

use bridge_return_host::s1;
use tokio::process::Command;

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

    /// Proves in a child process, files in and files out, so the memory a proof takes goes back
    /// to the OS when it exits and a kill ends the attempt, not the service.
    async fn prove_sp1(
        &self,
        batch_id: String,
        wire_input: Vec<u8>,
    ) -> Result<ProofBundle, ProverError> {
        let elf = self
            .config
            .elf_path
            .clone()
            .ok_or(ProverError::MissingSp1Elf)?;
        let paths = ProofPaths::new(&self.config.proof_dir, &batch_id);
        fs::create_dir_all(&self.config.proof_dir)?;
        fs::write(&paths.wire, &wire_input)?;
        let outcome = self.run_prove_command(&elf, &paths).await;
        let _ = fs::remove_file(&paths.wire);
        outcome?;
        let info = fs::read(&paths.info).map_err(|err| {
            ProverError::Command(format!(
                "the proving command for {batch_id} exited 0 without writing {}: {err}",
                paths.info.display()
            ))
        })?;
        ProofBundle::from_info_json(&info).map_err(|err| {
            ProverError::Command(format!(
                "{} of {batch_id} is not a proof info: {err}",
                paths.info.display()
            ))
        })
    }

    async fn run_prove_command(&self, elf: &Path, paths: &ProofPaths) -> Result<(), ProverError> {
        let mut child = Command::new("sh")
            .arg("-c")
            .arg(format!("{} \"$@\"", self.config.prove_cmd))
            .arg("sh")
            .arg(elf)
            .arg(&paths.wire)
            .arg(&paths.proof)
            .arg(&paths.info)
            .stdin(Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .map_err(|err| {
                ProverError::Command(format!("could not start the proving command: {err}"))
            })?;
        let status = tokio::time::timeout(self.config.prove_timeout, child.wait())
            .await
            .map_err(|_| {
                ProverError::Command(format!(
                    "proving {} timed out after {}s",
                    paths.batch_id,
                    self.config.prove_timeout.as_secs()
                ))
            })??;
        if status.success() {
            return Ok(());
        }
        let ended = match status.code() {
            Some(code) => format!("exited with status {code}"),
            None => format!("was killed by signal {}", status.signal().unwrap_or(0)),
        };
        Err(ProverError::Command(format!(
            "the proving command for {} {ended}; see the log above for its output",
            paths.batch_id
        )))
    }
}

impl ProofBundle {
    /// The info file the host's `sp1-groth16-files` command writes: `0x` hex for the bytes.
    fn from_info_json(bytes: &[u8]) -> Result<Self, String> {
        let info: ProofInfoFile = serde_json::from_slice(bytes).map_err(|err| err.to_string())?;
        Ok(Self {
            mode: info.proof_mode,
            public_values: hex_field(&info.public_values)?,
            proof_bytes: hex_field(&info.proof_bytes)?,
            vkey_hash: info.vkey,
        })
    }
}

#[derive(serde::Deserialize)]
struct ProofInfoFile {
    proof_mode: String,
    vkey: Option<String>,
    public_values: String,
    proof_bytes: String,
}

fn hex_field(value: &str) -> Result<Vec<u8>, String> {
    hex::decode(value.strip_prefix("0x").unwrap_or(value)).map_err(|err| err.to_string())
}

struct ProofPaths {
    batch_id: String,
    wire: PathBuf,
    proof: PathBuf,
    info: PathBuf,
}

impl ProofPaths {
    fn new(root: &Path, batch_id: &str) -> Self {
        Self {
            batch_id: batch_id.to_string(),
            wire: root.join(format!("{batch_id}.wire")),
            proof: root.join(format!("{batch_id}.bin")),
            info: root.join(format!("{batch_id}.json")),
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ProverError {
    #[error("{0}")]
    Host(#[from] bridge_return_host::HostError),
    #[error("prover task join failed: {0}")]
    Join(String),
    #[error("{0}")]
    Command(String),
    #[error("the guest key check needs bridge-return-service built with --features sp1")]
    Sp1FeatureDisabled,
    #[error("SP1_GUEST_ELF must be set for sp1_groth16 mode")]
    MissingSp1Elf,
    #[error("{0}")]
    Guest(#[from] GuestKeyError),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
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
    use std::{path::PathBuf, time::Duration};

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

    /// A prover whose proving command is a shell snippet, run as `sh -c "<snippet> \"$@\"" sh elf wire proof info`.
    fn proving_with(dir: &tempfile::TempDir, prove_cmd: &str, timeout: Duration) -> Prover {
        Prover::new(ServiceConfig {
            prove_mode: ProveMode::Sp1Groth16,
            elf_path: Some(dir.path().join("guest.elf")),
            proof_dir: dir.path().join("proofs"),
            prove_cmd: prove_cmd.to_string(),
            prove_timeout: timeout,
            ..ServiceConfig::default()
        })
    }

    // The command gets the ELF, wire, proof and info paths as its arguments, so a snippet is a function of them.
    const WRITES_INFO: &str = r#"f() { cmp -s "$2" "$3.expected" || exit 9; printf '{"proof_mode":"groth16","sp1_version":"6.3.1","vkey":"0x00aa","public_values":"0x0102","proof_bytes":"0xdeadbeef","proof_bytes_len":4}' > "$4"; : > "$3"; }; f"#;

    #[tokio::test]
    async fn a_proof_is_made_by_the_command_from_files_and_read_back_from_its_info_file() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("proofs")).unwrap();
        std::fs::write(dir.path().join("proofs/b-1.bin.expected"), b"wire bytes").unwrap();
        let prover = proving_with(&dir, WRITES_INFO, Duration::from_secs(5));

        let bundle = prover
            .prove("b-1".into(), b"wire bytes".to_vec())
            .await
            .unwrap();

        assert_eq!(
            bundle,
            ProofBundle {
                mode: "groth16".into(),
                public_values: vec![1, 2],
                proof_bytes: vec![0xde, 0xad, 0xbe, 0xef],
                vkey_hash: Some("0x00aa".into()),
            }
        );
        assert!(dir.path().join("proofs/b-1.bin").exists());
        assert!(dir.path().join("proofs/b-1.json").exists());
        assert!(
            !dir.path().join("proofs/b-1.wire").exists(),
            "the wire file is cleaned up"
        );
    }

    #[tokio::test]
    async fn a_command_that_exits_with_an_error_fails_the_proof_and_names_the_batch() {
        let dir = tempfile::tempdir().unwrap();
        let err = proving_with(&dir, "f() { exit 1; }; f", Duration::from_secs(5))
            .prove("b-2".into(), vec![1])
            .await
            .unwrap_err();
        let message = err.to_string();
        assert!(
            message.contains("exited with status 1") && message.contains("b-2"),
            "{message}"
        );
        assert!(!dir.path().join("proofs/b-2.wire").exists());
    }

    #[tokio::test]
    async fn a_command_killed_by_a_signal_fails_the_proof_with_the_signal() {
        let dir = tempfile::tempdir().unwrap();
        let err = proving_with(&dir, "f() { kill -9 $$; }; f", Duration::from_secs(5))
            .prove("b-3".into(), vec![1])
            .await
            .unwrap_err();
        assert!(err.to_string().contains("signal 9"), "{err}");
    }

    #[tokio::test]
    async fn a_command_that_runs_past_the_proving_timeout_is_cut_off() {
        let dir = tempfile::tempdir().unwrap();
        let started = std::time::Instant::now();
        let err = proving_with(&dir, "f() { sleep 5; }; f", Duration::from_millis(200))
            .prove("b-4".into(), vec![1])
            .await
            .unwrap_err();
        assert!(err.to_string().contains("timed out"), "{err}");
        assert!(started.elapsed() < Duration::from_secs(3));
    }

    #[tokio::test]
    async fn a_command_that_succeeds_without_an_info_file_fails_the_proof() {
        let dir = tempfile::tempdir().unwrap();
        let err = proving_with(&dir, "f() { exit 0; }; f", Duration::from_secs(5))
            .prove("b-5".into(), vec![1])
            .await
            .unwrap_err();
        assert!(err.to_string().contains("b-5.json"), "{err}");
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
