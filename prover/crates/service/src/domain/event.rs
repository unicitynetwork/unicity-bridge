use serde::{Deserialize, Serialize};

use super::{assembler::Exclusion, burn::Burn, ledger::Ledger};
use crate::store::{BatchBundle, ErrorKind, ReturnRecord};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Event {
    Snapshot(Box<Ledger>),
    Accepted {
        burn: Burn,
        record: ReturnRecord,
    },
    Requeued {
        id: String,
        at_ms: u128,
    },
    BatchFormed {
        members: Vec<String>,
        at_ms: u128,
    },
    BatchProven {
        id: String,
        bundle: BatchBundle,
        spent_root_old: String,
        at_ms: u128,
    },
    BatchSettled {
        id: String,
        txid: Option<String>,
        at_ms: u128,
    },
    BatchFailed {
        id: String,
        kind: ErrorKind,
        message: String,
        at_ms: u128,
    },
    BatchRebased {
        id: String,
        at_ms: u128,
    },
    BatchInterrupted {
        id: String,
        at_ms: u128,
        /// Written by a service that stopped in order and abandoned the proof itself, which
        /// costs the members no attempt; a crash leaves the batch proving and recovery
        /// charges one. Absent in journals written before it existed: those were charged.
        #[serde(default)]
        orderly: bool,
    },
    ReturnExcluded {
        id: String,
        reason: Exclusion,
        at_ms: u128,
    },
}
