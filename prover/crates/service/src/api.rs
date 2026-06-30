use std::sync::Arc;

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use bridge_return_guest::wire;
use bridge_return_host::s1;
use serde::{Deserialize, Serialize};
use sha2::Digest;

use crate::{
    sequencer,
    store::{ReturnRecord, ReturnStatus},
    AppState,
};

pub fn router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/accumulator", get(accumulator))
        .route("/returns", post(create_return))
        .route("/returns/:id", get(get_return))
        .with_state(state)
}

#[derive(Debug, Serialize)]
pub struct HealthResponse {
    pub status: &'static str,
    pub queue_depth: usize,
    pub active_batch: Option<String>,
    pub batch_target: usize,
    pub max_wait_ms: u128,
    pub prove_mode: String,
}

async fn health(State(state): State<Arc<AppState>>) -> Json<HealthResponse> {
    Json(HealthResponse {
        status: "ok",
        queue_depth: state.queue.depth().await,
        active_batch: state.queue.active_batch().await,
        batch_target: state.config.batch_target,
        max_wait_ms: state.config.max_wait.as_millis(),
        prove_mode: format!("{:?}", state.config.prove_mode),
    })
}

#[derive(Debug, Serialize)]
pub struct AccumulatorResponse {
    pub synced: bool,
    pub spent_root: String,
    pub spent_count: usize,
}

async fn accumulator() -> Result<Json<AccumulatorResponse>, ApiError> {
    // R0/R1 has no live TronGrid watcher yet. Rebuilding the empty log still
    // exercises the same S2 path and exposes the current disposable state shape.
    let acc = sequencer::rebuild_accumulator(&[])?;
    Ok(Json(AccumulatorResponse {
        synced: true,
        spent_root: format!("0x{}", hex::encode(acc.spent_root)),
        spent_count: acc.spent_count,
    }))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateReturnRequest {
    pub wire_input: String,
}

#[derive(Debug, Serialize)]
pub struct CreateReturnResponse {
    pub return_id: String,
    pub nullifier: String,
    pub status: ReturnStatus,
    pub terminal: bool,
    pub success: Option<bool>,
    pub progress: u8,
    pub message: String,
    pub next_poll_ms: u64,
    pub duplicate: bool,
}

async fn create_return(
    State(state): State<Arc<AppState>>,
    Json(req): Json<CreateReturnRequest>,
) -> Result<Json<CreateReturnResponse>, ApiError> {
    let wire_input = decode_hex(&req.wire_input)?;
    let report = s1::precheck_wire(&wire_input)
        .map_err(|err| ApiError::PrecheckRejected(err.to_string()))?;
    let input = wire::decode_guest_input(&wire_input).map_err(|err| {
        ApiError::BadRequest("invalid_wire_input", format!("wire decode failed: {err:?}"))
    })?;
    if input.return_leaves.len() != 1 {
        return Err(ApiError::BadRequest(
            "unsupported_batch_shape",
            "service intake currently expects one return leaf per submitted wire input".to_string(),
        ));
    }
    if let Some(expected) = state.config.config_hash {
        if report.public_values.config_hash != expected {
            return Err(ApiError::BadRequest(
                "config_hash_mismatch",
                format!(
                    "configHash mismatch: got 0x{}, expected 0x{}",
                    hex::encode(report.public_values.config_hash),
                    hex::encode(expected),
                ),
            ));
        }
    }
    let nullifier = input.return_leaves[0].nullifier;
    let return_id = return_id(&report.public_values_digest, &nullifier);
    let record = ReturnRecord::queued(
        return_id,
        nullifier,
        report.public_values_digest,
        report.public_values,
        wire_input,
    );
    let (record, inserted) = state.store.insert_or_get(record);
    if inserted {
        state.queue.enqueue(record.return_id.clone()).await?;
    }
    Ok(Json(CreateReturnResponse {
        return_id: record.return_id,
        nullifier: record.nullifier,
        status: record.status,
        terminal: record.terminal,
        success: record.success,
        progress: record.progress,
        message: record.message,
        next_poll_ms: record.next_poll_ms,
        duplicate: !inserted,
    }))
}

async fn get_return(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<ReturnRecord>, ApiError> {
    state.store.get(&id).map(Json).ok_or(ApiError::NotFound(id))
}

#[derive(Debug, thiserror::Error)]
pub enum ApiError {
    #[error("{1}")]
    BadRequest(&'static str, String),
    #[error("{0}")]
    PrecheckRejected(String),
    #[error("{0}")]
    Host(#[from] bridge_return_host::HostError),
    #[error("{0}")]
    Queue(#[from] crate::queue::QueueError),
    #[error("return not found: {0}")]
    NotFound(String),
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let status = match self {
            ApiError::BadRequest(_, _)
            | ApiError::PrecheckRejected(_)
            | ApiError::Host(_)
            | ApiError::Queue(_) => StatusCode::BAD_REQUEST,
            ApiError::NotFound(_) => StatusCode::NOT_FOUND,
        };
        let code = match &self {
            ApiError::BadRequest(code, _) => *code,
            ApiError::PrecheckRejected(_) => "precheck_rejected",
            ApiError::Host(_) => "host_error",
            ApiError::Queue(_) => "queue_closed",
            ApiError::NotFound(_) => "not_found",
        };
        let recoverable = matches!(
            &self,
            ApiError::PrecheckRejected(_) | ApiError::Queue(_) | ApiError::Host(_)
        );
        let body = Json(serde_json::json!({
            "error": {
                "code": code,
                "message": self.to_string(),
                "recoverable": recoverable
            }
        }));
        (status, body).into_response()
    }
}

fn decode_hex(input: &str) -> Result<Vec<u8>, ApiError> {
    hex::decode(input.strip_prefix("0x").unwrap_or(input))
        .map_err(|err| ApiError::BadRequest("invalid_hex", format!("invalid wireInput hex: {err}")))
}

fn return_id(public_values_digest: &[u8; 32], nullifier: &[u8; 32]) -> String {
    let mut hasher = sha2::Sha256::new();
    hasher.update(b"bridge-return-service:return-id:v1");
    hasher.update(public_values_digest);
    hasher.update(nullifier);
    format!("0x{}", hex::encode(hasher.finalize()))
}
