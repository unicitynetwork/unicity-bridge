use std::{
    collections::HashMap,
    sync::{Arc, RwLock},
    time::{SystemTime, UNIX_EPOCH},
};

use bridge_return_core::PublicValues;
use serde::{Deserialize, Serialize};

#[derive(Clone, Default)]
pub struct ReturnStore {
    inner: Arc<RwLock<StoreInner>>,
}

#[derive(Default)]
struct StoreInner {
    by_id: HashMap<String, ReturnRecord>,
    by_nullifier: HashMap<String, String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReturnStatus {
    Queued,
    Proving,
    Proven,
    Submitted,
    Settled,
    Failed,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ReturnRecord {
    pub return_id: String,
    pub nullifier: String,
    pub status: ReturnStatus,
    pub batch_id: Option<String>,
    pub error: Option<String>,
    pub batch_size: u32,
    pub total_amount: String,
    pub public_values_digest: String,
    pub public_values: PublicValuesHex,
    #[serde(skip)]
    pub wire_input: Vec<u8>,
    pub created_at_ms: u128,
    pub updated_at_ms: u128,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct PublicValuesHex {
    pub domain_tag: String,
    pub config_hash: String,
    pub trust_base_hash: String,
    pub spent_root_old: String,
    pub spent_root_new: String,
    pub return_root: String,
    pub lock_ref_root: String,
    pub batch_size: u32,
    pub total_amount: String,
}

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("return not found: {0}")]
    NotFound(String),
}

impl ReturnStore {
    pub fn insert_or_get(&self, record: ReturnRecord) -> (ReturnRecord, bool) {
        let mut guard = self.inner.write().expect("store poisoned");
        if let Some(existing_id) = guard.by_nullifier.get(&record.nullifier) {
            if let Some(existing) = guard.by_id.get(existing_id) {
                return (existing.clone(), false);
            }
        }
        guard
            .by_nullifier
            .insert(record.nullifier.clone(), record.return_id.clone());
        guard.by_id.insert(record.return_id.clone(), record.clone());
        (record, true)
    }

    pub fn get(&self, id: &str) -> Option<ReturnRecord> {
        self.inner
            .read()
            .expect("store poisoned")
            .by_id
            .get(id)
            .cloned()
    }

    pub fn queued(&self) -> Vec<ReturnRecord> {
        self.inner
            .read()
            .expect("store poisoned")
            .by_id
            .values()
            .filter(|r| r.status == ReturnStatus::Queued)
            .cloned()
            .collect()
    }

    pub fn update_status(
        &self,
        id: &str,
        status: ReturnStatus,
        batch_id: Option<String>,
        error: Option<String>,
    ) -> Result<ReturnRecord, StoreError> {
        let mut guard = self.inner.write().expect("store poisoned");
        let record = guard
            .by_id
            .get_mut(id)
            .ok_or_else(|| StoreError::NotFound(id.to_string()))?;
        record.status = status;
        if batch_id.is_some() {
            record.batch_id = batch_id;
        }
        record.error = error;
        record.updated_at_ms = now_ms();
        Ok(record.clone())
    }
}

impl ReturnRecord {
    pub fn queued(
        return_id: String,
        nullifier: [u8; 32],
        public_values_digest: [u8; 32],
        public_values: PublicValues,
        wire_input: Vec<u8>,
    ) -> Self {
        let now = now_ms();
        Self {
            return_id,
            nullifier: hex32(&nullifier),
            status: ReturnStatus::Queued,
            batch_id: None,
            error: None,
            batch_size: public_values.batch_size,
            total_amount: hex32(&public_values.total_amount),
            public_values_digest: hex32(&public_values_digest),
            public_values: PublicValuesHex::from(public_values),
            wire_input,
            created_at_ms: now,
            updated_at_ms: now,
        }
    }
}

impl From<PublicValues> for PublicValuesHex {
    fn from(value: PublicValues) -> Self {
        Self {
            domain_tag: hex32(&value.domain_tag),
            config_hash: hex32(&value.config_hash),
            trust_base_hash: hex32(&value.trust_base_hash),
            spent_root_old: hex32(&value.spent_root_old),
            spent_root_new: hex32(&value.spent_root_new),
            return_root: hex32(&value.return_root),
            lock_ref_root: hex32(&value.lock_ref_root),
            batch_size: value.batch_size,
            total_amount: hex32(&value.total_amount),
        }
    }
}

fn hex32(value: &[u8; 32]) -> String {
    format!("0x{}", hex::encode(value))
}

fn now_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time before unix epoch")
        .as_millis()
}
