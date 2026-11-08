mod support;

use std::time::Duration;

use bridge_return_service::{
    clock::Clock,
    config::ServiceConfig,
    domain::{
        assembler::RejectionKind, batch::BatchStatus, policy::BatchPolicy, retry::RetryPolicy,
    },
    orchestrator::Orchestrator,
    sequencer::{Release, SettlementState},
    shutdown::{self, Drain, StopSignal, Stopped},
    store::{ErrorKind, ReturnRecord, ReturnStatus, ReturnStore},
    submitter::SubmitOutcome,
};
use support::{
    burn_and_record, hex, member, nullifier_of, FakeChainLog, ProveStep, Receipts, ScriptedProver,
    ScriptedSettler, SecondSource,
};
use tokio::sync::mpsc;

const HOUR: Duration = Duration::from_secs(3_600);
const MINUTE: u128 = 60_000;

struct Harness {
    store: ReturnStore,
    prover: ScriptedProver,
    settler: ScriptedSettler,
    chain: FakeChainLog,
    clock: Clock,
    drain: Drain,
    task: tokio::task::JoinHandle<()>,
}

fn config(idle_wait_secs: u64, retry: RetryPolicy) -> ServiceConfig {
    ServiceConfig {
        idle_wait: Duration::from_secs(idle_wait_secs),
        max_batch_size: 8,
        retry,
        ..ServiceConfig::default()
    }
}

fn retry(max_attempts: u32, max_rebases: u32) -> RetryPolicy {
    RetryPolicy {
        base: Duration::from_secs(60),
        max_attempts,
        max_rebases,
    }
}

impl Harness {
    fn start(
        store: ReturnStore,
        config: &ServiceConfig,
        prover: ScriptedProver,
        settler: ScriptedSettler,
        chain: FakeChainLog,
    ) -> Self {
        let clock = Clock::default();
        let drain = Drain::default();
        let task = tokio::spawn(
            Orchestrator::new(
                store.clone(),
                prover.clone(),
                settler.clone(),
                chain.clone(),
                BatchPolicy::from(config),
                config.retry,
                clock.clone(),
            )
            .run(drain.clone()),
        );
        Self {
            store,
            prover,
            settler,
            chain,
            clock,
            drain,
            task,
        }
    }

    fn simple(prover: ScriptedProver, settler: ScriptedSettler) -> Self {
        Self::with_retry(prover, settler, RetryPolicy::default())
    }

    fn with_retry(prover: ScriptedProver, settler: ScriptedSettler, retry: RetryPolicy) -> Self {
        Self::start(
            ReturnStore::memory(retry),
            &config(0, retry),
            prover,
            settler,
            FakeChainLog::new(),
        )
    }

    fn accept(&self, id: &str, nonce: u64, salt: u8) -> [u8; 32] {
        let input = member(nonce, salt);
        let (burn, record) = burn_and_record(id, &input, self.clock.now_ms());
        self.store.accept(burn, record, self.clock.now_ms());
        nullifier_of(&input)
    }

    fn record(&self, id: &str) -> ReturnRecord {
        self.store.get(id).unwrap()
    }

    fn status(&self, id: &str) -> ReturnStatus {
        self.record(id).status
    }

    fn batch_of(&self, id: &str) -> bridge_return_service::domain::batch::Batch {
        let batch_id = self.record(id).batch_id.unwrap();
        self.store
            .read(|ledger| ledger.batch(&batch_id).cloned())
            .unwrap()
    }

    async fn reach(&self, id: &str, status: ReturnStatus) {
        let store = self.store.clone();
        let (key, wanted) = (id.to_string(), status.clone());
        yield_until(move || store.get(&key).unwrap().status == wanted).await;
        assert_eq!(self.status(id), status);
    }

    async fn reach_in(&self, id: &str, status: ReturnStatus, max: Duration) {
        let store = self.store.clone();
        let (key, wanted) = (id.to_string(), status.clone());
        advance_until(move || store.get(&key).unwrap().status == wanted, max).await;
        assert_eq!(self.status(id), status);
    }

    async fn stop(self) {
        self.task.abort();
        let _ = self.task.await;
    }

    /// Run the service's stop sequence for `signals`, sent before it starts.
    async fn stop_with(&mut self, signals: &[StopSignal], timeout: Duration) -> Stopped {
        let (tx, mut rx) = mpsc::channel(4);
        for signal in signals {
            tx.send(*signal).await.unwrap();
        }
        let stopping = shutdown::stop(
            &mut rx,
            &self.drain,
            &mut self.task,
            timeout,
            &self.store,
            &self.clock,
        );
        let stopped = tokio::time::timeout(Duration::from_secs(86_400), stopping)
            .await
            .expect("the service never stopped");
        assert!(self.task.is_finished());
        stopped
    }
}

async fn yield_until(condition: impl Fn() -> bool) {
    for _ in 0..20_000 {
        if condition() {
            return;
        }
        tokio::task::yield_now().await;
        std::thread::sleep(Duration::from_millis(1));
    }
    panic!("condition not reached without time passing");
}

async fn advance_until(condition: impl Fn() -> bool, max: Duration) {
    let step = Duration::from_secs(30);
    let mut elapsed = Duration::ZERO;
    while elapsed <= max {
        for _ in 0..10 {
            if condition() {
                return;
            }
            tokio::task::yield_now().await;
            std::thread::sleep(Duration::from_millis(1));
        }
        tokio::time::sleep(step).await;
        elapsed += step;
    }
    panic!("condition not reached within {max:?}");
}

#[tokio::test(start_paused = true)]
async fn idle_service_proves_one_burn_at_once() {
    let h = Harness::simple(
        ScriptedProver::instant(),
        ScriptedSettler::settling("0xfeed"),
    );
    h.accept("a", 11, 0x61);
    h.reach("a", ReturnStatus::Settled).await;
    let record = h.record("a");
    assert_eq!(record.settle_txid.as_deref(), Some("0xfeed"));
    assert!(record.terminal);
    assert_eq!(h.prover.calls(), 1);
    assert_eq!(h.store.health().queue_depth, 0);
    assert_eq!(h.batch_of("a").status, BatchStatus::Settled);
}

#[tokio::test(start_paused = true)]
async fn burns_arriving_during_a_proof_form_the_next_batch() {
    let prover = ScriptedProver::new(vec![
        ProveStep::Succeed { delay: HOUR },
        ProveStep::Succeed { delay: HOUR },
    ]);
    let h = Harness::simple(prover, ScriptedSettler::settling("0xfeed"));
    h.accept("a", 11, 0x61);
    h.reach("a", ReturnStatus::Proving).await;
    h.accept("b", 12, 0x62);
    h.accept("c", 13, 0x63);
    tokio::task::yield_now().await;
    assert_eq!(h.status("b"), ReturnStatus::Queued);
    assert_eq!(h.record("b").queue_position, Some(1));
    assert_eq!(h.record("c").queue_position, Some(2));
    let health = h.store.health();
    assert_eq!(health.active_batch, h.record("a").batch_id);
    assert_eq!(health.active_batch_size, 1);
    assert_eq!(health.queue_depth, 2);

    h.reach_in("a", ReturnStatus::Settled, 2 * HOUR).await;
    h.reach("b", ReturnStatus::Proving).await;
    assert_eq!(h.status("c"), ReturnStatus::Proving);
    assert_eq!(h.record("b").batch_id, h.record("c").batch_id);
    assert_ne!(h.record("b").batch_id, h.record("a").batch_id);

    h.reach_in("b", ReturnStatus::Settled, 2 * HOUR).await;
    assert_eq!(h.status("c"), ReturnStatus::Settled);
    let submitted = h.settler.submitted();
    assert_eq!(submitted.len(), 2);
    assert_eq!(submitted[1].leaves.len(), 2);
    assert_eq!(h.prover.calls(), 2);
}

