mod support;

use std::time::Duration;

use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use bridge_return_host::fixture::{build_b1_direct_bridge_fixture, build_split_bridge_fixture};
use bridge_return_service::{
    clock::Clock,
    config::ServiceConfig,
    domain::{event::Event, policy::BatchPolicy, retry::RetryPolicy},
    orchestrator::Orchestrator,
    prover::Prover,
    router,
    sequencer::ChainEvents,
    store::{ErrorKind, ReturnStore},
    AppState,
};
use serde_json::Value;
use support::{member, ScriptedSettler};
use tower::ServiceExt;

fn app(idle_wait: Duration, max_batch_size: usize) -> axum::Router {
    app_with(
        idle_wait,
        max_batch_size,
        ScriptedSettler::skipping(),
        RetryPolicy::default(),
    )
    .0
}

fn app_with(
    idle_wait: Duration,
    max_batch_size: usize,
    settler: ScriptedSettler,
    retry: RetryPolicy,
) -> (axum::Router, ReturnStore) {
    let config = ServiceConfig {
        idle_wait,
        max_batch_size,
        retry,
        ..ServiceConfig::default()
    };
    let store = ReturnStore::memory(retry);
    let clock = Clock::default();
    tokio::spawn(
        Orchestrator::new(
            store.clone(),
            Prover::new(config.clone()),
            settler,
            ChainEvents::none(),
            BatchPolicy::from(&config),
            retry,
            clock.clone(),
        )
        .run(),
    );
    let app = router(AppState {
        config,
        store: store.clone(),
        clock,
        intake: None,
        chain_events: ChainEvents::none(),
    });
    (app, store)
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
    assert_eq!(error["error"]["recoverable"], false);
}

#[tokio::test]
async fn resubmitting_a_recoverably_failed_return_requeues_it() {
    let immediate = RetryPolicy {
        base: Duration::ZERO,
        ..RetryPolicy::default()
    };
    let (app, store) = app_with(
        Duration::from_millis(20),
        1,
        ScriptedSettler::skipping(),
        immediate,
    );
    let input = build_b1_direct_bridge_fixture().input;
    let id = post_wire(&app, input.clone()).await;
    wait_status(&app, &id, "proven").await;
    let batch_id = store.get(&id).unwrap().batch_id.unwrap();
    store.apply(Event::BatchFailed {
        id: batch_id,
        kind: ErrorKind::SubmissionFailed,
        message: "out of gas".to_string(),
        at_ms: 0,
    });
    let failed = get_record(&app, &id).await;
    assert_eq!(failed["status"], "failed");
    assert_eq!(failed["failure"]["recoverable"], true);
    assert_eq!(failed["attempts"], 1);

    let again = post_return(&app, input).await;
    assert_eq!(again["returnId"].as_str().unwrap(), id);
    assert_eq!(again["duplicate"], false);
    assert_eq!(again["status"], "queued");
    wait_status(&app, &id, "proven").await;
}

#[tokio::test]
async fn burns_sharing_a_lock_nonce_prove_in_separate_batches() {
    let app = app(Duration::from_millis(100), 8);
    let first = post_wire(&app, build_b1_direct_bridge_fixture().input).await;
    let second = post_wire(&app, build_split_bridge_fixture().input).await;

    wait_status(&app, first.as_str(), "proven").await;
    wait_status(&app, second.as_str(), "proven").await;
    let first_batch = get_record(&app, &first).await["batchId"].clone();
    let second_batch = get_record(&app, &second).await["batchId"].clone();
    assert!(first_batch.is_string());
    assert_ne!(first_batch, second_batch);
}

#[tokio::test]
async fn two_posts_become_one_batch_and_settle() {
    let (app, _) = app_with(
        Duration::from_millis(100),
        8,
        ScriptedSettler::settling("0xfeed"),
        RetryPolicy::default(),
    );
    let first = post_wire(&app, member(11, 0x61)).await;
    let second = post_wire(&app, member(12, 0x62)).await;

    wait_status(&app, &first, "settled").await;
    wait_status(&app, &second, "settled").await;
    let first_record = get_record(&app, &first).await;
    let second_record = get_record(&app, &second).await;
    let batch_id = first_record["batchId"].as_str().unwrap().to_string();
    assert_eq!(second_record["batchId"], batch_id);
    assert_eq!(first_record["settleTxid"], "0xfeed");
    assert_eq!(second_record["settleTxid"], "0xfeed");
    assert_eq!(first_record["success"], true);

    let response = app
        .clone()
        .oneshot(
            Request::get(format!("/batches/{batch_id}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let bundle: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(bundle["leaves"].as_array().unwrap().len(), 2);
    assert_eq!(bundle["lockRefs"].as_array().unwrap().len(), 2);
    assert_eq!(bundle["settleTxid"], "0xfeed");
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
    post_return(app, input).await["returnId"]
        .as_str()
        .unwrap()
        .to_string()
}

async fn post_return(app: &axum::Router, input: bridge_return_guest::GuestInput) -> Value {
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
    serde_json::from_slice(&bytes).unwrap()
}

async fn get_record(app: &axum::Router, id: &str) -> Value {
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
    serde_json::from_slice(&bytes).unwrap()
}

async fn wait_status(app: &axum::Router, id: &str, status: &str) {
    let mut record = Value::Null;
    for _ in 0..120 {
        record = get_record(app, id).await;
        if record["status"] == status {
            assert!(record["events"].as_array().unwrap().len() >= 3);
            return;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    panic!("return {id} never reached {status}: {record}");
}
