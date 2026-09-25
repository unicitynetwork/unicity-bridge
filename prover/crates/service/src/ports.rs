use std::future::Future;

use bridge_return_host::s2::{RebuiltAccumulator, SettledBatch};

use crate::{
    domain::assembler::Rejected,
    prover::{ProofBundle, Prover, ProverError},
    sequencer::{ChainEvents, ChainSyncError},
    store::{BatchBundle, LeafHex},
    submitter::{SimulateError, SubmitOutcome, Submitter},
};

pub trait ProofBackend: Send + Sync + 'static {
    fn prove(
        &self,
        batch_id: &str,
        wire: Vec<u8>,
    ) -> impl Future<Output = Result<ProofBundle, ProverError>> + Send;
}

pub trait Settler: Send + Sync + 'static {
    fn submit(&self, bundle: &BatchBundle) -> impl Future<Output = SubmitOutcome> + Send;

    fn simulate(
        &self,
        leaves: &[LeafHex],
    ) -> impl Future<Output = Result<Vec<Rejected>, SimulateError>> + Send;
}

pub trait ChainLog: Send + Sync + 'static {
    fn synced_accumulator(
        &self,
        known: Vec<SettledBatch>,
    ) -> impl Future<Output = Result<RebuiltAccumulator, ChainSyncError>> + Send;
}

impl ProofBackend for Prover {
    async fn prove(&self, batch_id: &str, wire: Vec<u8>) -> Result<ProofBundle, ProverError> {
        Prover::prove(self, batch_id.to_string(), wire).await
    }
}

impl Settler for Submitter {
    async fn submit(&self, bundle: &BatchBundle) -> SubmitOutcome {
        Submitter::submit(self, bundle).await
    }

    async fn simulate(&self, leaves: &[LeafHex]) -> Result<Vec<Rejected>, SimulateError> {
        Submitter::simulate(self, leaves).await
    }
}

impl ChainLog for ChainEvents {
    async fn synced_accumulator(
        &self,
        known: Vec<SettledBatch>,
    ) -> Result<RebuiltAccumulator, ChainSyncError> {
        ChainEvents::synced_accumulator(self, known).await
    }
}
