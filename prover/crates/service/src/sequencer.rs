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
//!    spent_root_new, txid }], "spent_root": <live vault spentRoot> }` — exactly what
//!    `relayer.js events` emits.
//!
//! A second command, `BRIDGE_RETURN_CONFIRM_CMD`, confirms releases the service holds no
//! receipt for against a source other than the one the log came from (see
//! [`ChainEvents::confirm_releases`]).

use bridge_return_host::s2::{
    parse_settled_log, rebuild_verified, RebuiltAccumulator, SettledBatch, SettledLog,
};
use serde::{Deserialize, Serialize};
use std::time::Duration;

use crate::submitter::run_with_stdin;

/// Chain-sync backend for the accumulator.
#[derive(Clone)]
pub struct ChainEvents {
    backend: Backend,
    confirm: Option<String>,
    timeout: Duration,
}

/// A release the vault's log shows: a spent nullifier and the transaction it names.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Release {
    pub nullifier: String,
    pub txid: String,
}

/// What the confirmation command found: the releases it confirmed, and why it did not
/// confirm the rest.
#[derive(Clone, Debug, Default, Eq, PartialEq, Deserialize)]
pub struct Confirmations {
    #[serde(default)]
    pub confirmed: Vec<Release>,
    #[serde(default)]
    pub unconfirmed: Vec<Unconfirmed>,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize)]
pub struct Unconfirmed {
    pub nullifier: String,
    pub txid: String,
    #[serde(default)]
    pub reason: String,
}

impl Release {
    fn is(&self, nullifier: &str, txid: &str) -> bool {
        self.nullifier.eq_ignore_ascii_case(nullifier) && self.txid.eq_ignore_ascii_case(txid)
    }
}

impl Confirmations {
    /// `Ok` only for a release confirmed exactly as asked: the same nullifier in the same
    /// transaction. Otherwise the reason the source gave, or that it gave none.
    pub fn verdict(&self, release: &Release) -> Result<(), String> {
        if self
            .confirmed
            .iter()
            .any(|c| c.is(&release.nullifier, &release.txid))
        {
            return Ok(());
        }
        Err(self
            .unconfirmed
            .iter()
            .find(|u| {
                u.nullifier.eq_ignore_ascii_case(&release.nullifier)
                    && u.txid.eq_ignore_ascii_case(&release.txid)
            })
            .map(|u| u.reason.clone())
            .filter(|reason| !reason.is_empty())
            .unwrap_or_else(|| "the confirmation source did not confirm it".to_string()))
    }
}

#[derive(Clone)]
enum Backend {
    None,
    Command(String),
}

impl ChainEvents {
    /// Build from `BRIDGE_RETURN_EVENTS_CMD` (a shell command; empty/unset = `none`) and
    /// `BRIDGE_RETURN_CONFIRM_CMD`.
    pub fn from_env() -> Self {
        let command = |key: &str| std::env::var(key).ok().filter(|cmd| !cmd.trim().is_empty());
        let backend = match command("BRIDGE_RETURN_EVENTS_CMD") {
            Some(cmd) => Backend::Command(cmd),
            None => Backend::None,
        };
        Self {
            backend,
            confirm: command("BRIDGE_RETURN_CONFIRM_CMD"),
            timeout: Duration::from_secs(600),
        }
    }

    pub fn none() -> Self {
        Self {
            backend: Backend::None,
            confirm: None,
            timeout: Duration::from_secs(600),
        }
    }

    pub fn with_timeout(self, timeout: Duration) -> Self {
        Self { timeout, ..self }
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

    /// How releases the service did not see settle are confirmed, for startup logs.
    pub fn confirm_label(&self) -> &'static str {
        match self.confirm {
            None => "none (a release the service holds no receipt for never settles a return)",
            Some(_) => "command",
        }
    }

    /// Ask the confirmation command, which reads a source other than the events
    /// command's, whether each release happened: `{"releases": [{nullifier, txid}]}` on
    /// stdin, `{"confirmed": [{nullifier, txid}], "unconfirmed": [{nullifier, txid,
    /// reason}]}` on stdout. Only an exact match in `confirmed` confirms a release.
    pub async fn confirm_releases(
        &self,
        releases: &[Release],
    ) -> Result<Confirmations, ChainSyncError> {
        let Some(cmd) = &self.confirm else {
            return Err(ChainSyncError::NoConfirmationSource);
        };
        let payload = serde_json::json!({ "releases": releases }).to_string();
        let output = run_with_stdin(cmd, payload.as_bytes(), self.timeout)
            .await
            .map_err(ChainSyncError::Spawn)?;
        if !output.status.success() {
            return Err(ChainSyncError::CommandFailed {
                status: output.status.to_string(),
                stderr: String::from_utf8_lossy(&output.stderr).trim().to_string(),
            });
        }
        let stdout = String::from_utf8_lossy(&output.stdout);
        serde_json::from_str(stdout.trim()).map_err(|e| {
            ChainSyncError::Decode(format!(
                "confirm command stdout is not the expected JSON ({e}); got: {}",
                snippet(stdout.trim()),
            ))
        })
    }

    /// Fetch the current settlement log and rebuild the accumulator, verifying the
    /// reconstructed root against the vault's live `spentRoot` when the watcher
    /// reports it. The returned accumulator's `spent_root` equals the value the
    /// vault will check `spent_root_old` against in `fulfillBatch`.
    pub async fn synced_accumulator(
        &self,
        known: Vec<SettledBatch>,
    ) -> Result<RebuiltAccumulator, ChainSyncError> {
        let mut log = self.fetch_log().await?;
        log.batches.extend(known);
        rebuild_verified(&log).map_err(|e| ChainSyncError::Rebuild(e.to_string()))
    }