#[tokio::test(start_paused = true)]
async fn idle_window_delays_only_an_idle_start() {
    let prover = ScriptedProver::new(vec![
        ProveStep::Succeed { delay: HOUR },
        ProveStep::Succeed { delay: HOUR },
    ]);
    let h = Harness::start(
        ReturnStore::default(),
        &config(300, RetryPolicy::default()),
        prover,
        ScriptedSettler::settling("0xfeed"),
        FakeChainLog::new(),
    );
    h.accept("a", 11, 0x61);
    tokio::time::sleep(Duration::from_secs(200)).await;
    assert_eq!(h.status("a"), ReturnStatus::Queued);
    h.reach_in("a", ReturnStatus::Proving, Duration::from_secs(200))
        .await;

    h.accept("b", 12, 0x62);
    tokio::task::yield_now().await;
    assert_eq!(h.status("b"), ReturnStatus::Queued);
    h.reach_in("a", ReturnStatus::Settled, 2 * HOUR).await;
    h.reach("b", ReturnStatus::Proving).await;
    h.reach_in("b", ReturnStatus::Settled, 2 * HOUR).await;
}

#[tokio::test(start_paused = true)]
async fn prover_failure_fails_members_recoverably_and_retries_after_backoff() {
    let prover = ScriptedProver::new(vec![ProveStep::Fail("oom".to_string())]);
    let h = Harness::simple(prover, ScriptedSettler::settling("0xfeed"));
    h.accept("a", 11, 0x61);
    h.accept("b", 12, 0x62);
    h.reach("a", ReturnStatus::Failed).await;
    for id in ["a", "b"] {
        let record = h.record(id);
        assert_eq!(record.status, ReturnStatus::Failed);
        assert_eq!(record.attempts, 1);
        assert_eq!(record.not_before_ms, Some(record.updated_at_ms + MINUTE));
        let failure = record.failure.unwrap();
        assert!(failure.recoverable);
        assert_eq!(failure.kind, ErrorKind::ProvingFailed);
        assert!(failure.message.contains("oom"));
    }

    h.accept("c", 13, 0x63);
    h.reach("c", ReturnStatus::Settled).await;
    assert_eq!(h.status("a"), ReturnStatus::Failed);

    tokio::time::sleep(Duration::from_secs(30)).await;
    assert_eq!(h.status("a"), ReturnStatus::Failed);
    h.reach_in("a", ReturnStatus::Settled, Duration::from_secs(120))
        .await;
    assert_eq!(h.status("b"), ReturnStatus::Settled);
    assert_eq!(h.record("a").batch_id, h.record("b").batch_id);
    assert_eq!(h.prover.calls(), 3);
}

#[tokio::test(start_paused = true)]
async fn parked_after_max_attempts() {
    let prover = ScriptedProver::new(vec![
        ProveStep::Fail("oom".to_string()),
        ProveStep::Fail("oom".to_string()),
    ]);
    let h = Harness::with_retry(prover, ScriptedSettler::settling("0xfeed"), retry(2, 3));
    h.accept("a", 11, 0x61);
    h.reach("a", ReturnStatus::Failed).await;
    assert!(h.record("a").failure.unwrap().recoverable);
    advance_until(
        {
            let store = h.store.clone();
            move || {
                store
                    .get("a")
                    .unwrap()
                    .failure
                    .is_some_and(|f| !f.recoverable)
            }
        },
        Duration::from_secs(300),
    )
    .await;
    let record = h.record("a");
    assert_eq!(record.attempts, 2);
    assert!(record.failure.unwrap().message.contains("parked"));
    assert_eq!(h.store.health().queue_depth, 0);
    assert_eq!(h.prover.calls(), 2);
}

#[tokio::test(start_paused = true)]
async fn requeue_keeps_the_schedule() {
    let prover = ScriptedProver::new(vec![ProveStep::Fail("oom".to_string())]);
    let h = Harness::simple(prover, ScriptedSettler::settling("0xfeed"));
    let input = member(11, 0x61);
    let (burn, record) = burn_and_record("a", &input, h.clock.now_ms());
    h.store.accept(burn, record, h.clock.now_ms());
    h.reach("a", ReturnStatus::Failed).await;
    let not_before = h.record("a").not_before_ms;

    let (burn, record) = burn_and_record("a", &input, h.clock.now_ms());
    let (requeued, inserted) = h.store.accept(burn, record, h.clock.now_ms());
    assert!(inserted);
    assert_eq!(requeued.status, ReturnStatus::Queued);
    assert_eq!(requeued.attempts, 1);
    assert_eq!(requeued.not_before_ms, not_before);
    tokio::task::yield_now().await;
    assert_eq!(h.status("a"), ReturnStatus::Queued);
    assert_eq!(h.prover.calls(), 1);
    h.reach_in("a", ReturnStatus::Settled, Duration::from_secs(120))
        .await;
    assert_eq!(h.prover.calls(), 2);
}

#[tokio::test(start_paused = true)]
async fn root_moved_during_proof_rebases_once_and_proves_again() {
    let prover = ScriptedProver::new(vec![
        ProveStep::Succeed { delay: HOUR },
        ProveStep::Succeed { delay: HOUR },
    ]);
    let h = Harness::simple(prover, ScriptedSettler::settling("0xfeed"));
    h.accept("a", 11, 0x61);
    h.reach("a", ReturnStatus::Proving).await;
    h.chain
        .settle_externally(&[nullifier_of(&member(99, 0x70))]);
    h.reach_in("a", ReturnStatus::Settled, 3 * HOUR).await;
    let batch = h.batch_of("a");
    assert_eq!(batch.rebases, 1);
    assert_eq!(h.prover.calls(), 2);
    assert_eq!(h.settler.submitted().len(), 1);
    assert!(batch.chains_onto(&h.chain.spent_root()));
    assert!(h
        .record("a")
        .events
        .iter()
        .any(|e| e.message.contains("root moved")));
}

#[tokio::test(start_paused = true)]
async fn history_the_chain_no_longer_serves_comes_from_the_journal() {
    let chain = FakeChainLog::new();
    let h = Harness::start(
        ReturnStore::default(),
        &config(0, RetryPolicy::default()),
        ScriptedProver::instant(),
        ScriptedSettler::settling("0xfeed"),
        chain.clone(),
    );
    let first = h.accept("a", 11, 0x61);
    h.reach("a", ReturnStatus::Settled).await;
    chain.settle_externally(&[first]);
    let root_after_first = chain.spent_root();
    chain.forget_history();

    h.accept("b", 12, 0x62);
    h.reach("b", ReturnStatus::Settled).await;
    assert!(h.batch_of("b").chains_onto(&root_after_first));
    assert_eq!(h.settler.submitted().len(), 2);
}

#[tokio::test(start_paused = true)]
async fn history_nobody_holds_cannot_be_rebuilt() {
    let chain = FakeChainLog::new();
    chain.settle_externally(&[nullifier_of(&member(99, 0x70))]);
    chain.forget_history();
    let h = Harness::start(
        ReturnStore::memory(retry(1, 0)),
        &config(0, retry(1, 0)),
        ScriptedProver::instant(),
        ScriptedSettler::settling("0xfeed"),
        chain,
    );
    h.accept("a", 11, 0x61);
    h.reach("a", ReturnStatus::Failed).await;
    let failure = h.record("a").failure.unwrap();
    assert_eq!(failure.kind, ErrorKind::ChainRejected);
    assert!(
        failure.message.contains("diverged from chain"),
        "{}",
        failure.message
    );
    assert_eq!(h.prover.calls(), 0);
}

#[tokio::test(start_paused = true)]
async fn stale_root_from_settler_rebases() {
    let chain = FakeChainLog::new();
    let moved = chain.clone();
    let settler = ScriptedSettler::settling("0xfeed").then_with(move |_| {
        moved.settle_externally(&[nullifier_of(&member(99, 0x70))]);
        SubmitOutcome::StaleRoot
    });
    let h = Harness::start(
        ReturnStore::default(),
        &config(0, RetryPolicy::default()),
        ScriptedProver::instant(),
        settler,
        chain,
    );
    h.accept("a", 11, 0x61);
    h.reach("a", ReturnStatus::Settled).await;
    assert_eq!(h.batch_of("a").rebases, 1);
    assert_eq!(h.prover.calls(), 2);
    assert_eq!(h.settler.submitted().len(), 2);
}

