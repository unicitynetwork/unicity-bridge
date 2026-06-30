/**
 * Built-in {BridgeManifest}s for known deployments. Frozen from
 * `bridge-vectors/deployment/nile-usdt.json` (the cross-stack config freeze —
 * `configHash` equals the deployed vault's on-chain `CONFIG_HASH`). A wallet can
 * import one of these directly instead of shipping its own manifest file.
 */
import type { BridgeManifest } from './manifest.js';

/**
 * Tron Nile testnet USDT — the **live** deployment (2026-06-30). Vault
 * `TD89z57Xksziu3uk24qfjT27bJmeWLgjtk`, reason_tag **39048** (39050 conflicts with
 * `SpherePaymentData.CBOR_TAG`), real SP1 verifier `TN4nQmnVz3H3zDnN77NQZTAfBpzkEdoeBR`
 * (vkey `0x002b42fa…`), the R6 false-tolerant safe-transfer for the non-standard
 * Nile USDT `TXYZopYRdj2D9XRtbG411XZZ3kM5VkAeBf`, push-payment. `configHash` equals
 * the vault's on-chain `CONFIG_HASH` (cross-checked TS == Solidity), the integrity
 * pin. tokenType/coinId are asset-derived (stable across vault redeploys).
 *
 * `returnServiceUrl` defaults to a local Part-B service; override per environment.
 */
export const NILE_USDT_BRIDGE: BridgeManifest = {
  label: 'USDT (bridged · Tron)',
  chainId: 3448148188,
  vault: 'TD89z57Xksziu3uk24qfjT27bJmeWLgjtk',
  asset: 'TXYZopYRdj2D9XRtbG411XZZ3kM5VkAeBf',
  confirmations: 20,
  decimals: 6,
  rpcUrl: 'https://nile.trongrid.io',
  returnServiceUrl: 'http://localhost:8787',
  reasonTag: 39048,
  lockDomain: '158b847f78b3910a5f5f42820de61abba1bf5ae1fbb29dabfba09118f393f932',
  nullifierDomain: 'd4530e4ea58fc8e38f84506e62b421476c3eeec70f4cbebefc32688a510e2d5d',
  vkey: '0x002b42fa331ad29852eca758fb92cc64c41b349c2d982242a6b60f94a0ff0fb3',
  configHash: '594546ae7e114b8c5674b793234a45f72eca7727aa25b0f605200ebf3cae4b93',
  tokenTypeHex: '6f2d10d27abeb4960a7ef19370c965ec090bb4da1f17752be77334e2dde19c74',
  coinIdHex: 'f1634862e1b932acd1c791a1860c62f69c7f55aa6c6115ba631d3bf4a9d8ddbb',
} as const;

/** Override the return-service URL on a manifest (env-specific, keeps the freeze intact). */
export function withReturnServiceUrl(m: BridgeManifest, returnServiceUrl: string): BridgeManifest {
  return { ...m, returnServiceUrl };
}
