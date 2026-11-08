use std::{process::Stdio, time::Duration};

use serde::Deserialize;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};

use crate::{
    domain::{assembler::Rejected, burn::LeafToSettle},
    store::BatchBundle,
};

#[derive(Clone)]
pub struct Submitter {
    submit_cmd: Option<String>,
    simulate_cmd: Option<String>,
    timeout: Duration,
}

/// The submit command's exit status for a settlement it broadcast but did not see settle
/// (EX_TEMPFAIL): the service waits on the txid it printed.
pub const UNCONFIRMED_EXIT: i32 = 75;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SubmitOutcome {
    Skipped,
    /// Settled with the confirmations the submitter waits for.
    Submitted {
        txid: String,
    },
    /// The submitter printed the txid of a transaction it signed, and did not report it
    /// settled: the transaction may have settled, be waiting to, have failed, or never have
    /// gone out, which only a look at the chain tells.
    Unconfirmed {
        txid: String,
    },
    StaleRoot,
    Failed {
        message: String,
    },
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
            timeout: Duration::from_secs(600),
        }
    }

    pub fn with_timeout(self, timeout: Duration) -> Self {
        Self { timeout, ..self }
    }

    pub fn none() -> Self {
        Self::with_commands(None, None)
    }

    /// The simulation is what checks each burn's lock refs against the vault before
    /// proving, so a service that admits burns refuses to run without it.
    pub fn require_simulation(self) -> Result<Self, String> {
        if self.simulate_cmd.is_none() {
            return Err(
                "BRIDGE_RETURN_SIMULATE_CMD is required: it checks each burn's lock refs \
                        against the vault before proving"
                    .to_string(),
            );
        }
        Ok(self)
    }

    /// Whether a submit command is configured.
    pub fn submits(&self) -> bool {
        self.submit_cmd.is_some()
    }

    pub fn label(&self) -> &'static str {
        match self.submit_cmd {
            None => "none",
            Some(_) => "command",
        }
    }

    /// Run the submit command on `bundle`. `signed` is called with the txid as soon as the
    /// command prints it, which the relayers do once they have signed the transaction and
    /// before they send it, so the service can journal it while the command still runs.
    pub async fn submit(
        &self,
        bundle: &BatchBundle,
        signed: &(dyn Fn(&str) + Send + Sync),
    ) -> SubmitOutcome {
        match &self.submit_cmd {
            None => SubmitOutcome::Skipped,
            Some(cmd) => run_submit_command(cmd, bundle, self.timeout, signed).await,
        }
    }

    pub async fn simulate(&self, leaves: &[LeafToSettle]) -> Result<Vec<Rejected>, SimulateError> {
        match &self.simulate_cmd {
            None => Ok(Vec::new()),
            Some(cmd) => run_simulate_command(cmd, leaves, self.timeout).await,
        }
    }
}

fn env_command(key: &str) -> Option<String> {
    std::env::var(key).ok().filter(|cmd| !cmd.trim().is_empty())
}

/// The first line the command prints is the settlement's txid, handed to `signed` the moment
/// it arrives. Once there is one, only exit 0 (settled) or a stale root decides the outcome
/// by the exit status: a timeout, exit 75 or any other failure leaves the transaction to be
/// looked up, since it may have gone out whatever made the command stop.
async fn run_submit_command(
    cmd: &str,
    bundle: &BatchBundle,
    timeout: Duration,
    signed: &(dyn Fn(&str) + Send + Sync),
) -> SubmitOutcome {
    if !bundle.has_proof() {
        return SubmitOutcome::Skipped;
    }
    let payload = serde_json::to_string(bundle).expect("BatchBundle always serializes");
    tracing::debug!(batch_id = %bundle.batch_id, cmd, "spawning S4 submit command");
    let mut child = match spawn_with_stdin(cmd, payload.as_bytes()).await {
        Ok(child) => child,
        Err(message) => return SubmitOutcome::Failed { message },
    };
    let stdout = child.stdout.take().expect("stdout is piped");
    let mut stderr = child.stderr.take().expect("stderr is piped");
    let stderr = tokio::spawn(async move {
        let mut text = String::new();
        let _ = stderr.read_to_string(&mut text).await;
        text
    });
    let mut txid: Option<String> = None;
    let finished = tokio::time::timeout(timeout, async {
        let mut lines = BufReader::new(stdout).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            let line = line.trim();
            if txid.is_none() && !line.is_empty() {
                txid = Some(line.to_string());
                signed(line);
            }
        }
        child.wait().await
    })
    .await;
    if finished.is_err() {
        let _ = child.start_kill();
    }
    let stderr = tokio::time::timeout(Duration::from_secs(5), stderr)
        .await
        .ok()
        .and_then(Result::ok)
        .unwrap_or_default()
        .trim()
        .to_string();
    let status = match finished {
        Ok(Ok(status)) => status,
        Ok(Err(e)) => {
            return match txid {
                Some(txid) => SubmitOutcome::Unconfirmed { txid },
                None => SubmitOutcome::Failed {
                    message: format!("command wait failed: {e}"),
                },
            }
        }
        Err(_) => {
            return match txid {
                Some(txid) => SubmitOutcome::Unconfirmed { txid },
                None => SubmitOutcome::Failed {
                    message: format!("command timed out after {}s", timeout.as_secs()),
                },
            }
        }
    };
    if status.success() {
        return match txid {
            Some(txid) => SubmitOutcome::Submitted { txid },
            None => SubmitOutcome::Failed {
                message: "submit command exited 0 but printed no txid".to_string(),
            },
        };
    }
    if stderr.contains("stale root") {
        return SubmitOutcome::StaleRoot;
    }
    match txid {
        Some(txid) => {
            if status.code() != Some(UNCONFIRMED_EXIT) {
                tracing::warn!(batch_id = %bundle.batch_id, %txid, %status, %stderr, "submit command failed after signing; the transaction is looked up");
            }
            SubmitOutcome::Unconfirmed { txid }
        }
        None => SubmitOutcome::Failed {
            message: format!("submit command exited {status}: {stderr}"),
        },
    }
}