#[tokio::test(start_paused = true)]
async fn rebase_is_bounded() {
    let chain = FakeChainLog::new();
    let first = chain.clone();
    let second = chain.clone();
    let settler = ScriptedSettler::settling("0xfeed")
        .then_with(move |_| {
            first.settle_externally(&[nullifier_of(&member(98, 0x71))]);
            SubmitOutcome::StaleRoot
        })
        .then_with(move |_| {
            second.settle_externally(&[nullifier_of(&member(99, 0x70))]);
            SubmitOutcome::StaleRoot
        });
    let h = Harness::start(
        ReturnStore::memory(retry(5, 1)),
        &config(0, retry(5, 1)),
        ScriptedProver::instant(),
        settler,
        chain,
    );
    h.accept("a", 11, 0x61);
    h.reach("a", ReturnStatus::Failed).await;
    let failure = h.record("a").failure.unwrap();
    assert!(failure.recoverable);
    assert_eq!(failure.kind, ErrorKind::ChainRejected);
    assert_eq!(h.batch_of("a").rebases, 1);
    assert_eq!(h.batch_of("a").status, BatchStatus::Failed);
}

#[tokio::test(start_paused = true)]
async fn settler_failure_leaves_bundle_and_members_recoverable_and_retries_reuse_the_proof() {
    let settler = ScriptedSettler::settling("0xfeed").then(SubmitOutcome::Failed {
        message: "rpc down".to_string(),
    });
    let h = Harness::simple(ScriptedProver::instant(), settler);
    h.accept("a", 11, 0x61);
    h.reach("a", ReturnStatus::Failed).await;
    let record = h.record("a");
    let failure = record.failure.unwrap();
    assert!(failure.recoverable);
    assert_eq!(failure.kind, ErrorKind::SubmissionFailed);
    assert!(failure.message.contains("rpc down"));
    let bundle = h
        .store
        .get_batch(record.batch_id.as_deref().unwrap())
        .unwrap();
    assert_eq!(bundle.proof_bytes, "0x01");

    h.reach_in("a", ReturnStatus::Settled, Duration::from_secs(120))
        .await;
    assert_eq!(h.prover.calls(), 1);
    assert_eq!(h.settler.submitted().len(), 2);
    assert_ne!(h.record("a").batch_id, record.batch_id);
}

#[tokio::test(start_paused = true)]
async fn a_chain_log_read_that_fails_before_settling_is_read_again_and_the_proof_settles() {
    let h = Harness::simple(
        ScriptedProver::instant(),
        ScriptedSettler::settling("0xfeed"),
    );
    h.chain.answer(&[true, false]);
    h.accept("a", 11, 0x61);
    h.reach_in("a", ReturnStatus::Settled, Duration::from_secs(60))
        .await;
    let record = h.record("a");
    assert_eq!(record.attempts, 0);
    assert!(record
        .events
        .iter()
        .all(|e| e.status != ReturnStatus::Failed));
    assert_eq!(h.batch_of("a").rebases, 0);
    assert_eq!(h.prover.calls(), 1);
    assert_eq!(h.settler.submitted().len(), 1);
    assert_eq!(h.chain.calls(), 3);
}

#[tokio::test(start_paused = true)]
async fn a_chain_log_read_that_fails_before_proving_is_read_again_without_an_attempt() {
    let h = Harness::simple(
        ScriptedProver::instant(),
        ScriptedSettler::settling("0xfeed"),
    );
    h.chain.answer(&[false]);
    h.accept("a", 11, 0x61);
    h.reach_in("a", ReturnStatus::Settled, Duration::from_secs(60))
        .await;
    assert_eq!(h.record("a").attempts, 0);
    assert_eq!(h.prover.calls(), 1);
    assert_eq!(h.chain.calls(), 3);
}

#[tokio::test(start_paused = true)]
async fn a_chain_log_that_stays_unreadable_fails_the_proven_batch_as_unavailable_and_keeps_the_proof(
) {
    let h = Harness::simple(
        ScriptedProver::instant(),
        ScriptedSettler::settling("0xfeed"),
    );
    h.chain.answer(&[true, false, false, false, false]);
    h.accept("a", 11, 0x61);
    h.reach_in("a", ReturnStatus::Failed, Duration::from_secs(150))
        .await;
    let record = h.record("a");
    assert_eq!(record.attempts, 1);
    let failure = record.failure.unwrap();
    assert!(failure.recoverable);
    assert_eq!(failure.kind, ErrorKind::ServiceUnavailable);
    assert!(
        failure.message.contains("accumulator not synced to chain")
            && failure.message.contains("request timeout"),
        "{}",
        failure.message
    );
    assert_eq!(h.chain.calls(), 5);
    assert!(h.settler.submitted().is_empty());

    h.reach_in("a", ReturnStatus::Settled, Duration::from_secs(120))
        .await;
    assert_eq!(h.prover.calls(), 1);
    assert_eq!(h.settler.submitted().len(), 1);
}

#[tokio::test(start_paused = true)]
async fn a_chain_log_that_no_longer_reproduces_the_vault_fails_the_proven_batch_at_once() {
    let prover = ScriptedProver::new(vec![ProveStep::Succeed { delay: HOUR }]);
    let h = Harness::simple(prover, ScriptedSettler::settling("0xfeed"));
    h.accept("a", 11, 0x61);
    h.reach("a", ReturnStatus::Proving).await;
    h.chain
        .settle_externally(&[nullifier_of(&member(99, 0x70))]);
    h.chain.forget_history();
    h.reach_in("a", ReturnStatus::Failed, 2 * HOUR).await;
    let failure = h.record("a").failure.unwrap();
    assert_eq!(failure.kind, ErrorKind::ChainRejected);
    assert!(
        failure.message.contains("diverged from chain"),
        "{}",
        failure.message
    );
    assert_eq!(h.chain.calls(), 2);
    assert!(h.settler.submitted().is_empty());
}

#[tokio::test(start_paused = true)]
async fn already_settled_member_is_excluded_and_rest_proceeds() {
    let h = Harness::simple(
        ScriptedProver::instant(),
        ScriptedSettler::settling("0xfeed"),
    );
    let txid = h
        .chain
        .settle_externally(&[nullifier_of(&member(11, 0x61))]);
    h.accept("a", 11, 0x61);
    h.accept("b", 12, 0x62);
    h.reach("b", ReturnStatus::Settled).await;
    let a = h.record("a");
    assert_eq!(a.status, ReturnStatus::Settled);
    assert_eq!(a.settle_txid.as_deref(), Some(txid.as_str()));
    assert!(a.message.contains("already released"));
    assert_eq!(
        h.chain.asked(),
        vec![Release {
            nullifier: hex(&nullifier_of(&member(11, 0x61))),
            txid
        }]
    );
    assert_eq!(h.record("b").settle_txid.as_deref(), Some("0xfeed"));
    assert_eq!(h.settler.submitted()[0].leaves.len(), 1);
}

fn nullifiers_of(bundle: &bridge_return_service::store::BatchBundle) -> Vec<[u8; 32]> {
    bundle
        .leaves
        .iter()
        .map(|l| {
            hex::decode(l.nullifier.trim_start_matches("0x"))
                .unwrap()
                .try_into()
                .unwrap()
        })
        .collect()
}

#[tokio::test(start_paused = true)]
async fn a_release_no_second_source_confirms_waits_and_settles_once_one_does() {
    let h = Harness::simple(
        ScriptedProver::instant(),
        ScriptedSettler::settling("0xfeed"),
    );
    h.chain
        .second_source(SecondSource::Disagrees("no such transaction".to_string()));
    let txid = h
        .chain
        .settle_externally(&[nullifier_of(&member(11, 0x61))]);
    h.accept("a", 11, 0x61);
    h.reach("a", ReturnStatus::Submitted).await;
    let a = h.record("a");
    assert!(!a.terminal);
    assert_eq!(a.success, None);
    assert_eq!(a.settle_txid, None);
    assert_eq!(a.attempts, 0);
    assert!(a.message.contains(&txid), "{}", a.message);
    assert!(a.message.contains("no such transaction"), "{}", a.message);
    assert_eq!(h.store.health().unconfirmed_releases, 1);

    h.chain.second_source(SecondSource::Down);
    let chain = h.chain.clone();
    advance_until(
        move || chain.asked().len() == 2,
        Duration::from_secs(10 * 60),
    )
    .await;
    let a = h.record("a");
    assert_eq!(a.status, ReturnStatus::Submitted);
    assert!(a.message.contains("unreachable"), "{}", a.message);

    h.chain.second_source(SecondSource::Agrees);
    h.reach_in("a", ReturnStatus::Settled, Duration::from_secs(10 * 60))
        .await;
    let a = h.record("a");
    assert_eq!(a.settle_txid.as_deref(), Some(txid.as_str()));
    assert_eq!(a.attempts, 0);
    assert_eq!(h.store.health().unconfirmed_releases, 0);
    assert_eq!(h.prover.calls(), 0);
    assert!(h.settler.submitted().is_empty());
}

