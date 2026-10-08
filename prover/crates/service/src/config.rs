use std::{env, net::SocketAddr, path::PathBuf, time::Duration};

use serde::{Deserialize, Serialize};

use crate::domain::{
    fee::FeePolicy,
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
    pub command_timeout: Duration,
    pub state_dir: Option<PathBuf>,
    pub retry: RetryPolicy,
    pub fee: FeePolicy,
    pub elf_path: Option<PathBuf>,
    pub proof_dir: PathBuf,
    pub prove_mode: ProveMode,
    /// The command that proves one batch in a child process, given the ELF, wire, proof and info paths.
    pub prove_cmd: String,
    /// The command that prints the guest's verifying key as JSON, given the ELF path.
    pub vkey_cmd: String,
    /// The most a proof may take before it is killed; zero means no limit.
    pub prove_timeout: Duration,
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
            command_timeout: Duration::from_secs(600),
            state_dir: None,
            retry: RetryPolicy::default(),
            fee: FeePolicy::default(),
            elf_path: None,
            proof_dir: PathBuf::from("target/bridge-return-service/proofs"),
            prove_mode: ProveMode::PrecheckOnly,
            prove_cmd: "bridge-return-host sp1-groth16-files".to_string(),
            vkey_cmd: "bridge-return-host sp1-vkey".to_string(),
            prove_timeout: Duration::from_secs(7200),
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
        if let Some(secs) = env_parsed("BRIDGE_RETURN_COMMAND_TIMEOUT_SECS")? {
            cfg.command_timeout =
                Duration::from_secs(positive(secs, "BRIDGE_RETURN_COMMAND_TIMEOUT_SECS")? as u64);
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
        if let Some(amount) = env_parsed("BRIDGE_RETURN_FEE_AMOUNT")? {
            cfg.fee.amount = amount;
        }
        cfg.fee.floor = env_parsed("BRIDGE_RETURN_FEE_FLOOR")?.unwrap_or(0);
        if let Some(v) = env_opt("BRIDGE_RETURN_FEE_RECIPIENT") {
            cfg.fee.recipient =
                address(&v).ok_or(ConfigError::Invalid("BRIDGE_RETURN_FEE_RECIPIENT"))?;
        }
        if let Some(secs) = env_parsed::<u64>("BRIDGE_RETURN_FEE_WINDOW_SECS")? {
            cfg.fee.settle_window = Duration::from_secs(secs);
        }
        if !cfg.fee.is_collectable() {
            return Err(ConfigError::Invalid("BRIDGE_RETURN_FEE_RECIPIENT"));
        }
        if !cfg.fee.floor_within_amount() {
            return Err(ConfigError::Invalid("BRIDGE_RETURN_FEE_FLOOR"));
        }
        if let Some(v) = env_opt("BRIDGE_CONFIG_HASH") {
            cfg.config_hash = Some(
                crate::store::parse_hex32(&v).ok_or(ConfigError::Invalid("BRIDGE_CONFIG_HASH"))?,
            );
        }
        if let Some(v) = env_opt("BRIDGE_RETURN_PROVE_CMD") {
            cfg.prove_cmd = v;
        }
        if let Some(v) = env_opt("BRIDGE_RETURN_VKEY_CMD") {
            cfg.vkey_cmd = v;
        }
        if let Some(secs) = env_parsed::<u64>("BRIDGE_RETURN_PROVE_TIMEOUT_SECS")? {
            cfg.prove_timeout = Duration::from_secs(secs);
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

fn address(text: &str) -> Option<[u8; 20]> {
    hex::decode(text.strip_prefix("0x").unwrap_or(text))
        .ok()?
        .try_into()
        .ok()
}

fn positive(value: usize, key: &'static str) -> Result<usize, ConfigError> {
    if value == 0 {
        return Err(ConfigError::Invalid(key));
    }
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    const FEE_VARS: [&str; 4] = [
        "BRIDGE_RETURN_FEE_AMOUNT",
        "BRIDGE_RETURN_FEE_FLOOR",
        "BRIDGE_RETURN_FEE_RECIPIENT",
        "BRIDGE_RETURN_FEE_WINDOW_SECS",
    ];

    fn with_fee_env(vars: &[(&str, &str)]) -> Result<FeePolicy, ConfigError> {
        for key in FEE_VARS {
            env::remove_var(key);
        }
        for (key, value) in vars {
            env::set_var(key, value);
        }
        let result = ServiceConfig::from_env().map(|cfg| cfg.fee);
        for key in FEE_VARS {
            env::remove_var(key);
        }
        result
    }

    // One test, since the process environment is shared between test threads.
    #[test]
    fn the_fee_settings_are_read_with_a_floor_of_zero_unless_set() {
        let recipient = (
            "BRIDGE_RETURN_FEE_RECIPIENT",
            "0xc3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3",
        );

        let quoted = with_fee_env(&[("BRIDGE_RETURN_FEE_AMOUNT", "50000"), recipient]).unwrap();
        assert_eq!((quoted.amount, quoted.floor), (50_000, 0));
        assert!(!quoted.is_enforced());

        let enforced = with_fee_env(&[
            ("BRIDGE_RETURN_FEE_AMOUNT", "50000"),
            ("BRIDGE_RETURN_FEE_FLOOR", "40000"),
            recipient,
        ])
        .unwrap();
        assert_eq!((enforced.amount, enforced.floor), (50_000, 40_000));

        let window = with_fee_env(&[("BRIDGE_RETURN_FEE_WINDOW_SECS", "7200")]).unwrap();
        assert_eq!(window.settle_window, Duration::from_secs(7200));

        let defaults = ServiceConfig::from_env().unwrap();
        assert_eq!(defaults.prove_cmd, "bridge-return-host sp1-groth16-files");
        assert_eq!(defaults.vkey_cmd, "bridge-return-host sp1-vkey");
        env::set_var(
            "BRIDGE_RETURN_VKEY_CMD",
            "/app/bin/bridge-return-host sp1-vkey",
        );
        assert_eq!(
            ServiceConfig::from_env().unwrap().vkey_cmd,
            "/app/bin/bridge-return-host sp1-vkey"
        );
        env::remove_var("BRIDGE_RETURN_VKEY_CMD");
        assert_eq!(defaults.prove_timeout, Duration::from_secs(7200));
        env::set_var(
            "BRIDGE_RETURN_PROVE_CMD",
            "/app/bin/bridge-return-host sp1-groth16-files",
        );
        env::set_var("BRIDGE_RETURN_PROVE_TIMEOUT_SECS", "900");
        let proving = ServiceConfig::from_env().unwrap();
        env::set_var("BRIDGE_RETURN_PROVE_TIMEOUT_SECS", "0");
        let unlimited = ServiceConfig::from_env().unwrap();
        env::remove_var("BRIDGE_RETURN_PROVE_CMD");
        env::remove_var("BRIDGE_RETURN_PROVE_TIMEOUT_SECS");
        assert_eq!(
            proving.prove_cmd,
            "/app/bin/bridge-return-host sp1-groth16-files"
        );
        assert_eq!(proving.prove_timeout, Duration::from_secs(900));
        assert_eq!(unlimited.prove_timeout, Duration::ZERO);

        assert!(matches!(
            with_fee_env(&[("BRIDGE_RETURN_FEE_AMOUNT", "50000")]),
            Err(ConfigError::Invalid("BRIDGE_RETURN_FEE_RECIPIENT"))
        ));
        assert!(matches!(
            with_fee_env(&[
                ("BRIDGE_RETURN_FEE_AMOUNT", "50000"),
                ("BRIDGE_RETURN_FEE_FLOOR", "50001"),
                recipient
            ]),
            Err(ConfigError::Invalid("BRIDGE_RETURN_FEE_FLOOR"))
        ));
    }
}
