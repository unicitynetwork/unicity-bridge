/**
 * Wallet integration surface (06 §A2.2). The single entry point a wallet (Sphere)
 * imports — façade (plan builders), manifest (loader/type), the family-neutral
 * signer contract and each family's wallets. Kept as a subpath, *not* merged
 * into the package root, so the read-only re-exports here don't collide with
 * the root's bridge-back exports.
 */
export * from './facade.js';
export * from './manifests.js';
export * from './registry.js';
export * from './signer.js';
export * from '../tron/signer.js';
export * from '../tron/providers.js';
export * from '../tron/presentation.js';
export * from '../evm/signer.js';
export * from '../evm/providers.js';
export * from '../evm/presentation.js';
export * from './allowance.js';
export * from './source-adapter.js';
export * from './return-client.js';
export * from './self-mint-verifier.js';
export * from './token-plugin.js';
export * from './backing.js';
