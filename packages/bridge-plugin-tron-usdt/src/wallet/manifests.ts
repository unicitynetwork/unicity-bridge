/**
 * Built-in {BridgeManifest}s for known deployments. Frozen from
 * `deployments/nile/nile-usdt.json` (the cross-stack config freeze —
 * `configHash` equals the deployed vault's on-chain `CONFIG_HASH`). A wallet can
 * import one of these directly instead of shipping its own manifest file.
 */
import type { BridgeManifest } from './manifest.js';

/**
 * Tron Nile testnet USDT — the **live** deployment (2026-07-03). Vault
 * `TTKKLyhnRRQ7XV5vsRarV8xWWEvF9225mY`, reason_tag **39048** (39050 conflicts with
 * `SpherePaymentData.CBOR_TAG`), real SP1 verifier `TN4nQmnVz3H3zDnN77NQZTAfBpzkEdoeBR`
 * (vkey `0x00c34ae0…` — matches the current prover guest ELF `sp1-vkey`; the guest
 * relation decodes the `SpherePaymentData` envelope real Sphere-minted tokens use),
 * the R6 false-tolerant safe-transfer
 * (with an explicit energy stipend — TVM doesn't reliably forward all remaining
 * energy to a bare nested `.call()`) for the non-standard Nile USDT
 * `TXYZopYRdj2D9XRtbG411XZZ3kM5VkAeBf`, push-payment. `configHash` equals the
 * vault's on-chain `CONFIG_HASH` (cross-checked TS == Solidity), the integrity
 * pin. tokenType/coinId are asset-derived (stable across vault redeploys).
 *
 * `returnServiceUrl` defaults to a local Part-B service; override per environment.
 */
export const NILE_USDT_BRIDGE: BridgeManifest = {
  family: 'tron',
  label: 'USDT (bridged · Tron)',
  symbol: 'USDT',
  chainRef: 'tron:0xcd8690dc',
  chainId: 3448148188,
  vault: 'TTKKLyhnRRQ7XV5vsRarV8xWWEvF9225mY',
  asset: 'TXYZopYRdj2D9XRtbG411XZZ3kM5VkAeBf',
  confirmations: 20,
  decimals: 6,
  rpcUrl: 'https://nile.trongrid.io',
  returnServiceUrl: 'http://localhost:8787',
  reasonTag: 39048,
  lockDomain: '158b847f78b3910a5f5f42820de61abba1bf5ae1fbb29dabfba09118f393f932',
  nullifierDomain: 'd4530e4ea58fc8e38f84506e62b421476c3eeec70f4cbebefc32688a510e2d5d',
  vkey: '0x00c34ae0ebb63e86218a754892813f4744b2f6c9ed613c085ea40999b16ce3ad',
  configHash: '7f376b16b3bff3455f375e7cf30b9d29d2a14332912f0ffb69d78e1b31d5193f',
  tokenTypeHex: '6f2d10d27abeb4960a7ef19370c965ec090bb4da1f17752be77334e2dde19c74',
  coinIdHex: 'f1634862e1b932acd1c791a1860c62f69c7f55aa6c6115ba631d3bf4a9d8ddbb',
} as const;

/** Override the return-service URL on a manifest (env-specific, keeps the freeze intact). */
export function withReturnServiceUrl(m: BridgeManifest, returnServiceUrl: string): BridgeManifest {
  return { ...m, returnServiceUrl };
}
