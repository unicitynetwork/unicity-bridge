/**
 * Wallet integration surface (06 §A2.2). The single entry point a wallet (Sphere)
 * imports — façade (plan builders) + manifest (loader/type) + the `TronSigner`
 * abstraction. Kept as a subpath, *not* merged into the package root, so the
 * read-only re-exports here don't collide with the root's bridge-back exports.
 */
export * from './facade.js';
export * from './manifests.js';
export * from './tron-signer.js';
export * from './allowance.js';
export * from './return-client.js';
export * from './self-mint-verifier.js';
