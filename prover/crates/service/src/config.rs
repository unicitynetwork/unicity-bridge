use std::{env, net::SocketAddr, path::PathBuf, time::Duration};

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ServiceConfig {
    pub bind: SocketAddr,
    pub gateway_url: Option<String>,
    pub tron_grid_url: Option<String>,
    pub vault: Option<String>,
    pub config_hash: Option<[u8; 32]>,
    pub trust_base_path: Option<PathBuf>,
    /// Frozen deployment config JSON (e.g. `bridge-vectors/deployment/nile-usdt.json`).
    /// Required (with `trust_base_path`) to accept the wallet `{tokenCbor,reasonBytes}` envelope.
    pub deployment_config_path: Option<PathBuf>,
    /// Source-chain lock-justification CBOR tag (Tron USDT = 1330002).
    pub justification_tag: u64,
    pub max_wait: Duration,
    pub batch_target: usize,
    pub elf_path: Option<PathBuf>,
    pub proof_dir: PathBuf,
    pub prove_mode: ProveMode,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProveMode {
    PrecheckOnly,
    Sp1Groth16,
}

impl Default for ServiceConfig {
    fn default() -> Self {
        Self {
            bind: "127.0.0.1:8787".parse().expect("default bind addr"),
            gateway_url: None,
            tron_grid_url: None,
            vault: None,
            config_hash: None,
            trust_base_path: None,
            deployment_config_path: None,
            justification_tag: 1_330_002,
            max_wait: Duration::from_secs(60),
            batch_target: 1,
            elf_path: None,
            proof_dir: PathBuf::from("target/bridge-return-service/proofs"),
            prove_mode: ProveMode::PrecheckOnly,
        }
    }
}

impl ServiceConfig {
    pub fn from_env() -> Result<Self, ConfigError> {
        let mut cfg = Self::default();
        if let Some(v) = env_opt("BRIDGE_RETURN_BIND") {
            cfg.bind = v
                .parse()
                .map_err(|_| ConfigError::Invalid("BRIDGE_RETURN_BIND"))?;
        }
        cfg.gateway_url = env_opt("UNICITY_GATEWAY");
        cfg.tron_grid_url = env_opt("TRON_GRID_URL");
        cfg.vault = env_opt("BRIDGE_VAULT");
        cfg.trust_base_path = env_opt("TRUST_BASE_PATH").map(PathBuf::from);
        cfg.deployment_config_path = env_opt("BRIDGE_DEPLOYMENT_CONFIG").map(PathBuf::from);
        if let Some(v) = env_opt("BRIDGE_JUSTIFICATION_TAG") {
            cfg.justification_tag = v
                .parse()
                .map_err(|_| ConfigError::Invalid("BRIDGE_JUSTIFICATION_TAG"))?;
        }
        cfg.elf_path = env_opt("SP1_GUEST_ELF").map(PathBuf::from);
        if let Some(v) = env_opt("BRIDGE_RETURN_PROOF_DIR") {
            cfg.proof_dir = PathBuf::from(v);
        }
        if let Some(v) = env_opt("BRIDGE_RETURN_MAX_WAIT_SECS") {
            cfg.max_wait = Duration::from_secs(
                v.parse()
                    .map_err(|_| ConfigError::Invalid("BRIDGE_RETURN_MAX_WAIT_SECS"))?,
            );
        }
        if let Some(v) = env_opt("BRIDGE_RETURN_BATCH_TARGET") {
            cfg.batch_target = v
                .parse()
                .map_err(|_| ConfigError::Invalid("BRIDGE_RETURN_BATCH_TARGET"))?;
            if cfg.batch_target == 0 {
                return Err(ConfigError::Invalid("BRIDGE_RETURN_BATCH_TARGET"));
            }
        }
        if let Some(v) = env_opt("BRIDGE_CONFIG_HASH") {
            cfg.config_hash = Some(hex32(&v).ok_or(ConfigError::Invalid("BRIDGE_CONFIG_HASH"))?);
        }
        if env_opt("BRIDGE_RETURN_PROVE_MODE").as_deref() == Some("sp1_groth16") {
            cfg.prove_mode = ProveMode::Sp1Groth16;
        }
        Ok(cfg)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("invalid environment variable: {0}")]
    Invalid(&'static str),
}

fn env_opt(key: &str) -> Option<String> {
    env::var(key).ok().filter(|v| !v.is_empty())
}

fn hex32(input: &str) -> Option<[u8; 32]> {
    let raw = input.strip_prefix("0x").unwrap_or(input);
    let bytes = hex::decode(raw).ok()?;
    bytes.try_into().ok()
}
