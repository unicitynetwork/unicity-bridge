/**
 * Built-in {BridgeManifest}s for known deployments. Frozen from
 * `bridge-vectors/deployment/nile-usdt.json` (the cross-stack config freeze —
 * `configHash` equals the deployed vault's on-chain `CONFIG_HASH`). A wallet can
 * import one of these directly instead of shipping its own manifest file.
 */
import type { BridgeManifest } from './manifest.js';

/**
 * Tron Nile testnet USDT — reason_tag **39048** (39050 conflicts), real SP1
 * verifier, false-tolerant safe-transfer for the non-standard Nile USDT, asset
 * `TXYZopYRdj2D9XRtbG411XZZ3kM5VkAeBf`. tokenType/coinId are asset-derived (stable
 * across redeploys); `configHash` binds the self-stamped vault address.
 *
 * ⚠ PENDING REDEPLOY: the 2026-06-30 `real-vault` deploy ran OUT_OF_ENERGY (the
 * deployer `TPu3AykWeTSC1hBNnAHvqib7Hu9jbpvjG1` hit 0 TRX). `vault`/`configHash`
 * below are from that *failed* attempt and DO NOT exist on-chain. Fund the
 * deployer with Nile TRX, re-run `deploy-nile.js real-vault TXYZ…`, and replace
 * `vault` + `configHash` with the new deployment's values (a fresh deploy yields a
 * new address → new configHash). Until then loadBridges resolves but lock() has
 * no contract to call.
 *
 * `returnServiceUrl` defaults to a local Part-B service; override per environment.
 */
export const NILE_USDT_BRIDGE: BridgeManifest = {
  label: 'USDT (bridged · Tron)',
  chainId: 3448148188,
  vault: 'TDMDSzik8mZiTm7j86BM2mQ2Np8HV9fbCF',
  asset: 'TXYZopYRdj2D9XRtbG411XZZ3kM5VkAeBf',
  confirmations: 20,
  decimals: 6,
  rpcUrl: 'https://nile.trongrid.io',
  returnServiceUrl: 'http://localhost:8787',
  reasonTag: 39048,
  lockDomain: '158b847f78b3910a5f5f42820de61abba1bf5ae1fbb29dabfba09118f393f932',
  nullifierDomain: 'd4530e4ea58fc8e38f84506e62b421476c3eeec70f4cbebefc32688a510e2d5d',
  vkey: '0x002b42fa331ad29852eca758fb92cc64c41b349c2d982242a6b60f94a0ff0fb3',
  configHash: 'd210cca16f54cd8eed996e7148ae7abb320beefad029dc1bfa66933dd9ae0793',
  tokenTypeHex: '6f2d10d27abeb4960a7ef19370c965ec090bb4da1f17752be77334e2dde19c74',
  coinIdHex: 'f1634862e1b932acd1c791a1860c62f69c7f55aa6c6115ba631d3bf4a9d8ddbb',
} as const;

/** Override the return-service URL on a manifest (env-specific, keeps the freeze intact). */
export function withReturnServiceUrl(m: BridgeManifest, returnServiceUrl: string): BridgeManifest {
  return { ...m, returnServiceUrl };
}
