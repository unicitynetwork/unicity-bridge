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

## ⚠ reason_tag (39048 vs 39050)
The live Nile vault `TNXx9Pv6T8L983y3FM66xBYRip5G4MQH2a` is **reason_tag 39050**
(its on-chain `config_hash e06d52d9…`). Commit `3dcd2a7` moved the canonical tag to
**39048** in the generic `bridge-vectors`/Rust/deploy-script but never redeployed.
The wallet manifest + `nile-usdt.json` therefore track the live vault (**39050**) so
the demo works now. To move to 39048: run `contracts/tron/scripts/deploy-nile.js`
(already 39048) and update `NILE_USDT_BRIDGE` (vault + configHash) from the new
deployment. (config_hash binds the self-stamped vault address, so it's only known
after deploy.)
