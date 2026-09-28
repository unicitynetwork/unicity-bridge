use std::net::SocketAddr;

use bridge_return_service::{
    clock::Clock, config::ServiceConfig, domain::policy::BatchPolicy, load_intake,
    orchestrator::Orchestrator, prover::Prover, router, sequencer::ChainEvents, store::ReturnStore,
    submitter::Submitter, AppState,
};
use tower_http::trace::{DefaultMakeSpan, DefaultOnFailure, DefaultOnResponse, TraceLayer};
use tracing::Level;

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let config = ServiceConfig::from_env().unwrap_or_else(|err| {
        eprintln!("{err}");
        std::process::exit(2);
    });
    let bind: SocketAddr = config.bind;
    let intake = load_intake(&config).unwrap_or_else(|err| {
        eprintln!("envelope intake disabled: {err}");
        None
    });
    if intake.is_some() {
        tracing::info!("envelope intake enabled (deployment config + trust base loaded)");
    } else {
        tracing::warn!("envelope intake disabled — only wireInput submissions accepted");
    }
    let clock = Clock::default();
    let store = match &config.state_dir {
        Some(dir) => ReturnStore::open(dir, config.retry, clock.now_ms()).unwrap_or_else(|err| {
            eprintln!("cannot open state directory {}: {err}", dir.display());
            std::process::exit(2);
        }),
        None => {
            tracing::warn!(
                "BRIDGE_RETURN_STATE_DIR unset — returns are kept in memory and lost on restart"
            );
            ReturnStore::memory(config.retry)
        }
    };
    let submitter = Submitter::from_env().with_timeout(config.command_timeout);
    tracing::info!("S4 submitter: {}", submitter.label());
    let chain_events = ChainEvents::from_env().with_timeout(config.command_timeout);
    tracing::info!("accumulator chain-sync: {}", chain_events.label());
    if !chain_events.is_live() {
        tracing::warn!(
            "no chain watcher (BRIDGE_RETURN_EVENTS_CMD unset) — the vault is assumed pristine \
             (spentRoot=0); settlement will revert with `vault: stale root` once it has settled \
             any prior batch",
        );
    }
    tracing::info!(
        prove_mode = ?config.prove_mode,
        max_batch_size = config.max_batch_size,
        idle_wait_secs = config.idle_wait.as_secs(),
        state_dir = ?config.state_dir,
        vault = config.vault.as_deref().unwrap_or("(none)"),
        "service configuration",
    );
    tokio::spawn(
        Orchestrator::new(
            store.clone(),
            Prover::new(config.clone()),
            submitter,
            chain_events.clone(),
            BatchPolicy::from(&config),
            config.retry,
            clock.clone(),
        )
        .run(),
    );
    let app = router(AppState {
        config,
        store,
        clock,
        intake,
        chain_events,
    })
    .layer(
        TraceLayer::new_for_http()
            // Default TraceLayer spans/logs at DEBUG, which RUST_LOG=info silently
            // drops — every request went completely unlogged. Bump to INFO so
            // `method path status latency` is visible without knowing to also set
            // tower_http=debug.
            .make_span_with(DefaultMakeSpan::new().level(Level::INFO))
            .on_response(DefaultOnResponse::new().level(Level::INFO))
            .on_failure(DefaultOnFailure::new().level(Level::ERROR)),
    );

    let listener = tokio::net::TcpListener::bind(bind)
        .await
        .expect("bind bridge-return-service");
    tracing::info!("bridge-return-service listening on {bind}");
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await
        .expect("serve bridge-return-service");
    tracing::info!("shutting down; an in-flight proof is abandoned and re-run at start");
    std::process::exit(0);
}

async fn shutdown_signal() {
    let _ = tokio::signal::ctrl_c().await;
}
