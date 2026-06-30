use std::net::SocketAddr;

use bridge_return_service::{
    config::ServiceConfig, prover::Prover, queue, router, store::ReturnStore, AppState,
};
use tower_http::trace::TraceLayer;

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
    let store = ReturnStore::default();
    let queue = queue::spawn(
        store.clone(),
        Prover::new(config.clone()),
        config.batch_target,
        config.max_wait,
    );
    let app = router(AppState {
        config,
        store,
        queue,
    })
    .layer(TraceLayer::new_for_http());

    let listener = tokio::net::TcpListener::bind(bind)
        .await
        .expect("bind bridge-return-service");
    tracing::info!("bridge-return-service listening on {bind}");
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await
        .expect("serve bridge-return-service");
}

async fn shutdown_signal() {
    let _ = tokio::signal::ctrl_c().await;
}
