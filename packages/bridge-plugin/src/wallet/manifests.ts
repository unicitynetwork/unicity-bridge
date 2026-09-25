/**
 * Built-in {BridgeManifest}s for known deployments. Frozen from
 * `deployments/nile/nile-usdt.json` (the cross-stack config freeze —
 * `configHash` equals the deployed vault's on-chain `CONFIG_HASH`). A wallet can
 * import one of these directly instead of shipping its own manifest file.
 */
import type { BridgeManifest, EvmBridgeManifest, TronBridgeManifest } from './manifest.js';

/**
 * Tron Nile testnet USDT — the **live v2** deployment (2026-09-21). Vault
 * `TBKJ84417jdxo6j92TxQuYpZdRZGaeZVrv`, BridgeBackReason tag **39048** (distinct
 * from SpherePaymentData tag 39050), real SP1 verifier `TN4nQmnVz3H3zDnN77NQZTAfBpzkEdoeBR`
 * (vkey `0x0039a542…` — the BRIDGE_PROTO_VERSION 2 guest, which reads the wallet's
 * value payload and rejects empty burn batches), the R6 false-tolerant safe-transfer
 * (with an explicit energy stipend — TVM doesn't reliably forward all remaining
 * energy to a bare nested `.call()`) for the non-standard Nile USDT
 * `TXYZopYRdj2D9XRtbG411XZZ3kM5VkAeBf`, push-payment. `configHash` equals the
 * vault's on-chain `CONFIG_HASH` (cross-checked TS == Solidity), the integrity
 * pin. tokenType/coinId are asset-derived (stable across vault redeploys).
 *
 * `returnServiceUrl` defaults to a local Part-B service; override per environment.
 *
 * Disabled since 2026-09-22: Tron bounds a transaction to 80 ms of CPU and the
 * Groth16 verification needs more, so no return can settle on any Tron network
 * (docs/OPERATIONS.md §10). The entry stays listed so the wallet keeps showing
 * the deployment and its tokens; it refuses new locks and burns.
 */
export const NILE_USDT_BRIDGE: TronBridgeManifest = {
  family: 'tron',
  label: 'USDT (bridged · Tron)',
  symbol: 'USDT',
  chainRef: 'tron:0xcd8690dc',
  chainId: 3448148188,
  vault: 'TBKJ84417jdxo6j92TxQuYpZdRZGaeZVrv',
  asset: 'TXYZopYRdj2D9XRtbG411XZZ3kM5VkAeBf',
  confirmations: 20,
  decimals: 6,
  rpcUrl: 'https://nile.trongrid.io',
  returnServiceUrl: 'http://localhost:8787',
  reasonTag: 39048,
  lockDomain: '158b847f78b3910a5f5f42820de61abba1bf5ae1fbb29dabfba09118f393f932',
  nullifierDomain: 'd4530e4ea58fc8e38f84506e62b421476c3eeec70f4cbebefc32688a510e2d5d',
  vkey: '0x0039a5424014e57caf45d3451053e6c014547837ae09c9eb724aa569389b90d5',
  configHash: 'fa77a13a6fb24658fa75377b0eef7cf3e92f8caede9ea127afe21be7a036cb1b',
  tokenTypeHex: '6f2d10d27abeb4960a7ef19370c965ec090bb4da1f17752be77334e2dde19c74',
  coinIdHex: 'f1634862e1b932acd1c791a1860c62f69c7f55aa6c6115ba631d3bf4a9d8ddbb',
  disabledReason: 'Tron limits a transaction to 80 ms of CPU, less than the proof verification needs, so returns cannot settle. Bridging on Tron is paused.',
} as const;

export const NILE_USDT_BRIDGE_V1: TronBridgeManifest = {
  ...NILE_USDT_BRIDGE,
  vault: 'TTKKLyhnRRQ7XV5vsRarV8xWWEvF9225mY',
  vkey: '0x00c34ae0ebb63e86218a754892813f4744b2f6c9ed613c085ea40999b16ce3ad',
  configHash: '7f376b16b3bff3455f375e7cf30b9d29d2a14332912f0ffb69d78e1b31d5193f',
} as const;

export const SEPOLIA_USDC_BRIDGE: EvmBridgeManifest = {
  family: 'eip155',
  label: 'USDC (bridged · Ethereum)',
  symbol: 'USDC',
  chainRef: 'eip155:11155111',
  chainId: 11155111,
  vault: '0x9C2BF4Ed5b85130fFD14BE8FA65c60F299Fc9a2E',
  asset: '0x1c7D4B196Cb0C7B01d743Fbc6116a902379C7238',
  confirmations: 12,
  decimals: 6,
  rpcUrl: 'https://sepolia.gateway.tenderly.co',
  returnServiceUrl: 'http://localhost:8787',
  reasonTag: 39048,
  lockDomain: '158b847f78b3910a5f5f42820de61abba1bf5ae1fbb29dabfba09118f393f932',
  nullifierDomain: 'd4530e4ea58fc8e38f84506e62b421476c3eeec70f4cbebefc32688a510e2d5d',
  vkey: '0x0039a5424014e57caf45d3451053e6c014547837ae09c9eb724aa569389b90d5',
  configHash: '4058e87dff330d9c6925af755e0732dbc80be85643fdce46c8315eae51c9f7e8',
  tokenTypeHex: '2ccbf3157add2b9a2dcc10e772abf5cf328e2723f9f290a9d2b6c4a42a132d6c',
  coinIdHex: 'eae954053183b9d1836d6b5c892867014bcc1571fcc6813f5b56b16a78d0497f',
} as const;

/** Override the return-service URL on a manifest (env-specific, keeps the freeze intact). */
export function withReturnServiceUrl<M extends BridgeManifest>(m: M, returnServiceUrl: string): M {
  return { ...m, returnServiceUrl };
}