#[tokio::test(start_paused = true)]
async fn a_drain_begun_while_a_release_is_rechecked_starts_no_batch() {
    let h = Harness::simple(
        ScriptedProver::instant(),
        ScriptedSettler::settling("0xfeed"),
    );
    h.chain
        .second_source(SecondSource::Disagrees("no such transaction".to_string()));
    h.chain
        .settle_externally(&[nullifier_of(&member(11, 0x61))]);
    h.accept("a", 11, 0x61);
    h.reach("a", ReturnStatus::Submitted).await;

    h.chain.slow(Duration::from_secs(120));
    let reads = h.chain.calls();
    let chain = h.chain.clone();
    advance_until(move || chain.calls() > reads, Duration::from_secs(10 * 60)).await;
    h.accept("b", 12, 0x62);
    h.drain.start();
    let task = &h.task;
    advance_until(|| task.is_finished(), Duration::from_secs(10 * 60)).await;
    assert_eq!(h.status("b"), ReturnStatus::Queued);
    assert_eq!(h.prover.calls(), 0);
}

#[tokio::test(start_paused = true)]
async fn a_release_the_log_retracts_before_it_is_confirmed_is_proven_again() {
    let h = Harness::simple(
        ScriptedProver::instant(),
        ScriptedSettler::settling("0xfeed"),
    );
    h.chain
        .second_source(SecondSource::Disagrees("no such transaction".to_string()));
    h.chain
        .settle_externally(&[nullifier_of(&member(11, 0x61))]);
    h.accept("a", 11, 0x61);
    h.reach("a", ReturnStatus::Submitted).await;
    assert!(h.settler.submitted().is_empty());

    h.chain.retract_everything();
    h.reach_in("a", ReturnStatus::Settled, Duration::from_secs(10 * 60))
        .await;
    let a = h.record("a");
    assert_eq!(a.settle_txid.as_deref(), Some("0xfeed"));
    assert_eq!(a.attempts, 0);
    assert_eq!(h.settler.submitted().len(), 1);
}

#[tokio::test(start_paused = true)]
async fn a_waiting_release_is_rechecked_through_the_second_source_and_the_log_only_when_it_disagrees(
) {
    let h = Harness::simple(
        ScriptedProver::instant(),
        ScriptedSettler::settling("0xfeed"),
    );
    h.chain.second_source(SecondSource::Down);
    let txid = h
        .chain
        .settle_externally(&[nullifier_of(&member(11, 0x61))]);
    h.accept("a", 11, 0x61);
    h.reach("a", ReturnStatus::Submitted).await;

    let reads = h.chain.calls();
    let chain = h.chain.clone();
    advance_until(
        move || chain.asked().len() == 3,
        Duration::from_secs(15 * 60),
    )
    .await;
    assert_eq!(h.chain.calls(), reads);

    h.chain
        .second_source(SecondSource::Disagrees("no such transaction".to_string()));
    let chain = h.chain.clone();
    advance_until(move || chain.calls() > reads, Duration::from_secs(10 * 60)).await;
    assert_eq!(h.status("a"), ReturnStatus::Submitted);

    h.chain.second_source(SecondSource::Agrees);
    let reads = h.chain.calls();
    h.reach_in("a", ReturnStatus::Settled, Duration::from_secs(10 * 60))
        .await;
    assert_eq!(h.chain.calls(), reads);
    assert_eq!(h.record("a").settle_txid.as_deref(), Some(txid.as_str()));
}

#[tokio::test(start_paused = true)]
async fn without_a_second_source_a_waiting_release_is_not_rechecked() {
    let h = Harness::simple(
        ScriptedProver::instant(),
        ScriptedSettler::settling("0xfeed"),
    );
    h.chain.no_second_source();
    h.chain
        .settle_externally(&[nullifier_of(&member(11, 0x61))]);
    h.accept("a", 11, 0x61);
    h.reach("a", ReturnStatus::Submitted).await;
    assert!(h.record("a").message.contains("BRIDGE_RETURN_CONFIRM_CMD"));
    let (reads, asked) = (h.chain.calls(), h.chain.asked().len());
    tokio::time::sleep(HOUR).await;
    assert_eq!(h.chain.calls(), reads);
    assert_eq!(h.status("a"), ReturnStatus::Submitted);

    h.accept("b", 12, 0x62);
    h.reach_in("b", ReturnStatus::Settled, Duration::from_secs(60))
        .await;
    tokio::time::sleep(HOUR).await;
    assert_eq!(h.chain.asked().len(), asked);
    assert_eq!(h.status("a"), ReturnStatus::Submitted);
}

#[tokio::test(start_paused = true)]
async fn a_recheck_that_finds_a_release_waiting_as_before_adds_nothing_to_the_journal() {
    let dir = tempfile::tempdir().unwrap();
    let retry = RetryPolicy::default();
    let chain = FakeChainLog::new();
    chain.second_source(SecondSource::Disagrees("no such transaction".to_string()));
    chain.settle_externally(&[nullifier_of(&member(11, 0x61))]);
    let h = Harness::start(
        reopen(dir.path(), retry),
        &config(0, retry),
        ScriptedProver::instant(),
        ScriptedSettler::settling("0xfeed"),
        chain.clone(),
    );
    h.accept("a", 11, 0x61);
    h.reach("a", ReturnStatus::Submitted).await;
    let lines = journal_lines(dir.path());
    let probe = h.chain.clone();
    advance_until(
        move || probe.asked().len() >= 6,
        Duration::from_secs(40 * 60),
    )
    .await;
    assert_eq!(journal_lines(dir.path()), lines);
    assert_eq!(h.status("a"), ReturnStatus::Submitted);
    h.stop().await;

    let asked = chain.asked().len();
    let h = Harness::start(
        reopen(dir.path(), retry),
        &config(0, retry),
        ScriptedProver::instant(),
        ScriptedSettler::settling("0xfeed"),
        chain.clone(),
    );
    let probe = chain.clone();
    advance_until(
        move || probe.asked().len() > asked,
        Duration::from_secs(10 * 60),
    )
    .await;
    assert_eq!(h.status("a"), ReturnStatus::Submitted);
}

fn broadcasting(txid: &str) -> ScriptedSettler {
    ScriptedSettler::settling("0xfeed").then(SubmitOutcome::Unconfirmed {
        txid: txid.to_string(),
    })
}

fn journal_lines(dir: &std::path::Path) -> usize {
    std::fs::read_to_string(dir.join("journal.jsonl"))
        .unwrap()
        .lines()
        .count()
}

impl Harness {
    async fn submitted(&self, id: &str) {
        let store = self.store.clone();
        let key = id.to_string();
        yield_until(move || {
            store.read(|ledger| {
                let batch_id = ledger.get(&key).and_then(|r| r.record.batch_id.clone());
                batch_id
                    .and_then(|b| ledger.batch(&b).map(|b| b.status == BatchStatus::Submitted))
                    .unwrap_or(false)
            })
        })
        .await;
    }
}