async fn run_simulate_command(
    cmd: &str,
    leaves: &[LeafToSettle],
    timeout: Duration,
) -> Result<Vec<Rejected>, SimulateError> {
    let payload = serde_json::json!({ "leaves": leaves }).to_string();
    let output = run_with_stdin(cmd, payload.as_bytes(), timeout)
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

async fn spawn_with_stdin(cmd: &str, payload: &[u8]) -> Result<tokio::process::Child, String> {
    let mut child = tokio::process::Command::new("sh")
        .arg("-c")
        .arg(cmd)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .map_err(|e| format!("spawn command failed: {e}"))?;
    if let Some(mut stdin) = child.stdin.take() {
        stdin
            .write_all(payload)
            .await
            .map_err(|e| format!("write to command failed: {e}"))?;
    }
    Ok(child)
}

pub(crate) async fn run_with_stdin(
    cmd: &str,
    payload: &[u8],
    timeout: Duration,
) -> Result<std::process::Output, String> {
    let child = spawn_with_stdin(cmd, payload).await?;
    match tokio::time::timeout(timeout, child.wait_with_output()).await {
        Ok(output) => output.map_err(|e| format!("command wait failed: {e}")),
        Err(_) => Err(format!("command timed out after {}s", timeout.as_secs())),
    }
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
    use crate::{
        domain::assembler::RejectionKind,
        store::{LeafHex, LockRefHex},
    };

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
    async fn run_command_is_cut_off_after_the_timeout() {
        let outcome = submitter("sleep 5")
            .with_timeout(Duration::from_millis(200))
            .submit(&bundle(), &|_| {})
            .await;
        assert!(
            matches!(&outcome, SubmitOutcome::Failed { message } if message.contains("timed out")),
            "{outcome:?}"
        );
    }

    #[tokio::test]
    async fn submit_without_command_is_skipped() {
        assert_eq!(
            Submitter::none().submit(&bundle(), &|_| {}).await,
            SubmitOutcome::Skipped
        );
        assert_eq!(Submitter::none().label(), "none");
    }

    #[tokio::test]
    async fn run_command_reports_the_txid() {
        let outcome = submitter("cat >/dev/null; echo 0xabc")
            .submit(&bundle(), &|_| {})
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
            .submit(&bundle(), &|_| {})
            .await;
        assert_eq!(outcome, SubmitOutcome::StaleRoot);
    }

    #[tokio::test]
    async fn a_settlement_broadcast_without_a_receipt_is_unconfirmed_not_failed() {
        let outcome =
            submitter("cat >/dev/null; echo 0xabc; echo 'receipt unreadable' >&2; exit 75")
                .submit(&bundle(), &|_| {})
                .await;
        assert_eq!(
            outcome,
            SubmitOutcome::Unconfirmed {
                txid: "0xabc".to_string()
            }
        );
        let outcome = submitter("cat >/dev/null; echo 'nothing broadcast' >&2; exit 75")
            .submit(&bundle(), &|_| {})
            .await;
        assert!(
            matches!(&outcome, SubmitOutcome::Failed { message } if message.contains("nothing broadcast")),
            "{outcome:?}"
        );
    }

    #[tokio::test]
    async fn a_txid_printed_before_the_command_stops_leaves_the_settlement_to_be_looked_up() {
        let seen = std::sync::Mutex::new(Vec::new());
        let signed = |txid: &str| seen.lock().unwrap().push(txid.to_string());
        let hung = submitter("cat >/dev/null; echo 0xabc; sleep 5; echo 0xdef")
            .with_timeout(Duration::from_millis(300))
            .submit(&bundle(), &signed)
            .await;
        assert_eq!(
            hung,
            SubmitOutcome::Unconfirmed {
                txid: "0xabc".to_string()
            }
        );
        let failed = submitter("cat >/dev/null; echo 0xabc; echo 'reverted on chain' >&2; exit 1")
            .submit(&bundle(), &signed)
            .await;
        assert_eq!(
            failed,
            SubmitOutcome::Unconfirmed {
                txid: "0xabc".to_string()
            }
        );
        let stale = submitter("cat >/dev/null; echo 0xabc; echo 'vault: stale root' >&2; exit 1")
            .submit(&bundle(), &signed)
            .await;
        assert_eq!(stale, SubmitOutcome::StaleRoot);
        assert_eq!(*seen.lock().unwrap(), vec!["0xabc"; 3]);
    }

    #[tokio::test]
    async fn the_txid_is_handed_over_while_the_command_still_runs() {
        let (tx, rx) = std::sync::mpsc::channel();
        let signed = move |txid: &str| {
            tx.send((txid.to_string(), std::time::Instant::now()))
                .unwrap()
        };
        let started = std::time::Instant::now();
        let outcome = submitter("cat >/dev/null; echo 0xabc; sleep 1")
            .submit(&bundle(), &signed)
            .await;
        assert_eq!(
            outcome,
            SubmitOutcome::Submitted {
                txid: "0xabc".to_string()
            }
        );
        let (txid, at) = rx.recv().unwrap();
        assert_eq!(txid, "0xabc");
        assert!(at.duration_since(started) < Duration::from_millis(800));
        assert!(started.elapsed() >= Duration::from_secs(1));
    }

    #[tokio::test]
    async fn run_command_reports_other_failures() {
        let outcome = submitter("cat >/dev/null; echo boom >&2; exit 1")
            .submit(&bundle(), &|_| {})
            .await;
        assert!(matches!(outcome, SubmitOutcome::Failed { message } if message.contains("boom")));
    }

    #[tokio::test]
    async fn a_bundle_without_proof_is_left_proven_even_with_a_submit_command() {
        let mut no_proof = bundle();
        no_proof.proof_bytes = "0x".to_string();
        let outcome = submitter("cat >/dev/null; echo 0xabc")
            .submit(&no_proof, &|_| {})
            .await;
        assert_eq!(outcome, SubmitOutcome::Skipped);
    }

    #[test]
    fn simulation_is_required_by_a_service_that_admits_burns() {
        let Err(err) = Submitter::none().require_simulation() else {
            panic!("a submitter without simulation passed");
        };
        assert!(err.contains("BRIDGE_RETURN_SIMULATE_CMD"), "{err}");
        assert!(simulator("true").require_simulation().is_ok());
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
                reason: "blocked".to_string(),
                kind: RejectionKind::Recipient,
            }]
        );
    }

    #[tokio::test]
    async fn run_simulate_command_reads_a_lock_rejection() {
        let rejected = simulator(
            r#"cat >/dev/null; echo '{"rejected":[{"nullifier":"0x01","reason":"no lock","kind":"lock"}]}'"#,
        )
        .simulate(&[])
        .await
        .unwrap();
        assert_eq!(rejected[0].kind, RejectionKind::Lock);
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

    #[tokio::test]
    async fn run_simulate_command_hands_each_leaf_its_lock_refs() {
        let leaf = LeafToSettle {
            leaf: LeafHex {
                nullifier: "0x01".to_string(),
                recipient: "0xaa".to_string(),
                amount: "0x01".to_string(),
                fee_recipient: "0x00".to_string(),
                fee_amount: "0x00".to_string(),
                deadline: "0x00".to_string(),
            },
            lock_refs: vec![LockRefHex {
                nonce: "0x7".to_string(),
                digest: "0xab".to_string(),
            }],
        };
        let rejected = simulator(
            r#"grep -qF '"nonce":"0x7"' && echo '{"rejected":[]}' || echo '{"rejected":[{"nullifier":"0x01","reason":"lock refs missing from the payload"}]}'"#,
        )
        .simulate(&[leaf])
        .await
        .unwrap();
        assert_eq!(rejected, Vec::new());
    }
}
