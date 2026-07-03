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
    store::{ReturnRecord, ReturnStatus},
    AppState,
};

pub fn router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/accumulator", get(accumulator))
        .route("/returns", post(create_return).get(get_return_by_nullifier))
        .route("/returns/:id", get(get_return))
        .route("/batches/:id", get(get_batch))
        .with_state(state)
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HealthResponse {
    pub status: &'static str,
    pub queue_depth: usize,
    pub active_batch: Option<String>,
    pub batch_target: usize,
    pub max_wait_ms: u128,
    pub prove_mode: String,
    pub chain_sync: String,
}

async fn health(State(state): State<Arc<AppState>>) -> Json<HealthResponse> {
    Json(HealthResponse {
        status: "ok",
        queue_depth: state.queue.depth().await,
        active_batch: state.queue.active_batch().await,
        batch_target: state.config.batch_target,
        max_wait_ms: state.config.max_wait.as_millis(),
        prove_mode: format!("{:?}", state.config.prove_mode),
        chain_sync: state.chain_events.label().to_string(),
    })
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AccumulatorResponse {
    pub synced: bool,
    pub spent_root: String,
    pub spent_count: usize,
}

async fn accumulator(
    State(state): State<Arc<AppState>>,
) -> Result<Json<AccumulatorResponse>, ApiError> {
    // Reconstruct the vault's accumulator from its on-chain settlement events
    // (via the configured watcher), verified against the live `spentRoot`. With
    // no watcher this rebuilds the empty log (`spent_root = 0`, pristine vault).
    let acc = state
        .chain_events
        .synced_accumulator()
        .await
        .map_err(|e| ApiError::ChainUnsynced(e.to_string()))?;
    Ok(Json(AccumulatorResponse {
        synced: state.chain_events.is_live(),
        spent_root: format!("0x{}", hex::encode(acc.spent_root)),
        spent_count: acc.spent_count,
    }))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateReturnRequest {
    /// The wallet witness envelope (02 §2c) — preferred. Built into the guest wire
    /// input in-service (requires the deployment config + trust base, §intake).
    #[serde(default)]
    pub token_cbor: Option<String>,
    #[serde(default)]
    pub reason_bytes: Option<String>,
    #[serde(default)]
    pub config_hash: Option<String>,
    #[serde(default)]
    pub anchor_hint: Option<String>,
    /// Alternative: a pre-assembled guest wire input (fixtures / relayers).
    #[serde(default)]
    pub wire_input: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
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
    let wire_input = build_wire_input(&state, &req)?;
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
        tracing::info!(
            return_id = %record.return_id,
            nullifier = %record.nullifier,
            "return accepted and queued",
        );
    } else {
        tracing::info!(
            return_id = %record.return_id,
            nullifier = %record.nullifier,
            "return already known (idempotent re-submit)",
        );
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

/// `GET /batches/:id` — the published bundle (vkey, publicValues, proofBytes) for
/// a proven batch; anyone can submit it to the vault (self-settle, §B4).
async fn get_batch(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<crate::store::BatchBundle>, ApiError> {
    state
        .store
        .get_batch(&id)
        .map(Json)
        .ok_or(ApiError::NotFound(id))
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
    #[error("accumulator not synced to chain: {0}")]
    ChainUnsynced(String),
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
            ApiError::ChainUnsynced(_) => StatusCode::SERVICE_UNAVAILABLE,
            ApiError::NotFound(_) => StatusCode::NOT_FOUND,
        };
        let code = match &self {
            ApiError::BadRequest(code, _) => *code,
            ApiError::PrecheckRejected(_) => "precheck_rejected",
            ApiError::Host(_) => "host_error",
            ApiError::Queue(_) => "queue_closed",
            ApiError::ChainUnsynced(_) => "chain_unsynced",
            ApiError::NotFound(_) => "not_found",
        };
        let recoverable = matches!(
            &self,
            ApiError::PrecheckRejected(_)
                | ApiError::Queue(_)
                | ApiError::Host(_)
                | ApiError::ChainUnsynced(_)
        );
        // Centralized so every rejection path (current and future) is audit-able
        // from the log alone — this is what "why did that submit fail" resolves
        // to when the caller only reports a code, not the full message.
        if status == StatusCode::NOT_FOUND {
            tracing::debug!(code, message = %self, "not found");
        } else {
            tracing::warn!(code, recoverable, message = %self, "request rejected");
        }
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

/// Resolve the request into the guest wire input: prefer the wallet envelope
/// (`tokenCbor` + `reasonBytes`, built in-service), else a pre-assembled `wireInput`.
fn build_wire_input(state: &AppState, req: &CreateReturnRequest) -> Result<Vec<u8>, ApiError> {
    match (&req.token_cbor, &req.reason_bytes) {
        (Some(token_cbor), Some(reason_bytes)) => {
            let intake = state.intake.as_ref().ok_or_else(|| {
                ApiError::BadRequest(
                    "intake_unconfigured",
                    "envelope intake is not configured (set BRIDGE_DEPLOYMENT_CONFIG + TRUST_BASE_PATH)"
                        .to_string(),
                )
            })?;
            if let Some(declared) = &req.config_hash {
                let declared = decode_hex(declared)?;
                if declared != intake.config_hash() {
                    return Err(ApiError::BadRequest(
                        "config_hash_mismatch",
                        format!(
                            "configHash mismatch: got 0x{}, service uses 0x{}",
                            hex::encode(&declared),
                            hex::encode(intake.config_hash()),
                        ),
                    ));
                }
            }
            intake
                .build_wire_input(&decode_hex(token_cbor)?, &decode_hex(reason_bytes)?)
                .map_err(|err| ApiError::PrecheckRejected(err.to_string()))
        }
        (None, None) => {
            let wire = req.wire_input.as_ref().ok_or_else(|| {
                ApiError::BadRequest(
                    "missing_payload",
                    "provide {tokenCbor, reasonBytes} (wallet envelope) or wireInput".to_string(),
                )
            })?;
            decode_hex(wire)
        }
        _ => Err(ApiError::BadRequest(
            "incomplete_envelope",
            "tokenCbor and reasonBytes must be provided together".to_string(),
        )),
    }
}

#[derive(Debug, Deserialize)]
pub struct NullifierQuery {
    pub nullifier: String,
}

/// `GET /returns?nullifier=` — wallet idempotency / recovery lookup (§B4).
async fn get_return_by_nullifier(
    State(state): State<Arc<AppState>>,
    axum::extract::Query(q): axum::extract::Query<NullifierQuery>,
) -> Json<Option<ReturnRecord>> {
    Json(state.store.get_by_nullifier(&q.nullifier))
}

fn decode_hex(input: &str) -> Result<Vec<u8>, ApiError> {
    hex::decode(input.strip_prefix("0x").unwrap_or(input))
        .map_err(|err| ApiError::BadRequest("invalid_hex", format!("invalid hex: {err}")))
}

fn return_id(public_values_digest: &[u8; 32], nullifier: &[u8; 32]) -> String {
    let mut hasher = sha2::Sha256::new();
    hasher.update(b"bridge-return-service:return-id:v1");
    hasher.update(public_values_digest);
    hasher.update(nullifier);
    format!("0x{}", hex::encode(hasher.finalize()))
}
