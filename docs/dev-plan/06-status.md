# 06 — Wallet bridge integration: build status

Tracks the implementation of [`06-wallet-bridge-integration.md`](./06-wallet-bridge-integration.md).

## Done (tested / typechecked)

### W0 — Plugin façade + manifest + sphere-sdk passthrough ✅
- **Plugin** `bridge-plugin-tron-usdt/src/wallet/` (subpath `lib/wallet/…`):
  - `manifest.ts` — `BridgeManifest` type + `loadBridges()` with the **configHash
    integrity-pin** (rejects a manifest that doesn't describe the deployed vault).
  - `facade.ts` — `buildBridgeInPlan` (derivation + unsigned Tron `approve`/`lock`
    calls), `buildBridgeBackBurn` / `finalizeBridgeBack`, read-only re-exports.
  - `manifests.ts` — `NILE_USDT_BRIDGE` (frozen from `bridge-vectors/deployment/
    nile-usdt.json`).
  - `tron-signer.ts` — `TronSigner` + `TronLinkSigner` (W2 capability).
  - `return-client.ts` — `ReturnServiceClient` typed to 07 §B4 (W3).
  - Tests: `test/facade.test.ts` (+ 32 total plugin tests green).
- **sphere-sdk:** `SphereInitOptions.{bridgeJustificationVerifiers, bridges}` (via a
  `BridgeInitOptions` mixin) → forwarded into `buildTokenEngine`; `Sphere.bridges`
  getter + `bridgeForCoin()`. Engine **bridged-mint**: `MintParams.{tokenType,
  salt, genesisReason}` + `SphereTokenEngine.mint` custom path +
  `PaymentsModule.bridgeMint()`. `npx tsc --noEmit` clean; wiring test green.

### W1 — Bridged-asset display ✅ (badge)
- `AssetRow` renders a `bridgedLabel` badge; `L3WalletView` passes
  `sphere.bridgeForCoin(coinId)?.label`.

### W2 — Bridge-in (TronLink) ✅ (code; live-untested)
- App: `src/bridge/{loadBridges,store,bridgeIn}.ts`, `useBridgeIn` hook,
  `L3/modals/BridgeModal.tsx` (Bridge-in tab), "Bridge USDT" button, `BridgeStore`
  pending-lock persistence + `resumePending()` (resume-mint on reopen).

## Remaining
- **W3 — Bridge-out:** needs an engine **burn** path (`createBridgeBackBurnTransfer`
  → certify, modeled on `bridgeMint`), a `useBridgeClaims` hook (poll `/returns/:id`
  + watch `Released{nullifier}`), burned-blob persistence (BridgeStore has the slot),
  and the Part-B return service (07) to settle. Modal has the bridge-out scaffold.
- **W4 —** WalletConnect + managed-key `TronSigner`s; finality/unverified polish.

## How to run the demo
1. **Link local packages** (the app pins published `sphere-sdk@0.10.7`; the bridge
   work lives in this workspace's sphere-sdk + the plugin):
   - build the plugin: `cd bridge-plugin-tron-usdt && npm run build`
   - link the local sphere-sdk into `sphere/` (npm/yarn link or a `file:` override),
   - `cd sphere && npm install` (picks up the `file:` plugin dep + linked sphere-sdk).
2. Optional: `VITE_BRIDGE_RETURN_SERVICE_URL=…` for bridge-out.
3. `cd sphere && npm run dev`. With TronLink on Nile + faucet USDT, "Bridge USDT" →
   Bridge in: approve + lock (TronLink prompts) → mint → spendable "USDT (bridged ·
   Tron)" with the badge.

## reason_tag 39048 (live)
39050 collides with `SpherePaymentData.CBOR_TAG`, so the canonical tag is **39048**.
The Nile vault was redeployed at 39048 (2026-06-30):
- vault **`TD89z57Xksziu3uk24qfjT27bJmeWLgjtk`** (real SP1 verifier
  `TN4nQmnVz3H3zDnN77NQZTAfBpzkEdoeBR`, R6 false-tolerant safe-transfer, push-payment)
- on-chain `CONFIG_HASH 0x594546ae7e114b8c5674b793234a45f72eca7727aa25b0f605200ebf3cae4b93`
  == TS == Rust (cross-stack freeze verified; prover `nile_config` test green).

`NILE_USDT_BRIDGE`, `bridges.nile.json`, `nile-usdt.json` and the prover guard all
point at this vault. The old 39050 vault `TNXx9Pv6…` is retired. A heavy vault deploy
needs ~1.42M energy (≈600+ TRX or staked energy) — fund the `TRON_SK` deployer first.
