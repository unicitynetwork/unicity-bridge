/**
 * Built-in {BridgeManifest}s for known deployments. Frozen from
 * `bridge-vectors/deployment/nile-usdt.json` (the cross-stack config freeze —
 * `configHash` equals the deployed vault's on-chain `CONFIG_HASH`). A wallet can
 * import one of these directly instead of shipping its own manifest file.
 */
import type { BridgeManifest } from './manifest.js';

/**
 * Tron Nile testnet USDT — the **live** deployment (2026-07-02). Vault
 * `TMckEpYxv8QA7oL36FvFRR7Gg1bL5DHsbt`, reason_tag **39048** (39050 conflicts with
 * `SpherePaymentData.CBOR_TAG`), real SP1 verifier `TN4nQmnVz3H3zDnN77NQZTAfBpzkEdoeBR`
 * (vkey `0x00d75299…` — the guest relation decodes the `SpherePaymentData` envelope
 * real Sphere-minted tokens actually use), the R6 false-tolerant safe-transfer
 * (with an explicit energy stipend — TVM doesn't reliably forward all remaining
 * energy to a bare nested `.call()`) for the non-standard Nile USDT
 * `TXYZopYRdj2D9XRtbG411XZZ3kM5VkAeBf`, push-payment. `configHash` equals the
 * vault's on-chain `CONFIG_HASH` (cross-checked TS == Solidity), the integrity
 * pin. tokenType/coinId are asset-derived (stable across vault redeploys).
 *
 * `returnServiceUrl` defaults to a local Part-B service; override per environment.
 */
export const NILE_USDT_BRIDGE: BridgeManifest = {
  label: 'USDT (bridged · Tron)',
  symbol: 'USDT',
  chainId: 3448148188,
  vault: 'TMckEpYxv8QA7oL36FvFRR7Gg1bL5DHsbt',
  asset: 'TXYZopYRdj2D9XRtbG411XZZ3kM5VkAeBf',
  confirmations: 20,
  decimals: 6,
  rpcUrl: 'https://nile.trongrid.io',
  returnServiceUrl: 'http://localhost:8787',
  reasonTag: 39048,
  lockDomain: '158b847f78b3910a5f5f42820de61abba1bf5ae1fbb29dabfba09118f393f932',
  nullifierDomain: 'd4530e4ea58fc8e38f84506e62b421476c3eeec70f4cbebefc32688a510e2d5d',
  vkey: '0x00d75299dfc01ff06af28435bb830f6b477eb8d4eb88b760e4daee04b496b000',
  configHash: '4aea387d6a39eaa40b620b3989266919537d2dedf22dc6de8cb9c2d31a3d905a',
  tokenTypeHex: '6f2d10d27abeb4960a7ef19370c965ec090bb4da1f17752be77334e2dde19c74',
  coinIdHex: 'f1634862e1b932acd1c791a1860c62f69c7f55aa6c6115ba631d3bf4a9d8ddbb',
} as const;

/** Override the return-service URL on a manifest (env-specific, keeps the freeze intact). */
export function withReturnServiceUrl(m: BridgeManifest, returnServiceUrl: string): BridgeManifest {
  return { ...m, returnServiceUrl };
}
