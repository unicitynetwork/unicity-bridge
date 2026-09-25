mod support;

use std::time::Duration;

use bridge_return_service::{
    clock::Clock,
    config::ServiceConfig,
    domain::{batch::BatchStatus, policy::BatchPolicy, retry::RetryPolicy},
    orchestrator::Orchestrator,
    store::{ErrorKind, ReturnRecord, ReturnStatus, ReturnStore},
    submitter::SubmitOutcome,
};
use support::{
    burn_and_record, member, nullifier_of, FakeChainLog, ProveStep, ScriptedProver, ScriptedSettler,
};

const HOUR: Duration = Duration::from_secs(3_600);
const MINUTE: u128 = 60_000;

struct Harness {
    store: ReturnStore,
    prover: ScriptedProver,
    settler: ScriptedSettler,
    chain: FakeChainLog,
    clock: Clock,
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
            .run(),
        );
        Self {
            store,
            prover,
            settler,
            chain,
            clock,
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
    assert!(failure.message.contains("diverged from chain"), "{}", failure.message);
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
async fn already_settled_member_is_excluded_and_rest_proceeds() {
    let h = Harness::simple(
        ScriptedProver::instant(),
        ScriptedSettler::settling("0xfeed"),
    );
    h.chain
        .settle_externally(&[nullifier_of(&member(11, 0x61))]);
    h.accept("a", 11, 0x61);
    h.accept("b", 12, 0x62);
    h.reach("b", ReturnStatus::Settled).await;
    let a = h.record("a");
    assert_eq!(a.status, ReturnStatus::Settled);
    assert_eq!(a.settle_txid, None);
    assert!(a.message.contains("already released"));
    assert_eq!(h.record("b").settle_txid.as_deref(), Some("0xfeed"));
    assert_eq!(h.settler.submitted()[0].leaves.len(), 1);
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
    chain.settle_externally(&[a_nullifier]);

    let h = Harness::start(
        reopen(dir.path(), retry),
        &config(0, retry),
        ScriptedProver::instant(),
        ScriptedSettler::settling("0xfeed"),
        chain,
    );
    h.reach("a", ReturnStatus::Settled).await;
    assert_eq!(h.record("a").settle_txid, None);
    assert_eq!(h.prover.calls(), 0);
    assert!(h.settler.submitted().is_empty());
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
