# 06 — Wallet bridge integration (Sphere) development plan

**Stack:** TypeScript — `bridge-plugin/` (owns *all* bridge logic),
`sphere-sdk/` (provider passthrough), `sphere/` (UI + wiring). React 19 +
TanStack Query + Vitest.

**Owns:** surfacing the built bridge in/out flows in the real Sphere UI. Design:
[`integration.md`](./integration.md) Part A. Conforms to
[`00-interop-contract.md`](./00-interop-contract.md) §2–5, §7 and reuses
[`02-ts-sdk-and-wallet.md`](./02-ts-sdk-and-wallet.md) (the plugin + bridge-back
module are already built).

**Hard boundary (decision #2):** Sphere holds **zero** chain-agnostic bridge code.
Everything — derivations *and* Tron specifics — lives in the plugin package; Sphere
imports a façade + a manifest and renders UI.

---

## Architecture recap (what's built vs new)

- **Built:** `createTronUsdtBridgePlugin`, the `IMintJustificationVerifier`,
  `bridge-back/{derivations,burn,cbor,abi}` (`createBridgeBackBurnTransfer`,
  `previewReturn`, `buildWitnessRequest`), the engine hook
  `config.bridgeJustificationVerifiers` (`token-engine/factory.ts`).
- **New (plugin package):** a wallet-facing **façade** + a **`TronSigner`** abstraction
  with TronLink/WalletConnect/managed impls.
- **New (sphere-sdk):** a `bridges?: BridgeManifest[]` passthrough in the browser
  provider factory (no core engine change).
- **New (sphere):** `BridgeModal`, the "Bridge" action button, three hooks, a
  `BridgeStore`.

---

## Phases

Phases are tagged to the cross-cutting milestones in
[`README.md`](./README.md); these are post-M4 integration work.

### W0 — Plugin façade + manifest + provider passthrough (foundation)

**Plugin package (`bridge-plugin/src/`):**
- `wallet/facade.ts` — the only surface Sphere calls:
  - `loadBridges(manifest, deps) → BridgePlugin[]` (wraps `createTronUsdtBridgePlugin`).
  - `buildBridgeInPlan({plugin, amount, owner}) → { tokenId, recipientCommitment, salt, approveTx, lockTx }` (derives the target token + the unsigned Tron txs).
  - `buildBridgeBackPlan({plugin, token, reason}) → { burnTransfer, preview, witnessRequest }` (wraps `createBridgeBackBurnTransfer` + `previewReturn` + `buildWitnessRequest`).
  - re-export `previewReturn`, `decodeBridgeBackReason` for read-only UI.
- `BridgeManifest` type (keyed by `tokenTypeHex`): vault, asset, `chainId`,
  `confirmations`, decimals, `configHash`, `vkey`, `returnServiceUrl`.

**sphere-sdk browser build:**
- Extend the provider/engine config with `bridges?: BridgeManifest[]`; build plugins
  via the façade and forward `bridgeJustificationVerifiers` into
  `createSphereTokenEngine`. **No core engine change** (the factory already loops).

**Deliverables:** façade + manifest type; sphere-sdk passthrough; a
`bridges.<network>.json` manifest for Nile USDT (vault `TLXkaf…`, the Nile test
USDT asset).
**Tests:** façade unit tests; conformance — `buildBridgeInPlan`/`buildBridgeBackPlan`
outputs match `bridge-vectors` (`config`, `lock` recipientCommitment, `reason`,
`nullifier`, `public`).
**Exit:** a bridged token minted out-of-band verifies in Sphere via the manifest;
CI green on vectors.

### W1 — Bridged-asset display (read-only, no signing)

**Sphere (`components/wallet`, `sdk/hooks`):**
- Render bridged balances as Assets with a **"bridged (USDT · Tron)"** badge; value
  via SDK `PaymentAssetCollection`.
- **Degrade-safe**: unknown-`tokenType` / failed-re-verify → **"unverified"**,
  excluded from spendable (`02` M1). Engine re-verifies on receive with the
  receiver's `confirmations: K` (manifest) — surface "confirming (N blocks)".
- **Returnability flag** (time-independent-predicate lineage) computed from the
  token; gates the future "Bridge out".

**Deliverables:** badge + unverified handling + returnability in `L3WalletView` /
`AssetRow`.
**Exit:** a manually-bridged token shows correctly (spendable when verified,
"confirming"/"unverified" otherwise) with no signing path yet.

### W2 — Bridge-in (TronLink) — M-integration

**Plugin (`wallet/tron-signer.ts`):**
- `TronSigner` interface (`getAddress()`, `signTransaction(unsignedTx) → signedTx`,
  `broadcast?`); `TronLinkSigner` (injected `window.tronLink`/`tronWeb`,
  `tron_requestAccounts`, sign; the wallet broadcasts via TronWeb).

**Sphere:**
- `BridgeModal` **Bridge in** tab (modeled on `SendModal` step machine) + the
  **"Bridge"** action button (`L3WalletView.tsx:321`).
- Flow (§A1.1): derive `tokenId`+`recipientCommitment` → `approve` (one-time max) →
  `lock(amount, tokenId, recipientCommitment)` → on **lock acceptance** mint
  (`confirmations: 0`, self-trust) → token spendable. A **"locking on Tron"**
  sub-state between confirm and mint.
- `BridgeStore.persistPendingLock({nonce, salt, recipient, amount, lockTxid})` +
  **resume-mint** on reopen (recovery).

**Deliverables:** `TronLinkSigner`, Bridge-in modal, pending-lock recovery.
**Tests:** Vitest with a mock `TronSigner` + mock Tron RPC (existing
`test/helpers.ts` `MockTronRpc` pattern); resume-mint after simulated crash.
**Exit (demo):** lock→mint **live on Nile** from the real UI with TronLink; token
spendable immediately; reload mid-flow resumes the mint.

### W3 — Bridge-back + return-service handoff — M-integration

**Plugin:** `wallet/return-client.ts` — `ReturnServiceClient`
(`postReturn(witnessRequest) → {returnId,nullifier}`, `getReturn(id)`,
`getByNullifier(η)`), typed to the §B4 API.

**Sphere:**
- `BridgeModal` **Bridge out** tab: pick a returnable bridged balance, Tron dest +
  amount (+ deadline; fee=0 subsidized); partial ⇒ split first then burn the child.
- `createBridgeBackBurnTransfer` (user signs the Unicity burn) → **persist burned
  blob** (`BridgeStore`, recovery-critical) → `previewReturn` shows nullifier/leaf →
  `postReturn`.
- `useBridgeClaims`: poll `/returns/:id` **and** watch `Released{nullifier}` over
  Tron RPC; show `queued→proving→submitted→settled` + the settle txid. **Self-settle
  after deadline** affordance (publish/submit the bundle yourself).

**Deliverables:** return client, Bridge-out modal, claim tracking, burned-blob
persistence + export.
**Tests:** Vitest against a stub return service; the existing live e2e
(`demo/bridge-back-e2e.ts`) becomes the integration backstop (assert wallet-derived
nullifier/leaf == service's).
**Exit (demo):** burn in the UI → service settles → USDT on Nile; wallet shows the
`Released`/`fulfillBatch` txid; self-settle path demonstrated.

### W4 — UX iteration (mobile + polish)

- `WalletConnectSigner` (`@tronweb3/walletconnect-tron`, Reown `projectId`,
  `WalletConnectChainID.Nile`) and `ManagedKeySigner` (the proven `demo/tron.ts`
  path) — both behind `TronSigner`, no flow change.
- Polish: returnability/finality explainers, "final for others in N blocks", batch
  ETA from `/health`, backup-flow inclusion of recovery material, error/empty/
  unverified states.

**Exit:** the same round trip works via WalletConnect (TokenPocket mobile); managed
key works with no extension.

---

## Testing & CI

- Vitest, existing `test/` patterns (`verifier.test.ts`, `bridge-back.test.ts`,
  `MockTronRpc`); a mock `TronSigner` + a stub `ReturnServiceClient`.
- **Conformance** (`bridge-vectors`, `00` §10): façade plan outputs must match
  `config`/`lock`/`reason`/`nullifier`/`public`; CI fails on mismatch or
  `BRIDGE_PROTO_VERSION` skew.
- **Round-trip**: build `BridgeBackReason` → encode → byte-equality with the
  Rust-generated `reason` fixtures.
- `npx tsc --noEmit`, `npm run lint`, `npm run build` in `sphere`.

## Risks

| Risk | Mitigation |
|---|---|
| TronLink injection / Nile selection flakiness | `TronSigner` abstraction; explicit "switch to Nile" guard; WalletConnect/managed fallbacks |
| Chain-agnostic logic leaking into Sphere | enforced by the façade boundary (decision #2); Sphere imports only the façade + manifest |
| User loses burned blob | `BridgeStore` persistence + export + backup inclusion; UX warning (ZK_BACK3 §13) |
| Lock succeeds, mint never runs | pending-lock recovery (resume mint); the lock is on-chain so funds aren't lost |
| Returning a time-dependent-predicate token | returnability flag hides "Bridge out" for such tokens (`00` §8) |
| Manifest trust-anchor misconfig | integrity-pinned manifest (`PLUGIN_ARCHITECTURE.md`); `configHash` cross-checked vs the deployed vault |
