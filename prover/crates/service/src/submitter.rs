//! S4 submitter — turns a proven batch bundle into an on-chain `fulfillBatch`
//! settlement (07 §B7). Pluggable so the proven relayer (or, in production, an
//! all-Rust Tron client) drops in without touching the queue:
//!
//! - `none` (default): do nothing — the return stays `proven`. The published
//!   bundle (`/batches/:id`) is **self-settleable** by anyone, so the principal is
//!   never stuck (06 §A1.2).
//! - `command`: run an operator-provided program (`BRIDGE_RETURN_SUBMIT_CMD`) with
//!   the bundle JSON on **stdin**; it submits `fulfillBatch` and prints the settle
//!   **txid** on stdout (exit 0). This is the integration seam for the existing
//!   `relayer.js settle` and for the future in-process Tron submitter.

use crate::store::BatchBundle;

#[derive(Clone)]
pub struct Submitter {
    backend: Backend,
}

#[derive(Clone)]
enum Backend {
    None,
    Command(String),
}

pub enum SubmitOutcome {
    /// No submitter configured, or no proof to settle: left at `proven`.
    Skipped,
    Submitted { txid: String },
    Failed { message: String },
}

impl Submitter {
    /// Build from `BRIDGE_RETURN_SUBMIT_CMD` (a shell command). Empty/unset = `none`.
    pub fn from_env() -> Self {
        match std::env::var("BRIDGE_RETURN_SUBMIT_CMD") {
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

    /// What this submitter does, for `/health`.
    pub fn label(&self) -> &'static str {
        match self.backend {
            Backend::None => "none",
            Backend::Command(_) => "command",
        }
    }

    pub async fn submit(&self, bundle: &BatchBundle) -> SubmitOutcome {
        match &self.backend {
            Backend::None => SubmitOutcome::Skipped,
            Backend::Command(cmd) => run_command(cmd, bundle).await,
        }
    }
}

async fn run_command(cmd: &str, bundle: &BatchBundle) -> SubmitOutcome {
    use std::process::Stdio;
    use tokio::io::AsyncWriteExt;

    // A no-proof bundle (precheck-only mode) can't settle on-chain.
    if bundle.proof_bytes == "0x" {
        return SubmitOutcome::Skipped;
    }

    // The whole published bundle — batchId, mode, vkey, publicValues, proofBytes,
    // plus the leaves/lockRefs fulfillBatch calldata (§B4) — same shape as
    // `GET /batches/:id`, so a self-settler and this command share one format.
    let payload = serde_json::to_string(bundle).expect("BatchBundle always serializes");

    tracing::debug!(batch_id = %bundle.batch_id, cmd, "spawning S4 submit command");

    let mut child = match tokio::process::Command::new("sh")
        .arg("-c")
        .arg(cmd)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
    {
        Ok(c) => c,
        Err(e) => {
            return SubmitOutcome::Failed {
                message: format!("spawn submit command failed: {e}"),
            }
        }
    };

    if let Some(mut stdin) = child.stdin.take() {
        if let Err(e) = stdin.write_all(payload.as_bytes()).await {
            return SubmitOutcome::Failed {
                message: format!("write to submit command failed: {e}"),
            };
        }
    }

    match child.wait_with_output().await {
        Ok(out) if out.status.success() => {
            let txid = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if txid.is_empty() {
                SubmitOutcome::Failed {
                    message: "submit command exited 0 but printed no txid".to_string(),
                }
            } else {
                SubmitOutcome::Submitted { txid }
            }
        }
        Ok(out) => SubmitOutcome::Failed {
            message: format!(
                "submit command exited {}: {}",
                out.status,
                String::from_utf8_lossy(&out.stderr).trim()
            ),
        },
        Err(e) => SubmitOutcome::Failed {
            message: format!("submit command wait failed: {e}"),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bundle(proof_bytes: &str) -> BatchBundle {
        BatchBundle {
            batch_id: "b".to_string(),
            mode: "precheck_only".to_string(),
            vkey: None,
            public_values: "0x".to_string(),
            proof_bytes: proof_bytes.to_string(),
            settle_txid: None,
            leaves: Vec::new(),
            lock_refs: Vec::new(),
        }
    }

    #[tokio::test]
    async fn a_bundle_without_proof_is_left_proven_even_with_a_submit_command() {
        let submitter = Submitter {
            backend: Backend::Command("false".to_string()),
        };
        assert!(matches!(
            submitter.submit(&bundle("0x")).await,
            SubmitOutcome::Skipped
        ));
    }
}
