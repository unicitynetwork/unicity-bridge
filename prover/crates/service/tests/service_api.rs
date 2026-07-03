use std::time::Duration;

use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use bridge_return_host::fixture::{build_b1_direct_bridge_fixture, build_split_bridge_fixture};
use bridge_return_service::{
    config::ServiceConfig, prover::Prover, queue, router, sequencer::ChainEvents,
    store::ReturnStore, submitter::Submitter, AppState,
};
use serde_json::Value;
use tower::ServiceExt;

fn app(max_wait: Duration, batch_target: usize) -> axum::Router {
    let config = ServiceConfig {
        max_wait,
        batch_target,
        ..ServiceConfig::default()
    };
    let store = ReturnStore::default();
    let queue = queue::spawn(
        store.clone(),
        Prover::new(config.clone()),
        Submitter::none(),
        ChainEvents::none(),
        config.batch_target,
        config.max_wait,
    );
    router(AppState {
        config,
        store,
        queue,
        intake: None,
        chain_events: ChainEvents::none(),
    })
}

#[tokio::test]
async fn health_and_accumulator_are_served() {
    let app = app(Duration::from_secs(60), 1);
    let response = app
        .clone()
        .oneshot(Request::get("/health").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let response = app
        .oneshot(Request::get("/accumulator").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn post_return_prechecks_enqueues_and_is_idempotent() {
    let app = app(Duration::from_millis(20), 1);
    let wire =
        bridge_return_guest::wire::encode_guest_input(&build_b1_direct_bridge_fixture().input);
    let body = serde_json::json!({ "wireInput": format!("0x{}", hex::encode(wire)) }).to_string();

    let response = app
        .clone()
        .oneshot(
            Request::post("/returns")
                .header("content-type", "application/json")
                .body(Body::from(body.clone()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let created: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(created["duplicate"], false);
    assert_eq!(created["terminal"], false);
    assert_eq!(created["success"], Value::Null);
    assert_eq!(created["progress"], 20);
    assert_eq!(created["nextPollMs"], 5000);
    let id = created["returnId"].as_str().unwrap();

    let response = app
        .clone()
        .oneshot(
            Request::post("/returns")
                .header("content-type", "application/json")
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let duplicate: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(duplicate["duplicate"], true);
    assert_eq!(duplicate["returnId"].as_str().unwrap(), id);

    tokio::time::sleep(Duration::from_millis(60)).await;
    wait_status(&app, id, "proven").await;
}

#[tokio::test]
async fn rejects_truncated_wire() {
    let app = app(Duration::from_secs(60), 1);
    let response = app
        .oneshot(
            Request::post("/returns")
                .header("content-type", "application/json")
                .body(Body::from(r#"{"wireInput":"0x0001"}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let error: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(error["error"]["code"], "precheck_rejected");
    assert_eq!(error["error"]["recoverable"], true);
}

#[tokio::test]
async fn queue_closes_when_batch_target_is_reached() {
    let app = app(Duration::from_secs(30), 2);
    let first = post_wire(&app, build_b1_direct_bridge_fixture().input).await;
    let second = post_wire(&app, build_split_bridge_fixture().input).await;

    tokio::time::sleep(Duration::from_millis(80)).await;
    wait_status(&app, first.as_str(), "proven").await;
    wait_status(&app, second.as_str(), "proven").await;
}

#[tokio::test]
async fn nullifier_lookup_and_unknown_batch_404() {
    let app = app(Duration::from_secs(60), 1);
    let wire =
        bridge_return_guest::wire::encode_guest_input(&build_b1_direct_bridge_fixture().input);
    let body = serde_json::json!({ "wireInput": format!("0x{}", hex::encode(wire)) }).to_string();
    let response = app
        .clone()
        .oneshot(
            Request::post("/returns")
                .header("content-type", "application/json")
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let created: Value = serde_json::from_slice(&bytes).unwrap();
    let id = created["returnId"].as_str().unwrap().to_string();
    let nullifier = created["nullifier"].as_str().unwrap().to_string();

    // GET /returns?nullifier= resolves to the same record (wallet idempotency).
    let response = app
        .clone()
        .oneshot(
            Request::get(format!("/returns?nullifier={nullifier}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let found: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(found["returnId"].as_str().unwrap(), id);

    // An unknown batch is a clean 404.
    let response = app
        .oneshot(
            Request::get("/batches/0xdeadbeef")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

async fn post_wire(app: &axum::Router, input: bridge_return_guest::GuestInput) -> String {
    let wire = bridge_return_guest::wire::encode_guest_input(&input);
    let body = serde_json::json!({ "wireInput": format!("0x{}", hex::encode(wire)) }).to_string();
    let response = app
        .clone()
        .oneshot(
            Request::post("/returns")
                .header("content-type", "application/json")
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let created: Value = serde_json::from_slice(&bytes).unwrap();
    created["returnId"].as_str().unwrap().to_string()
}

async fn assert_status(app: &axum::Router, id: &str, status: &str) {
    let response = app
        .clone()
        .oneshot(
            Request::get(format!("/returns/{id}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let record: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(record["status"], status);
    assert_eq!(record["terminal"], false);
    assert_eq!(record["progress"], 70);
    assert!(record["events"].as_array().unwrap().len() >= 3);
}

async fn wait_status(app: &axum::Router, id: &str, status: &str) {
    for _ in 0..20 {
        let response = app
            .clone()
            .oneshot(
                Request::get(format!("/returns/{id}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let record: Value = serde_json::from_slice(&bytes).unwrap();
        if record["status"] == status {
            return;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    assert_status(app, id, status).await;
}
