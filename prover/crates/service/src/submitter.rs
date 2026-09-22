use std::process::Stdio;

use serde::Deserialize;
use tokio::io::AsyncWriteExt;

use crate::{
    domain::assembler::Rejected,
    store::{BatchBundle, LeafHex},
};

#[derive(Clone)]
pub struct Submitter {
    submit_cmd: Option<String>,
    simulate_cmd: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SubmitOutcome {
    Skipped,
    Submitted { txid: String },
    StaleRoot,
    Failed { message: String },
}

#[derive(Debug, thiserror::Error)]
pub enum SimulateError {
    #[error("spawn simulate command failed: {0}")]
    Spawn(String),
    #[error("simulate command exited {status}: {stderr}")]
    Command { status: String, stderr: String },
    #[error("simulate command output decode failed: {0}")]
    Decode(String),
}

#[derive(Deserialize)]
struct Simulation {
    #[serde(default)]
    rejected: Vec<Rejected>,
}

impl Submitter {
    pub fn from_env() -> Self {
        Self::with_commands(
            env_command("BRIDGE_RETURN_SUBMIT_CMD"),
            env_command("BRIDGE_RETURN_SIMULATE_CMD"),
        )
    }

    pub fn with_commands(submit_cmd: Option<String>, simulate_cmd: Option<String>) -> Self {
        Self {
            submit_cmd,
            simulate_cmd,
        }
    }

    pub fn none() -> Self {
        Self::with_commands(None, None)
    }

    pub fn label(&self) -> &'static str {
        match self.submit_cmd {
            None => "none",
            Some(_) => "command",
        }
    }

    pub async fn submit(&self, bundle: &BatchBundle) -> SubmitOutcome {
        match &self.submit_cmd {
            None => SubmitOutcome::Skipped,
            Some(cmd) => run_submit_command(cmd, bundle).await,
        }
    }

    pub async fn simulate(&self, leaves: &[LeafHex]) -> Result<Vec<Rejected>, SimulateError> {
        match &self.simulate_cmd {
            None => Ok(Vec::new()),
            Some(cmd) => run_simulate_command(cmd, leaves).await,
        }
    }
}

fn env_command(key: &str) -> Option<String> {
    std::env::var(key).ok().filter(|cmd| !cmd.trim().is_empty())
}

async fn run_submit_command(cmd: &str, bundle: &BatchBundle) -> SubmitOutcome {
    if bundle.proof_bytes == "0x" {
        return SubmitOutcome::Failed {
            message: "submit requested but the batch has no proof (prove_mode != sp1_groth16)"
                .to_string(),
        };
    }
    let payload = serde_json::to_string(bundle).expect("BatchBundle always serializes");
    tracing::debug!(batch_id = %bundle.batch_id, cmd, "spawning S4 submit command");
    let output = match run_with_stdin(cmd, payload.as_bytes()).await {
        Ok(output) => output,
        Err(e) => return SubmitOutcome::Failed { message: e },
    };
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    if !output.status.success() {
        if stderr.contains("stale root") {
            return SubmitOutcome::StaleRoot;
        }
        return SubmitOutcome::Failed {
            message: format!("submit command exited {}: {stderr}", output.status),
        };
    }
    let txid = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if txid.is_empty() {
        return SubmitOutcome::Failed {
            message: "submit command exited 0 but printed no txid".to_string(),
        };
    }
    SubmitOutcome::Submitted { txid }
}

async fn run_simulate_command(
    cmd: &str,
    leaves: &[LeafHex],
) -> Result<Vec<Rejected>, SimulateError> {
    let payload = serde_json::json!({ "leaves": leaves }).to_string();
    let output = run_with_stdin(cmd, payload.as_bytes())
        .await
        .map_err(SimulateError::Spawn)?;
    if !output.status.success() {
        return Err(SimulateError::Command {
            status: output.status.to_string(),
            stderr: String::from_utf8_lossy(&output.stderr).trim().to_string(),
        });
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    serde_json::from_str::<Simulation>(stdout.trim())
        .map(|s| s.rejected)
        .map_err(|e| SimulateError::Decode(format!("{e}; got: {}", snippet(stdout.trim()))))
}

async fn run_with_stdin(cmd: &str, payload: &[u8]) -> Result<std::process::Output, String> {
    let mut child = tokio::process::Command::new("sh")
        .arg("-c")
        .arg(cmd)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("spawn command failed: {e}"))?;
    if let Some(mut stdin) = child.stdin.take() {
        stdin
            .write_all(payload)
            .await
            .map_err(|e| format!("write to command failed: {e}"))?;
    }
    child
        .wait_with_output()
        .await
        .map_err(|e| format!("command wait failed: {e}"))
}