#[tokio::test(start_paused = true)]
async fn a_broadcast_settlement_settles_once_its_receipt_shows_it_settled() {
    let h = Harness::simple(ScriptedProver::instant(), broadcasting("0xslow"));
    h.chain.transaction("0xslow", SettlementState::Pending);
    h.accept("a", 11, 0x61);
    h.submitted("a").await;
    assert_eq!(h.store.health().unconfirmed_settlements, 1);
    let a = h.record("a");
    assert_eq!(a.status, ReturnStatus::Proven);
    assert!(a.message.contains("0xslow"), "{}", a.message);

    tokio::time::sleep(Duration::from_secs(5 * 60)).await;
    assert_eq!(h.status("a"), ReturnStatus::Proven);
    h.chain.transaction("0xslow", SettlementState::Confirmed);
    h.reach_in("a", ReturnStatus::Settled, Duration::from_secs(60))
        .await;
    let a = h.record("a");
    assert_eq!(a.settle_txid.as_deref(), Some("0xslow"));
    assert_eq!(a.attempts, 0);
    assert_eq!(h.batch_of("a").status, BatchStatus::Settled);
    assert_eq!(h.store.health().unconfirmed_settlements, 0);
    assert!(h.chain.asked().is_empty());
    assert!(h.chain.looked_up().iter().all(|t| t == "0xslow"));
    assert_eq!(h.settler.submitted().len(), 1);
}

#[tokio::test(start_paused = true)]
async fn a_broadcast_settlement_the_chain_does_not_know_fails_and_the_retry_settles() {
    let h = Harness::with_retry(
        ScriptedProver::instant(),
        broadcasting("0xlost"),
        retry(5, 3),
    );
    h.accept("a", 11, 0x61);
    h.submitted("a").await;
    tokio::time::sleep(Duration::from_secs(9 * 60)).await;
    assert_eq!(h.status("a"), ReturnStatus::Proven);
    assert_eq!(h.record("a").attempts, 0);
    h.reach_in("a", ReturnStatus::Failed, Duration::from_secs(5 * 60))
        .await;
    let a = h.record("a");
    assert_eq!(a.attempts, 1);
    let failure = a.failure.unwrap();
    assert_eq!(failure.kind, ErrorKind::SubmissionFailed);
    assert!(failure.recoverable);
    assert!(failure.message.contains("0xlost"), "{}", failure.message);
    assert!(failure.message.contains("dropped"), "{}", failure.message);

    h.reach_in("a", ReturnStatus::Settled, Duration::from_secs(10 * 60))
        .await;
    assert_eq!(h.record("a").settle_txid.as_deref(), Some("0xfeed"));
    assert_eq!(h.prover.calls(), 1);
    assert_eq!(h.settler.submitted().len(), 2);
}

#[tokio::test(start_paused = true)]
async fn a_broadcast_settlement_nothing_can_read_waits_uncharged_while_other_batches_settle() {
    let h = Harness::simple(ScriptedProver::instant(), broadcasting("0xslow"));
    h.chain.receipts(Receipts::Down);
    h.accept("a", 11, 0x61);
    h.submitted("a").await;
    h.chain.unreadable(true);

    tokio::time::sleep(Duration::from_secs(20 * 60)).await;
    let a = h.record("a");
    assert_eq!(a.status, ReturnStatus::Proven);
    assert_eq!(a.attempts, 0);
    assert_eq!(a.failure, None);
    assert_eq!(h.batch_of("a").status, BatchStatus::Submitted);
    assert_eq!(h.store.health().unconfirmed_settlements, 1);

    h.chain.unreadable(false);
    h.accept("b", 12, 0x62);
    h.reach_in("b", ReturnStatus::Settled, Duration::from_secs(60))
        .await;
    assert_eq!(h.record("b").settle_txid.as_deref(), Some("0xfeed"));
    assert_eq!(h.status("a"), ReturnStatus::Proven);

    h.chain.receipts(Receipts::Read);
    h.chain.transaction("0xslow", SettlementState::Confirmed);
    h.reach_in("a", ReturnStatus::Settled, Duration::from_secs(10 * 60))
        .await;
    let a = h.record("a");
    assert_eq!(a.settle_txid.as_deref(), Some("0xslow"));
    assert_eq!(a.attempts, 0);
    assert_eq!(h.settler.submitted().len(), 2);
}

#[tokio::test(start_paused = true)]
async fn a_reverted_settlement_fails_its_batch_only_once_the_log_shows_nothing_else_settled_it() {
    let h = Harness::with_retry(
        ScriptedProver::instant(),
        broadcasting("0xbad"),
        retry(5, 3),
    );
    h.chain.transaction("0xbad", SettlementState::Reverted);
    h.accept("a", 11, 0x61);
    h.submitted("a").await;
    h.chain.unreadable(true);
    tokio::time::sleep(Duration::from_secs(20 * 60)).await;
    assert_eq!(h.status("a"), ReturnStatus::Proven);
    assert_eq!(h.record("a").attempts, 0);

    h.chain.unreadable(false);
    h.reach_in("a", ReturnStatus::Failed, Duration::from_secs(10 * 60))
        .await;
    let a = h.record("a");
    assert_eq!(a.attempts, 1);
    let failure = a.failure.unwrap();
    assert_eq!(failure.kind, ErrorKind::SubmissionFailed);
    assert!(
        failure.message.contains("0xbad reverted"),
        "{}",
        failure.message
    );
    h.reach_in("a", ReturnStatus::Settled, Duration::from_secs(10 * 60))
        .await;
    assert_eq!(h.record("a").settle_txid.as_deref(), Some("0xfeed"));
}

#[tokio::test(start_paused = true)]
async fn a_batch_another_proof_released_settles_its_burns_there_without_taking_its_transition() {
    let retry = RetryPolicy::default();
    let chain = FakeChainLog::new();
    let rival = chain.clone();
    let settler = ScriptedSettler::settling("0xfeed").then_with(move |bundle| {
        let mut released = nullifiers_of(bundle);
        released.push(nullifier_of(&member(99, 0x70)));
        rival.settle_as(&released, "0xrival");
        SubmitOutcome::Unconfirmed {
            txid: "0xours".to_string(),
        }
    });
    let h = Harness::start(
        ReturnStore::memory(retry),
        &config(0, retry),
        ScriptedProver::instant(),
        settler,
        chain,
    );
    h.chain.transaction("0xours", SettlementState::Reverted);
    h.accept("a", 11, 0x61);
    h.reach_in("a", ReturnStatus::Settled, Duration::from_secs(5 * 60))
        .await;
    let a = h.record("a");
    assert_eq!(a.settle_txid.as_deref(), Some("0xrival"));
    assert_eq!(a.attempts, 0);
    assert_eq!(h.batch_of("a").status, BatchStatus::Failed);
    assert!(h.store.read(|ledger| ledger.settled_batches()).is_empty());

    h.accept("b", 12, 0x62);
    h.reach_in("b", ReturnStatus::Settled, Duration::from_secs(60))
        .await;
    assert_eq!(h.record("b").settle_txid.as_deref(), Some("0xfeed"));
}

#[tokio::test(start_paused = true)]
async fn a_restart_during_the_wait_looks_the_settlement_up_instead_of_sending_another() {
    let dir = tempfile::tempdir().unwrap();
    let retry = RetryPolicy::default();
    let chain = FakeChainLog::new();
    chain.transaction("0xslow", SettlementState::Pending);
    let h = Harness::start(
        reopen(dir.path(), retry),
        &config(0, retry),
        ScriptedProver::instant(),
        broadcasting("0xslow"),
        chain.clone(),
    );
    h.accept("a", 11, 0x61);
    h.submitted("a").await;
    tokio::time::sleep(Duration::from_secs(60)).await;
    h.stop().await;

    let h = Harness::start(
        reopen(dir.path(), retry),
        &config(0, retry),
        ScriptedProver::instant(),
        ScriptedSettler::settling("0xsecond"),
        chain.clone(),
    );
    let probe = chain.clone();
    let looked = chain.looked_up().len();
    yield_until(move || probe.looked_up().len() > looked).await;
    tokio::time::sleep(Duration::from_secs(60)).await;
    assert_eq!(h.status("a"), ReturnStatus::Proven);
    assert!(h.settler.submitted().is_empty());
    assert_eq!(h.batch_of("a").status, BatchStatus::Submitted);

    chain.transaction("0xslow", SettlementState::Confirmed);
    h.reach_in("a", ReturnStatus::Settled, Duration::from_secs(10 * 60))
        .await;
    let a = h.record("a");
    assert_eq!(a.settle_txid.as_deref(), Some("0xslow"));
    assert_eq!(a.attempts, 0);
    assert!(h.settler.submitted().is_empty());
    assert_eq!(h.prover.calls(), 0);
}

