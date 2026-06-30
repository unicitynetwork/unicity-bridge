#![forbid(unsafe_code)]

pub mod api;
pub mod config;
pub mod prover;
pub mod queue;
pub mod sequencer;
pub mod store;

use std::sync::Arc;

use axum::Router;
use queue::QueueHandle;
use store::ReturnStore;

#[derive(Clone)]
pub struct AppState {
    pub config: config::ServiceConfig,
    pub store: ReturnStore,
    pub queue: QueueHandle,
}

pub fn router(state: AppState) -> Router {
    api::router(Arc::new(state))
}
