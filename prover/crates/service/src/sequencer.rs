//! S2/S3 chain-sync seam — reconstruct the vault's replay accumulator from its
//! on-chain settlement events so the prover builds each batch on top of the
//! vault's **current** `spentRoot` (not a stale/empty one).
//!
//! Without this, intake assembles every batch against an empty accumulator
//! (`spent_root_old = 0`), and `fulfillBatch` reverts with `vault: stale root`
//! the moment the vault has settled even one prior batch. The accumulator math
//! lives in `bridge_return_host::s2`; this module only owns the seam that feeds
//! it the on-chain event log.
//!
//! Pluggable (like the S4 {Submitter}) so a JS relayer, or a future in-process
//! Tron client, drops in without touching the queue:
//!  - `none` (default): no watcher — treats the vault as pristine
//!    (`spent_root = 0`). Correct only for a fresh vault / fixtures / tests.
//!  - `command`: runs `BRIDGE_RETURN_EVENTS_CMD`, which must print the settlement
//!    log JSON on stdout: `{ "batches": [{ nullifiers, spent_root_old,
//!    spent_root_new }], "spent_root": <live vault spentRoot> }` — exactly what
//!    `relayer.js events` emits.

use bridge_return_host::s2::{parse_settled_log, rebuild_verified, RebuiltAccumulator, SettledLog};

/// Chain-sync backend for the accumulator.
#[derive(Clone)]
pub struct ChainEvents {
    backend: Backend,
}

#[derive(Clone)]
enum Backend {
    None,
    Command(String),
}

impl ChainEvents {
    /// Build from `BRIDGE_RETURN_EVENTS_CMD` (a shell command). Empty/unset = `none`.
    pub fn from_env() -> Self {
        match std::env::var("BRIDGE_RETURN_EVENTS_CMD") {
            Ok(cmd) if !cmd.trim().is_empty() => Self {
                backend: Backend::Command(cmd),
            },
            _ => Self {
                backend: Backend::None,
            },
        }
    }

    pub fn none() -> Self {
        Self {
            backend: Backend::None,
        }
    }

    /// Whether a live watcher is configured — `false` means the vault is assumed
    /// pristine, which is only safe for a fresh deployment.
    pub fn is_live(&self) -> bool {
        matches!(self.backend, Backend::Command(_))
    }

    /// What this seam does, for `/health` and startup logs.
    pub fn label(&self) -> &'static str {
        match self.backend {
            Backend::None => "none (vault assumed pristine)",
            Backend::Command(_) => "command",
        }
    }

    /// Fetch the current settlement log and rebuild the accumulator, verifying the
    /// reconstructed root against the vault's live `spentRoot` when the watcher
    /// reports it. The returned accumulator's `spent_root` equals the value the
    /// vault will check `spent_root_old` against in `fulfillBatch`.
    pub async fn synced_accumulator(&self) -> Result<RebuiltAccumulator, ChainSyncError> {
        let log = self.fetch_log().await?;
        rebuild_verified(&log).map_err(|e| ChainSyncError::Rebuild(e.to_string()))
    }

    async fn fetch_log(&self) -> Result<SettledLog, ChainSyncError> {
        match &self.backend {
            // No watcher: pristine vault. `rebuild_verified` over an empty log
            // yields `spent_root = 0` with no on-chain assertion.
            Backend::None => Ok(SettledLog::default()),
            Backend::Command(cmd) => run_events_command(cmd).await,
        }
    }
}

/// Rebuild the accumulator directly from an explicit batch list (fixtures/tests).
pub fn rebuild_accumulator(
    batches: &[bridge_return_host::s2::SettledBatch],
) -> bridge_return_host::Result<RebuiltAccumulator> {
    bridge_return_host::s2::rebuild(batches)
}

async fn run_events_command(cmd: &str) -> Result<SettledLog, ChainSyncError> {
    use std::process::Stdio;

    let output = tokio::process::Command::new("sh")
        .arg("-c")
        .arg(cmd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .await
        .map_err(|e| ChainSyncError::Spawn(e.to_string()))?;

    if !output.status.success() {
        return Err(ChainSyncError::CommandFailed {
            status: output.status.to_string(),
            stderr: String::from_utf8_lossy(&output.stderr).trim().to_string(),
        });
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let json: serde_json::Value = serde_json::from_str(stdout.trim()).map_err(|e| {
        ChainSyncError::Decode(format!(
            "events command stdout is not valid JSON ({e}); got: {}",
            snippet(stdout.trim()),
        ))
    })?;
    parse_settled_log(&json).map_err(|e| ChainSyncError::Decode(e.to_string()))
}

fn snippet(s: &str) -> String {
    if s.len() <= 200 {
        s.to_string()
    } else {
        format!("{}…", &s[..200])
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ChainSyncError {
    #[error("spawn events command failed: {0}")]
    Spawn(String),
    #[error("events command exited {status}: {stderr}")]
    CommandFailed { status: String, stderr: String },
    #[error("events log decode failed: {0}")]
    Decode(String),
    #[error("{0}")]
    Rebuild(String),
}
