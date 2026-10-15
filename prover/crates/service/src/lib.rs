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
    /// The deployment config + trust base every admitted burn is checked against.
    /// The binary always sets it; `None` is the fixture mode of in-process tests,
    /// which post pre-assembled `wireInput`s certified by fixture validators.
    pub intake: Option<Arc<EnvelopeIntake>>,
    /// S2/S3 chain-sync seam — reconstructs the vault's accumulator so proofs
    /// chain onto its current `spentRoot` (`BRIDGE_RETURN_EVENTS_CMD`).
    pub chain_events: sequencer::ChainEvents,
}

/// Load the envelope intake (deployment JSON + trust base). The service admits
/// only burns certified under the deployment's trust base, so both are required.
pub fn load_intake(config: &config::ServiceConfig) -> Result<Arc<EnvelopeIntake>, String> {
    let (Some(dep), Some(tb)) = (&config.deployment_config_path, &config.trust_base_path) else {
        return Err("BRIDGE_DEPLOYMENT_CONFIG and TRUST_BASE_PATH are required".to_string());
    };
    let dep_json = std::fs::read_to_string(dep).map_err(|e| format!("read {dep:?}: {e}"))?;
    let tb_json = std::fs::read_to_string(tb).map_err(|e| format!("read {tb:?}: {e}"))?;
    let intake = EnvelopeIntake::from_json(&dep_json, &tb_json, config.justification_tag)
        .map_err(|e| format!("intake init: {e}"))?;
    Ok(Arc::new(intake))
}

/// The S4 submitter and settlement simulation from the environment. The simulation
/// checks lock refs before proving, so the service fails closed without it; only
/// in-process tests (no intake) run a submitter without one.
pub fn load_submitter(config: &config::ServiceConfig) -> Result<submitter::Submitter, String> {
    submitter::Submitter::from_env()
        .with_timeout(config.command_timeout)
        .require_simulation()
}

pub fn router(state: AppState) -> Router {
    api::router(Arc::new(state))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_intake_is_required_so_the_service_fails_closed_without_a_deployment() {
        let Err(err) = load_intake(&config::ServiceConfig::default()) else {
            panic!("started without a deployment");
        };
        assert!(err.contains("BRIDGE_DEPLOYMENT_CONFIG"), "{err}");
    }

    // The only test that touches BRIDGE_RETURN_SIMULATE_CMD.
    #[test]
    fn the_simulation_is_required_so_the_service_fails_closed_without_a_lock_check() {
        std::env::remove_var("BRIDGE_RETURN_SIMULATE_CMD");
        let Err(err) = load_submitter(&config::ServiceConfig::default()) else {
            panic!("started without a settlement simulation");
        };
        assert!(err.contains("BRIDGE_RETURN_SIMULATE_CMD"), "{err}");
        std::env::set_var(
            "BRIDGE_RETURN_SIMULATE_CMD",
            "node scripts/relayer-eth.js simulate --stdin",
        );
        let loaded = load_submitter(&config::ServiceConfig::default());
        std::env::remove_var("BRIDGE_RETURN_SIMULATE_CMD");
        assert!(loaded.is_ok());
    }
}
