#![forbid(unsafe_code)]

pub mod api;
pub mod clock;
pub mod config;
pub mod domain;
pub mod journal;
pub mod orchestrator;
pub mod ports;
pub mod prover;
pub mod sequencer;
pub mod store;
pub mod submitter;

use std::sync::Arc;

use axum::Router;
use bridge_return_host::s1::EnvelopeIntake;
use clock::Clock;
use store::ReturnStore;

#[derive(Clone)]
pub struct AppState {
    pub config: config::ServiceConfig,
    pub store: ReturnStore,
    pub clock: Clock,
    /// Loaded when `BRIDGE_DEPLOYMENT_CONFIG` + `TRUST_BASE_PATH` are set — enables
    /// the wallet `{tokenCbor, reasonBytes}` envelope intake (else only `wireInput`).
    pub intake: Option<Arc<EnvelopeIntake>>,
    /// S2/S3 chain-sync seam — reconstructs the vault's accumulator so proofs
    /// chain onto its current `spentRoot` (`BRIDGE_RETURN_EVENTS_CMD`).
    pub chain_events: sequencer::ChainEvents,
}

/// Load the envelope intake from config (deployment JSON + trust base), if both
/// paths are set. Returns `Ok(None)` when unconfigured (wire-input-only mode).
pub fn load_intake(config: &config::ServiceConfig) -> Result<Option<Arc<EnvelopeIntake>>, String> {
    let (Some(dep), Some(tb)) = (&config.deployment_config_path, &config.trust_base_path) else {
        return Ok(None);
    };
    let dep_json = std::fs::read_to_string(dep).map_err(|e| format!("read {dep:?}: {e}"))?;
    let tb_json = std::fs::read_to_string(tb).map_err(|e| format!("read {tb:?}: {e}"))?;
    let intake = EnvelopeIntake::from_json(&dep_json, &tb_json, config.justification_tag)
        .map_err(|e| format!("intake init: {e}"))?;
    Ok(Some(Arc::new(intake)))
}

pub fn router(state: AppState) -> Router {
    api::router(Arc::new(state))
}
