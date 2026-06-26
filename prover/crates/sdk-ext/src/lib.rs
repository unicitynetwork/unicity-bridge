//! Bridge-return extensions over the read-only Rust token SDK.
//!
//! This crate is prover-owned. It keeps bridge-return-only machinery out of
//! `state-transition-sdk-rust` while reusing the SDK's public token, CBOR,
//! predicate, and proof types.

#![no_std]
#![forbid(unsafe_code)]

extern crate alloc;

pub mod accumulator;
pub mod bridge;
pub mod trust;
pub mod verify;

mod error;

pub use error::{BridgeExtError, Result};