fn snippet(s: &str) -> String {
    if s.len() <= 200 {
        s.to_string()
    } else {
        format!("{}…", &s[..200])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bundle() -> BatchBundle {
        BatchBundle {
            batch_id: "0xb".to_string(),
            mode: "scripted".to_string(),
            vkey: None,
            public_values: "0xab".to_string(),
            proof_bytes: "0x01".to_string(),
            settle_txid: None,
            leaves: Vec::new(),
            lock_refs: Vec::new(),
        }
    }

    fn submitter(submit: &str) -> Submitter {
        Submitter::with_commands(Some(submit.to_string()), None)
    }

    fn simulator(simulate: &str) -> Submitter {
        Submitter::with_commands(None, Some(simulate.to_string()))
    }

    #[tokio::test]
    async fn submit_without_command_is_skipped() {
        assert_eq!(
            Submitter::none().submit(&bundle()).await,
            SubmitOutcome::Skipped
        );
        assert_eq!(Submitter::none().label(), "none");
    }

    #[tokio::test]
    async fn run_command_reports_the_txid() {
        let outcome = submitter("cat >/dev/null; echo 0xabc")
            .submit(&bundle())
            .await;
        assert_eq!(
            outcome,
            SubmitOutcome::Submitted {
                txid: "0xabc".to_string()
            }
        );
    }

    #[tokio::test]
    async fn run_command_maps_stale_root_stderr() {
        let outcome = submitter("cat >/dev/null; echo 'vault: stale root' >&2; exit 1")
            .submit(&bundle())
            .await;
        assert_eq!(outcome, SubmitOutcome::StaleRoot);
    }

    #[tokio::test]
    async fn run_command_reports_other_failures() {
        let outcome = submitter("cat >/dev/null; echo boom >&2; exit 1")
            .submit(&bundle())
            .await;
        assert!(matches!(outcome, SubmitOutcome::Failed { message } if message.contains("boom")));
    }

    #[tokio::test]
    async fn run_command_refuses_a_bundle_without_proof() {
        let mut no_proof = bundle();
        no_proof.proof_bytes = "0x".to_string();
        let outcome = submitter("cat >/dev/null; echo 0xabc")
            .submit(&no_proof)
            .await;
        assert!(matches!(outcome, SubmitOutcome::Failed { .. }));
    }

    #[tokio::test]
    async fn simulate_without_command_rejects_nothing() {
        assert_eq!(Submitter::none().simulate(&[]).await.unwrap(), Vec::new());
    }

    #[tokio::test]
    async fn run_simulate_command_parses_rejections() {
        let rejected = simulator(
            r#"cat >/dev/null; echo '{"rejected":[{"nullifier":"0x01","reason":"blocked"}]}'"#,
        )
        .simulate(&[])
        .await
        .unwrap();
        assert_eq!(
            rejected,
            vec![Rejected {
                nullifier: "0x01".to_string(),
                reason: "blocked".to_string()
            }]
        );
    }

    #[tokio::test]
    async fn run_simulate_command_reports_failures() {
        let err = simulator("cat >/dev/null; echo down >&2; exit 3")
            .simulate(&[])
            .await
            .unwrap_err();
        assert!(matches!(err, SimulateError::Command { stderr, .. } if stderr == "down"));
        let err = simulator("cat >/dev/null; echo nope")
            .simulate(&[])
            .await
            .unwrap_err();
        assert!(matches!(err, SimulateError::Decode(_)));
    }
}
