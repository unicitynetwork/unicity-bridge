use std::{env, net::SocketAddr, path::PathBuf, time::Duration};

use serde::{Deserialize, Serialize};

use crate::domain::{
    policy::{BatchPolicy, Limits},
    retry::RetryPolicy,
};

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ServiceConfig {
    pub bind: SocketAddr,
    pub gateway_url: Option<String>,
    pub vault: Option<String>,
    pub config_hash: Option<[u8; 32]>,
    pub trust_base_path: Option<PathBuf>,
    /// Frozen deployment config JSON (e.g. `deployments/nile/nile-usdt.json`).
    /// Required (with `trust_base_path`) to accept the wallet `{tokenCbor,reasonBytes}` envelope.
    pub deployment_config_path: Option<PathBuf>,
    /// Source-chain lock-justification CBOR tag (bridge lock = 1330002 on every family).
    pub justification_tag: u64,
    pub idle_wait: Duration,
    pub max_batch_size: usize,
    pub max_batch_bytes: usize,
    pub state_dir: Option<PathBuf>,
    pub retry: RetryPolicy,
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
            vault: None,
            config_hash: None,
            trust_base_path: None,
            deployment_config_path: None,
            justification_tag: 1_330_002,
            idle_wait: Duration::ZERO,
            max_batch_size: 8,
            max_batch_bytes: 8 << 20,
            state_dir: None,
            retry: RetryPolicy::default(),
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
        if let Some(secs) = env_parsed::<u64>("BRIDGE_RETURN_IDLE_WAIT_SECS")? {
            cfg.idle_wait = Duration::from_secs(secs);
        }
        if let Some(size) = env_parsed("BRIDGE_RETURN_MAX_BATCH_SIZE")? {
            cfg.max_batch_size = positive(size, "BRIDGE_RETURN_MAX_BATCH_SIZE")?;
        }
        if let Some(bytes) = env_parsed("BRIDGE_RETURN_MAX_BATCH_BYTES")? {
            cfg.max_batch_bytes = positive(bytes, "BRIDGE_RETURN_MAX_BATCH_BYTES")?;
        }
        cfg.state_dir = env_opt("BRIDGE_RETURN_STATE_DIR").map(PathBuf::from);
        if let Some(secs) = env_parsed::<u64>("BRIDGE_RETURN_RETRY_BASE_SECS")? {
            cfg.retry.base = Duration::from_secs(secs);
        }
        if let Some(attempts) = env_parsed("BRIDGE_RETURN_MAX_ATTEMPTS")? {
            cfg.retry.max_attempts = positive(attempts, "BRIDGE_RETURN_MAX_ATTEMPTS")? as u32;
        }
        if let Some(rebases) = env_parsed("BRIDGE_RETURN_MAX_REBASES")? {
            cfg.retry.max_rebases = rebases;
        }
        if let Some(v) = env_opt("BRIDGE_CONFIG_HASH") {
            cfg.config_hash = Some(
                crate::store::parse_hex32(&v).ok_or(ConfigError::Invalid("BRIDGE_CONFIG_HASH"))?,
            );
        }
        if env_opt("BRIDGE_RETURN_PROVE_MODE").as_deref() == Some("sp1_groth16") {
            cfg.prove_mode = ProveMode::Sp1Groth16;
        }
        Ok(cfg)
    }
}

impl From<&ServiceConfig> for Limits {
    fn from(config: &ServiceConfig) -> Self {
        Self {
            max_size: config.max_batch_size,
            max_bytes: config.max_batch_bytes,
            idle_wait: config.idle_wait,
        }
    }
}

impl From<&ServiceConfig> for BatchPolicy {
    fn from(config: &ServiceConfig) -> Self {
        Self {
            limits: Limits::from(config),
        }
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

fn env_parsed<T: std::str::FromStr>(key: &'static str) -> Result<Option<T>, ConfigError> {
    env_opt(key)
        .map(|v| v.parse().map_err(|_| ConfigError::Invalid(key)))
        .transpose()
}

fn positive(value: usize, key: &'static str) -> Result<usize, ConfigError> {
    if value == 0 {
        return Err(ConfigError::Invalid(key));
    }
    Ok(value)
}