#[tokio::test(start_paused = true)]
async fn a_drain_does_not_wait_on_a_broadcast_settlement_and_the_next_start_looks_it_up() {
    let dir = tempfile::tempdir().unwrap();
    let retry = RetryPolicy::default();
    let chain = FakeChainLog::new();
    chain.transaction("0xslow", SettlementState::Pending);
    let mut h = Harness::start(
        reopen(dir.path(), retry),
        &config(0, retry),
        ScriptedProver::instant(),
        broadcasting("0xslow"),
        chain.clone(),
    );
    h.accept("a", 11, 0x61);
    h.submitted("a").await;
    let started = h.clock.now_ms();
    assert_eq!(
        h.stop_with(&[StopSignal::Terminate], HOUR).await,
        Stopped::Drained
    );
    assert!(h.clock.now_ms() - started < MINUTE);
    assert_eq!(h.batch_of("a").status, BatchStatus::Submitted);
    assert_eq!(h.record("a").attempts, 0);
    drop(h);

    chain.transaction("0xslow", SettlementState::Confirmed);
    let h = Harness::start(
        reopen(dir.path(), retry),
        &config(0, retry),
        ScriptedProver::instant(),
        ScriptedSettler::settling("0xsecond"),
        chain,
    );
    h.reach("a", ReturnStatus::Settled).await;
    assert_eq!(h.record("a").settle_txid.as_deref(), Some("0xslow"));
    assert!(h.settler.submitted().is_empty());
}

#[tokio::test(start_paused = true)]
async fn without_a_receipt_command_a_broadcast_settlement_is_decided_by_the_log_alone() {
    let h = Harness::simple(ScriptedProver::instant(), broadcasting("0xlost"));
    h.chain.receipts(Receipts::Absent);
    h.accept("a", 11, 0x61);
    h.submitted("a").await;
    tokio::time::sleep(Duration::from_secs(30 * 60)).await;
    assert_eq!(h.status("a"), ReturnStatus::Proven);
    assert_eq!(h.record("a").attempts, 0);

    let bundle = h.batch_of("a").bundle.unwrap();
    h.chain.settle_as(&nullifiers_of(&bundle), "0xlost");
    h.reach_in("a", ReturnStatus::Settled, Duration::from_secs(10 * 60))
        .await;
    assert_eq!(h.record("a").settle_txid.as_deref(), Some("0xlost"));
    assert_eq!(h.batch_of("a").status, BatchStatus::Settled);
    assert_eq!(h.settler.submitted().len(), 1);
}

#[tokio::test(start_paused = true)]
async fn an_unconfirmed_settlement_overtaken_by_another_batch_is_proven_again() {
    let retry = RetryPolicy::default();
    let chain = FakeChainLog::new();
    let rival = chain.clone();
    let settler = ScriptedSettler::settling("0xfeed").then_with(move |_| {
        rival.settle_externally(&[nullifier_of(&member(99, 0x70))]);
        SubmitOutcome::Unconfirmed {
            txid: "0xlost".to_string(),
        }
    });
    let h = Harness::start(
        ReturnStore::memory(retry),
        &config(0, retry),
        ScriptedProver::instant(),
        settler,
        chain,
    );
    h.accept("a", 11, 0x61);
    h.reach_in("a", ReturnStatus::Settled, Duration::from_secs(20 * 60))
        .await;
    let a = h.record("a");
    assert_eq!(a.settle_txid.as_deref(), Some("0xfeed"));
    assert_eq!(a.attempts, 0);
    assert_eq!(h.batch_of("a").rebases, 1);
    assert_eq!(h.prover.calls(), 2);
}

#[tokio::test(start_paused = true)]
async fn a_settlement_that_reverts_behind_another_batch_is_proven_again_without_an_attempt() {
    let retry = RetryPolicy::default();
    let chain = FakeChainLog::new();
    let rival = chain.clone();
    let settler = ScriptedSettler::settling("0xfeed").then_with(move |_| {
        rival.settle_externally(&[nullifier_of(&member(99, 0x70))]);
        SubmitOutcome::Unconfirmed {
            txid: "0xbehind".to_string(),
        }
    });
    let h = Harness::start(
        ReturnStore::memory(retry),
        &config(0, retry),
        ScriptedProver::instant(),
        settler,
        chain,
    );
    h.chain.transaction("0xbehind", SettlementState::Reverted);
    h.accept("a", 11, 0x61);
    h.reach_in("a", ReturnStatus::Settled, Duration::from_secs(2 * 60))
        .await;
    let a = h.record("a");
    assert_eq!(a.settle_txid.as_deref(), Some("0xfeed"));
    assert_eq!(a.attempts, 0);
    assert_eq!(h.batch_of("a").rebases, 1);
}

#[tokio::test(start_paused = true)]
async fn a_restart_while_the_relayer_still_waits_on_its_receipt_sends_nothing_again() {
    let dir = tempfile::tempdir().unwrap();
    let retry = RetryPolicy::default();
    let chain = FakeChainLog::new();
    chain.transaction("0xslow", SettlementState::Pending);
    let h = Harness::start(
        reopen(dir.path(), retry),
        &config(0, retry),
        ScriptedProver::instant(),
        ScriptedSettler::settling("0xfeed").then_sign_and_hang("0xslow"),
        chain.clone(),
    );
    h.accept("a", 11, 0x61);
    h.submitted("a").await;
    assert_eq!(h.batch_of("a").broadcast_txid.as_deref(), Some("0xslow"));
    h.stop().await;

    let h = Harness::start(
        reopen(dir.path(), retry),
        &config(0, retry),
        ScriptedProver::instant(),
        ScriptedSettler::settling("0xsecond"),
        chain.clone(),
    );
    tokio::time::sleep(Duration::from_secs(10 * 60)).await;
    assert!(h.settler.submitted().is_empty());
    assert_eq!(h.status("a"), ReturnStatus::Proven);
    assert_eq!(h.record("a").attempts, 0);

    chain.transaction("0xslow", SettlementState::Confirmed);
    h.reach_in("a", ReturnStatus::Settled, Duration::from_secs(10 * 60))
        .await;
    assert_eq!(h.record("a").settle_txid.as_deref(), Some("0xslow"));
    assert!(h.settler.submitted().is_empty());
}

#[tokio::test(start_paused = true)]
async fn rejected_recipient_is_excluded_before_proving() {
    let h = Harness::simple(
        ScriptedProver::instant(),
        ScriptedSettler::settling("0xfeed"),
    );
    let a_nullifier = nullifier_of(&member(11, 0x61));
    h.settler
        .reject(&format!("0x{}", hex::encode(a_nullifier)), "blocked");
    h.accept("a", 11, 0x61);
    h.accept("b", 12, 0x62);
    h.reach("b", ReturnStatus::Settled).await;
    let a = h.record("a");
    assert_eq!(a.status, ReturnStatus::Failed);
    assert_eq!(a.attempts, 1);
    let failure = a.failure.unwrap();
    assert!(failure.recoverable);
    assert_eq!(failure.kind, ErrorKind::SubmissionFailed);
    assert!(failure.message.contains("blocked"));
    assert_eq!(h.prover.calls(), 1);
    assert_eq!(h.settler.submitted()[0].leaves.len(), 1);
}

#[tokio::test(start_paused = true)]
async fn a_lock_mismatch_fails_its_burn_for_good_and_the_rest_settles() {
    let h = Harness::simple(
        ScriptedProver::instant(),
        ScriptedSettler::settling("0xfeed"),
    );
    let a_nullifier = nullifier_of(&member(11, 0x61));
    h.settler.reject_as(
        &format!("0x{}", hex::encode(a_nullifier)),
        "lock 11 digest 0xab is not the vault's 0x00",
        RejectionKind::Lock,
    );
    h.accept("a", 11, 0x61);
    h.accept("b", 12, 0x62);
    h.reach("b", ReturnStatus::Settled).await;
    let a = h.record("a");
    assert_eq!(a.status, ReturnStatus::Failed);
    let failure = a.failure.unwrap();
    assert!(!failure.recoverable);
    assert_eq!(failure.kind, ErrorKind::ChainRejected);
    assert!(failure.message.contains("lock 11"));
    assert_eq!(h.settler.submitted()[0].leaves.len(), 1);
    assert_eq!(h.store.health().queue_depth, 0);

    tokio::time::sleep(HOUR).await;
    h.accept("c", 13, 0x63);
    h.reach("c", ReturnStatus::Settled).await;
    assert_eq!(h.status("a"), ReturnStatus::Failed);
    assert_eq!(h.prover.calls(), 2);
    assert!(h
        .settler
        .submitted()
        .iter()
        .all(|bundle| bundle.leaves.len() == 1));
}