    async fn fetch_log(&self) -> Result<SettledLog, ChainSyncError> {
        match &self.backend {
            // No watcher: pristine vault. `rebuild_verified` over an empty log
            // yields `spent_root = 0` with no on-chain assertion.
            Backend::None => Ok(SettledLog::default()),
            Backend::Command(cmd) => run_events_command(cmd, self.timeout).await,
        }
    }
}

/// Rebuild the accumulator directly from an explicit batch list (fixtures/tests).
pub fn rebuild_accumulator(
    batches: &[bridge_return_host::s2::SettledBatch],
) -> bridge_return_host::Result<RebuiltAccumulator> {
    bridge_return_host::s2::rebuild(batches)
}

async fn run_events_command(cmd: &str, timeout: Duration) -> Result<SettledLog, ChainSyncError> {
    use std::process::Stdio;

    let child = tokio::process::Command::new("sh")
        .arg("-c")
        .arg(cmd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .output();
    let output = tokio::time::timeout(timeout, child)
        .await
        .map_err(|_| {
            ChainSyncError::Spawn(format!(
                "events command timed out after {}s",
                timeout.as_secs()
            ))
        })?
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
    #[error("no confirmation source is configured (BRIDGE_RETURN_CONFIRM_CMD)")]
    NoConfirmationSource,
}

impl ChainSyncError {
    /// The log could not be read at all: the command did not start, timed out or exited
    /// non-zero, which is what one failed RPC request makes it do. A log that was read but
    /// does not decode or does not reproduce the vault's root is not this.
    pub fn is_unavailable(&self) -> bool {
        matches!(self, Self::Spawn(_) | Self::CommandFailed { .. })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn events(cmd: &str) -> ChainEvents {
        ChainEvents {
            backend: Backend::Command(cmd.to_string()),
            confirm: None,
            timeout: Duration::from_secs(5),
        }
    }

    fn confirming(cmd: &str) -> ChainEvents {
        ChainEvents {
            confirm: Some(cmd.to_string()),
            ..ChainEvents::none().with_timeout(Duration::from_secs(5))
        }
    }

    fn release(nullifier: &str, txid: &str) -> Release {
        Release {
            nullifier: nullifier.to_string(),
            txid: txid.to_string(),
        }
    }

    #[tokio::test]
    async fn only_a_release_confirmed_in_the_same_transaction_is_confirmed() {
        let confirmations = confirming(
            r#"grep -qF '"txid":"0xaa"' && echo '{"confirmed":[{"nullifier":"0x01","txid":"0xAA"},{"nullifier":"0x02","txid":"0xcc"}],"unconfirmed":[{"nullifier":"0x03","txid":"0xbb","reason":"no such transaction"}]}'"#,
        )
        .confirm_releases(&[release("0x01", "0xaa"), release("0x02", "0xbb")])
        .await
        .unwrap();
        assert_eq!(confirmations.verdict(&release("0x01", "0xaa")), Ok(()));
        assert!(confirmations.verdict(&release("0x02", "0xbb")).is_err());
        assert_eq!(
            confirmations.verdict(&release("0x03", "0xbb")),
            Err("no such transaction".to_string())
        );
    }

    #[tokio::test]
    async fn without_a_confirmation_command_nothing_is_confirmed() {
        let err = ChainEvents::none()
            .confirm_releases(&[release("0x01", "0xaa")])
            .await
            .unwrap_err();
        assert!(matches!(err, ChainSyncError::NoConfirmationSource), "{err}");
        let err = confirming("cat >/dev/null; echo 'ETH_CONFIRM_RPC_URL not set' >&2; exit 1")
            .confirm_releases(&[release("0x01", "0xaa")])
            .await
            .unwrap_err();
        assert!(err.to_string().contains("ETH_CONFIRM_RPC_URL"), "{err}");
        let err = confirming("cat >/dev/null; echo nope")
            .confirm_releases(&[release("0x01", "0xaa")])
            .await
            .unwrap_err();
        assert!(matches!(err, ChainSyncError::Decode(_)), "{err}");
    }

    #[tokio::test]
    async fn a_command_that_cannot_reach_the_chain_leaves_the_log_unavailable() {
        let err = events("echo 'request timeout' >&2; exit 1")
            .synced_accumulator(Vec::new())
            .await
            .unwrap_err();
        assert!(err.is_unavailable(), "{err}");
        let err = events("sleep 5")
            .with_timeout(Duration::from_millis(200))
            .synced_accumulator(Vec::new())
            .await
            .unwrap_err();
        assert!(err.is_unavailable(), "{err}");
    }

    #[tokio::test]
    async fn a_log_that_was_read_but_is_unusable_is_not_unavailable() {
        let err = events("echo nope")
            .synced_accumulator(Vec::new())
            .await
            .unwrap_err();
        assert!(matches!(err, ChainSyncError::Decode(_)), "{err}");
        assert!(!err.is_unavailable());
        let root = format!("0x{}", "01".repeat(32));
        let err = events(&format!(r#"echo '{{"batches":[],"spent_root":"{root}"}}'"#))
            .synced_accumulator(Vec::new())
            .await
            .unwrap_err();
        assert!(matches!(err, ChainSyncError::Rebuild(_)), "{err}");
        assert!(!err.is_unavailable());
    }
}
