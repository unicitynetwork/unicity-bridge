/**
 * Bridge-back construction (02-ts-sdk-and-wallet.md Part 2): chain-agnostic,
 * authority-free derivations the wallet/relayer produces for the return path.
 * Every byte here is consumed by the prover (Rust) and the vault (Solidity), so
 * it is conformance-tested against `bridge-vectors` (00 §10).
 */
export * as bridgeCbor from './cbor.js';
export * as bridgeAbi from './abi.js';
export * from './derivations.js';