#[tokio::test(start_paused = true)]
async fn hanging_prover_keeps_new_burns_queued() {
    let prover = ScriptedProver::new(vec![ProveStep::Hang]);
    let h = Harness::simple(prover, ScriptedSettler::settling("0xfeed"));
    h.accept("a", 11, 0x61);
    h.reach("a", ReturnStatus::Proving).await;
    h.accept("b", 12, 0x62);
    tokio::time::sleep(HOUR).await;
    assert_eq!(h.status("b"), ReturnStatus::Queued);
    assert_eq!(h.record("b").queue_position, Some(1));
    let health = h.store.health();
    assert_eq!(health.active_batch, h.record("a").batch_id);
    assert!(health.proving_since_ms.is_some());
    assert_eq!(health.last_proof_ms, None);
}

#[tokio::test(start_paused = true)]
async fn settle_without_submitter_stays_proven() {
    let h = Harness::simple(ScriptedProver::instant(), ScriptedSettler::skipping());
    h.accept("a", 11, 0x61);
    h.reach("a", ReturnStatus::Proven).await;
    tokio::time::sleep(HOUR).await;
    assert_eq!(h.status("a"), ReturnStatus::Proven);
    assert!(h
        .store
        .get_batch(&h.record("a").batch_id.unwrap())
        .is_some());
    assert_eq!(h.store.health().queue_depth, 0);
    assert!(h.store.health().last_proof_ms.is_some());
}

#[tokio::test(start_paused = true)]
async fn a_drain_lets_the_batch_in_flight_settle_and_starts_no_other() {
    let prover = ScriptedProver::new(vec![ProveStep::Succeed {
        delay: Duration::from_secs(1_200),
    }]);
    let mut h = Harness::simple(prover, ScriptedSettler::settling("0xfeed"));
    h.accept("a", 11, 0x61);
    h.reach("a", ReturnStatus::Proving).await;
    h.accept("b", 12, 0x62);
    let stopped = h.stop_with(&[StopSignal::Terminate], Duration::ZERO).await;
    assert_eq!(stopped, Stopped::Drained);
    assert!(h.drain.is_draining());
    assert_eq!(h.status("a"), ReturnStatus::Settled);
    assert_eq!(h.status("b"), ReturnStatus::Queued);
    assert_eq!(h.record("b").attempts, 0);
    assert_eq!(h.prover.calls(), 1);
}

#[tokio::test(start_paused = true)]
async fn a_drain_with_nothing_in_flight_ends_at_once() {
    let mut h = Harness::simple(
        ScriptedProver::instant(),
        ScriptedSettler::settling("0xfeed"),
    );
    tokio::task::yield_now().await;
    let started = h.clock.now_ms();
    let stopped = h.stop_with(&[StopSignal::Terminate], Duration::ZERO).await;
    assert_eq!(stopped, Stopped::Drained);
    assert_eq!(h.clock.now_ms(), started);
}

#[tokio::test(start_paused = true)]
async fn a_drain_starts_no_batch_for_burns_waiting_out_the_idle_window() {
    let retry = RetryPolicy::default();
    let mut h = Harness::start(
        ReturnStore::memory(retry),
        &config(60, retry),
        ScriptedProver::instant(),
        ScriptedSettler::settling("0xfeed"),
        FakeChainLog::new(),
    );
    h.accept("a", 11, 0x61);
    tokio::task::yield_now().await;
    let started = h.clock.now_ms();
    let stopped = h.stop_with(&[StopSignal::Terminate], Duration::ZERO).await;
    assert_eq!(stopped, Stopped::Drained);
    assert_eq!(h.clock.now_ms(), started);
    assert_eq!(h.status("a"), ReturnStatus::Queued);
    assert_eq!(h.prover.calls(), 0);
}

#[tokio::test(start_paused = true)]
async fn a_second_sigterm_abandons_the_proof_and_the_next_start_proves_it_without_an_attempt() {
    let dir = tempfile::tempdir().unwrap();
    let retry = RetryPolicy::default();
    let mut h = Harness::start(
        reopen(dir.path(), retry),
        &config(0, retry),
        ScriptedProver::new(vec![ProveStep::Hang]),
        ScriptedSettler::settling("0xfeed"),
        FakeChainLog::new(),
    );
    h.accept("a", 11, 0x61);
    h.reach("a", ReturnStatus::Proving).await;
    let abandoned = h.record("a").batch_id.unwrap();
    let stopped = h
        .stop_with(
            &[StopSignal::Terminate, StopSignal::Terminate],
            Duration::ZERO,
        )
        .await;
    assert_eq!(stopped, Stopped::Abandoned);
    drop(h);

    let store = reopen(dir.path(), retry);
    let record = store.get("a").unwrap();
    assert_eq!(record.status, ReturnStatus::Queued);
    assert_eq!(record.attempts, 0);
    assert_eq!(record.not_before_ms, None);
    assert!(record.message.contains("stopped with the service"));
    assert_eq!(
        store.read(|l| l.batch(&abandoned).unwrap().status),
        BatchStatus::Interrupted
    );
    let h = Harness::start(
        store,
        &config(0, retry),
        ScriptedProver::instant(),
        ScriptedSettler::settling("0xfeed"),
        FakeChainLog::new(),
    );
    h.reach("a", ReturnStatus::Settled).await;
    assert_eq!(h.record("a").attempts, 0);
    assert_ne!(h.record("a").batch_id.unwrap(), abandoned);
}

#[tokio::test(start_paused = true)]
async fn sigint_abandons_the_proof_at_once_without_an_attempt() {
    let prover = ScriptedProver::new(vec![ProveStep::Hang]);
    let mut h = Harness::simple(prover, ScriptedSettler::settling("0xfeed"));
    h.accept("a", 11, 0x61);
    h.reach("a", ReturnStatus::Proving).await;
    let stopped = h.stop_with(&[StopSignal::Interrupt], Duration::ZERO).await;
    assert_eq!(stopped, Stopped::Abandoned);
    assert!(!h.drain.is_draining());
    assert_eq!(h.status("a"), ReturnStatus::Queued);
    assert_eq!(h.record("a").attempts, 0);
}

#[tokio::test(start_paused = true)]
async fn a_drain_that_outlasts_its_timeout_abandons_the_proof_without_an_attempt() {
    let prover = ScriptedProver::new(vec![ProveStep::Hang]);
    let mut h = Harness::simple(prover, ScriptedSettler::settling("0xfeed"));
    h.accept("a", 11, 0x61);
    h.reach("a", ReturnStatus::Proving).await;
    let started = h.clock.now_ms();
    let stopped = h
        .stop_with(&[StopSignal::Terminate], Duration::from_secs(600))
        .await;
    assert_eq!(stopped, Stopped::Abandoned);
    assert_eq!(h.clock.now_ms(), started + 600_000);
    assert_eq!(h.status("a"), ReturnStatus::Queued);
    assert_eq!(h.record("a").attempts, 0);
}

fn reopen(dir: &std::path::Path, retry: RetryPolicy) -> ReturnStore {
    ReturnStore::open(dir, retry, Clock::default().now_ms()).unwrap()
}

#[tokio::test(start_paused = true)]
async fn restart_requeues_queued_and_interrupted() {
    let dir = tempfile::tempdir().unwrap();
    let retry = RetryPolicy::default();
    let h = Harness::start(
        reopen(dir.path(), retry),
        &config(0, retry),
        ScriptedProver::new(vec![ProveStep::Hang]),
        ScriptedSettler::settling("0xfeed"),
        FakeChainLog::new(),
    );
    h.accept("a", 11, 0x61);
    h.reach("a", ReturnStatus::Proving).await;
    h.accept("b", 12, 0x62);
    let interrupted = h.record("a").batch_id.unwrap();
    h.stop().await;

    let store = reopen(dir.path(), retry);
    assert_eq!(store.get("a").unwrap().status, ReturnStatus::Queued);
    assert_eq!(store.get("a").unwrap().attempts, 1);
    assert!(store.get("a").unwrap().not_before_ms.is_some());
    assert_eq!(store.get("b").unwrap().status, ReturnStatus::Queued);
    assert_eq!(store.get("b").unwrap().attempts, 0);
    assert_eq!(
        store.read(|l| l.batch(&interrupted).unwrap().status),
        BatchStatus::Interrupted
    );
    assert!(store.get("a").unwrap().message.contains("interrupted"));

    let h = Harness::start(
        store,
        &config(0, retry),
        ScriptedProver::instant(),
        ScriptedSettler::settling("0xfeed"),
        FakeChainLog::new(),
    );
    h.reach("b", ReturnStatus::Settled).await;
    assert_eq!(h.status("a"), ReturnStatus::Queued);
    h.reach_in("a", ReturnStatus::Settled, Duration::from_secs(120))
        .await;
    assert_ne!(h.record("a").batch_id, h.record("b").batch_id);
    assert_eq!(h.prover.calls(), 2);
}

#[tokio::test(start_paused = true)]
async fn restart_settles_proven_batch_found_on_chain() {
    let dir = tempfile::tempdir().unwrap();
    let retry = RetryPolicy::default();
    let chain = FakeChainLog::new();
    let h = Harness::start(
        reopen(dir.path(), retry),
        &config(0, retry),
        ScriptedProver::instant(),
        ScriptedSettler::skipping(),
        chain.clone(),
    );
    let a_nullifier = h.accept("a", 11, 0x61);
    h.reach("a", ReturnStatus::Proven).await;
    h.stop().await;
    let txid = chain.settle_externally(&[a_nullifier]);

    let h = Harness::start(
        reopen(dir.path(), retry),
        &config(0, retry),
        ScriptedProver::instant(),
        ScriptedSettler::settling("0xfeed"),
        chain,
    );
    h.reach("a", ReturnStatus::Settled).await;
    assert_eq!(h.record("a").settle_txid.as_deref(), Some(txid.as_str()));
    assert_eq!(h.batch_of("a").status, BatchStatus::Settled);
    assert_eq!(h.prover.calls(), 0);
    assert!(h.settler.submitted().is_empty());
}

#[tokio::test(start_paused = true)]
async fn a_proven_batch_released_without_confirmation_is_not_taken_for_settled() {
    let dir = tempfile::tempdir().unwrap();
    let retry = RetryPolicy::default();
    let chain = FakeChainLog::new();
    let h = Harness::start(
        reopen(dir.path(), retry),
        &config(0, retry),
        ScriptedProver::instant(),
        ScriptedSettler::skipping(),
        chain.clone(),
    );
    let a_nullifier = h.accept("a", 11, 0x61);
    h.reach("a", ReturnStatus::Proven).await;
    h.stop().await;
    let txid = chain.settle_externally(&[a_nullifier]);
    chain.second_source(SecondSource::Disagrees("no such transaction".to_string()));

    let h = Harness::start(
        reopen(dir.path(), retry),
        &config(0, retry),
        ScriptedProver::instant(),
        ScriptedSettler::settling("0xfeed"),
        chain,
    );
    h.reach("a", ReturnStatus::Submitted).await;
    assert_eq!(h.record("a").attempts, 0);
    assert_eq!(h.batch_of("a").status, BatchStatus::Failed);
    assert!(h.store.read(|ledger| ledger.settled_batches()).is_empty());
    assert!(h.settler.submitted().is_empty());

    h.chain.second_source(SecondSource::Agrees);
    h.reach_in("a", ReturnStatus::Settled, Duration::from_secs(10 * 60))
        .await;
    assert_eq!(h.record("a").settle_txid.as_deref(), Some(txid.as_str()));
    assert_eq!(h.prover.calls(), 0);
}

#[tokio::test(start_paused = true)]
async fn restart_resubmits_proven_batch_when_root_matches() {
    let dir = tempfile::tempdir().unwrap();
    let retry = RetryPolicy::default();
    let h = Harness::start(
        reopen(dir.path(), retry),
        &config(0, retry),
        ScriptedProver::instant(),
        ScriptedSettler::skipping(),
        FakeChainLog::new(),
    );
    h.accept("a", 11, 0x61);
    h.reach("a", ReturnStatus::Proven).await;
    let batch_id = h.record("a").batch_id;
    h.stop().await;

    let h = Harness::start(
        reopen(dir.path(), retry),
        &config(0, retry),
        ScriptedProver::instant(),
        ScriptedSettler::settling("0xfeed"),
        FakeChainLog::new(),
    );
    h.reach("a", ReturnStatus::Settled).await;
    assert_eq!(h.record("a").settle_txid.as_deref(), Some("0xfeed"));
    assert_eq!(h.record("a").batch_id, batch_id);
    assert_eq!(h.prover.calls(), 0);
}

#[tokio::test(start_paused = true)]
async fn restart_rebases_proven_batch_when_root_moved() {
    let dir = tempfile::tempdir().unwrap();
    let retry = RetryPolicy::default();
    let chain = FakeChainLog::new();
    let h = Harness::start(
        reopen(dir.path(), retry),
        &config(0, retry),
        ScriptedProver::instant(),
        ScriptedSettler::skipping(),
        chain.clone(),
    );
    h.accept("a", 11, 0x61);
    h.reach("a", ReturnStatus::Proven).await;
    h.stop().await;
    chain.settle_externally(&[nullifier_of(&member(99, 0x70))]);

    let h = Harness::start(
        reopen(dir.path(), retry),
        &config(0, retry),
        ScriptedProver::instant(),
        ScriptedSettler::settling("0xfeed"),
        chain,
    );
    h.reach("a", ReturnStatus::Settled).await;
    assert_eq!(h.batch_of("a").rebases, 1);
    assert_eq!(h.prover.calls(), 1);
    assert_eq!(h.record("a").settle_txid.as_deref(), Some("0xfeed"));
}

#[tokio::test(start_paused = true)]
async fn restart_retries_recoverable_failures_on_schedule() {
    let dir = tempfile::tempdir().unwrap();
    let retry = RetryPolicy::default();
    let h = Harness::start(
        reopen(dir.path(), retry),
        &config(0, retry),
        ScriptedProver::new(vec![ProveStep::Fail("oom".to_string())]),
        ScriptedSettler::settling("0xfeed"),
        FakeChainLog::new(),
    );
    h.accept("a", 11, 0x61);
    h.reach("a", ReturnStatus::Failed).await;
    h.stop().await;

    let h = Harness::start(
        reopen(dir.path(), retry),
        &config(0, retry),
        ScriptedProver::instant(),
        ScriptedSettler::settling("0xfeed"),
        FakeChainLog::new(),
    );
    tokio::task::yield_now().await;
    assert_eq!(h.status("a"), ReturnStatus::Failed);
    assert_eq!(h.prover.calls(), 0);
    h.reach_in("a", ReturnStatus::Settled, Duration::from_secs(120))
        .await;
    assert_eq!(h.record("a").attempts, 1);
}

#[tokio::test(start_paused = true)]
async fn restart_leaves_final_failures_alone() {
    let dir = tempfile::tempdir().unwrap();
    let parked = retry(1, 3);
    let h = Harness::start(
        reopen(dir.path(), parked),
        &config(0, parked),
        ScriptedProver::new(vec![ProveStep::Fail("oom".to_string())]),
        ScriptedSettler::settling("0xfeed"),
        FakeChainLog::new(),
    );
    h.accept("a", 11, 0x61);
    h.reach("a", ReturnStatus::Failed).await;
    assert!(!h.record("a").failure.unwrap().recoverable);
    h.stop().await;

    let h = Harness::start(
        reopen(dir.path(), parked),
        &config(0, parked),
        ScriptedProver::instant(),
        ScriptedSettler::settling("0xfeed"),
        FakeChainLog::new(),
    );
    tokio::time::sleep(HOUR).await;
    assert_eq!(h.status("a"), ReturnStatus::Failed);
    assert!(!h.record("a").failure.unwrap().recoverable);
    assert_eq!(h.prover.calls(), 0);
    assert_eq!(h.store.health().queue_depth, 0);
}
