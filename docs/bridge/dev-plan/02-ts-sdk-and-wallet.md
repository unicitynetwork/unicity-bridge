# 02 — TS SDK & wallet integration

**Stack:** TypeScript (Node ≥ 22 + browser), `@unicitylabs/state-transition-sdk`,
`@noble/hashes`, `fetch`-only (no `tronweb`). Lives in `bridge-plugin-tron-usdt/`
(extend), `state-transition-sdk-js/` (burn flow, if missing), and
`sphere-sdk/token-engine/` (wiring).

**Owns, two halves:**
1. **Bridge-in verifier plugin** — the wallet-side re-check that a bridged token's
   genesis is backed by a real source-chain lock. **Built**; this plan finalizes
   and freezes it.
2. **Bridge-back construction** — building the burn (canonical `BridgeBackReason`
   in auxiliary data, bound by `BurnPredicate(H(reasonBytes))`), deriving the
   nullifier/leaf, and handing the burned blob to a prover. **New.**

Conforms to [`00-interop-contract.md`](./00-interop-contract.md) §2–5, §7. The TS
side never proves and never settles; it constructs and verifies.

**Reference:** `PLUGIN_ARCHITECTURE.md`, `MINT_REASON.md`, ZK_BACK3 §4, §10.

---

## Current state

`bridge-plugin-tron-usdt/` is a complete `IMintJustificationVerifier`:

- `createTronUsdtBridgePlugin(config)` → `{cborTag, tokenTypeHex, coinIdHex,
  decimals, verifier}` (`src/index.ts`).
- `TronUsdtMintJustificationVerifier.verify()` (`src/TronUsdtMintJustificationVerifier.ts`):
  decodes the lock justification, checks trust anchors (chainId / lockContract /
  assetContract), token type, lock-tx success + confirmations over Tron RPC,
  amount, and the `{nonce, unicityTokenId, recipientCommitment}` binding.
- Derivations in `src/identifiers.ts` (`deriveTokenType`, `deriveCoinId`,
  `recipientCommitment`) — these are now frozen in `00` §2/§3.
- Registered via `MintJustificationVerifierService.register(verifier)`;
  `sphere-sdk/token-engine/factory.ts` is the wiring point (`PLUGIN_ARCHITECTURE.md`).

Extension point is generic (`MintJustificationVerifierService`,
`IMintJustificationVerifier{tag, verify}`), so **no core SDK change** is needed
for plugins.

---

## Part 1 — Bridge-in plugin (finalize)

### M1 deliverables
- **Freeze derivations** against `bridge-vectors/config`: assert
  `deriveTokenType`/`deriveCoinId` and `recipientCommitment` match the fixtures
  (`00` §2/§3) in `test/`.
- **Plugin manifest** (`PLUGIN_ARCHITECTURE.md` "unknown asset"): ship a registry
  JSON keyed by `tokenTypeHex` that names the plugin package + version + config,
  alongside `unicity-ids.<network>.json`. Add the `EngineConfig.bridgePlugins`
  hook in `sphere-sdk/token-engine/factory.ts` (register right after
  `SplitMintJustificationVerifier`).
- **Degrade-safe** unknown-asset handling: a token whose `tokenType` has no
  trusted plugin is shown "unverified" and excluded from spendable balance.
- **Finality realization parity:** the verifier's `confirmations`-based finality
  must match the circuit/spec `LockFinal` contract (`appendix-bridging.tex`
  external finality). Keep `confirmations` configurable per chain (`00` config).

### Note on the fresh vault
When the contract team lands the fresh ZK_BACK3 vault (`01`), the `lockContract`
trust anchor in plugin config points at the new vault address; the `Lock` event
fields are unchanged, so `decodeLockEvent` and the verifier need no logic change —
only a config (address) update and redeploy coordination.

---

## Part 2 — Bridge-back construction (new)

The wallet's job on return is small and authority-free, but every byte it
produces is consumed by the circuit, so it is conformance-critical.

### 2a. Burn the token (M2)

- Build `BridgeBackReason` CBOR exactly per `00` §4 (tag `reasonTag`, 11 fields,
  deterministic CBOR). Provide a typed builder + encoder in a new module
  `bridge-plugin-*/src/bridge-back-reason.ts` (or a shared
  `@unicitylabs/bridge-core` package — see "Shared core" below).
- Burn the source token so the terminal transfer carries the canonical
  `reasonBytes` in its **auxiliary data** and its recipient predicate is
  `BurnPredicate(H(reasonBytes))` — *not* `BurnPredicate(reasonBytes)` (`00` §4).
  The JS SDK already has `BurnPredicate` and burns tokens for splits
  (`payment/TokenSplit.ts`); reuse that path, hashing the reason for the predicate
  and attaching the bytes as aux data. For a **partial** return, split first
  (existing split flow) and burn the child whose value equals `amount`.
- Only tokens whose lineage uses **time-independent predicates** can be returned in
  v1 (`00` §8); the wallet should surface a token whose history needs validation
  time (timelock/HTLC) as not-yet-bridgeable-back rather than letting the prove
  step fail later.
- Persist the **burned token blob** as critical recovery material (ZK_BACK3 §13):
  if lost before settlement, the release is unrecoverable. Surface this in wallet
  UX.

### 2b. Derive nullifier + return leaf (M2)

- Implement, in TS, the **read-only** derivations the prover will also compute, so
  the wallet can show the user the pending `nullifier` / release leaf and so a
  self-service relayer can be written in TS:
  - `reasonHash = H(reasonBytes)` (the value the burn predicate binds, `00` §4)
  - `burnTransitionId = H("unicity-burn-transition:v1", burnStateId, burnTxHash)`
  - `nullifier = H("unicity-bridge-return-nullifier:v1", configHash, burnTransitionId)`
  - `returnLeaf = (nullifier, recipient, amount, feeRecipient, feeAmount, deadline)`
- These must match `bridge-vectors/nullifier` and `/public` exactly (SHA-256/CBOR
  for the nullifier; keccak/ABI for the leaf encoding — `00` §1).

### 2c. Hand off to a prover (M2 → M4)

- Define the **witness-request envelope** the wallet/relayer posts to a prover:
  `{ tokenCbor, configHash, anchorHint?, reason }`. The prover (`03`) fetches
  anchor + inclusion proofs itself; the wallet supplies only what it owns.
- M4: optional thin **relayer client** in TS that polls the vault's `spentRoot`,
  rebuilds the accumulator from `Released`/`BatchFulfilled` events (it can reuse
  the read-only derivations in 2b), and submits `fulfillBatch`. This makes "anyone
  can self-settle after the deadline" (ZK_BACK3 §13) a concrete, shippable tool.

---

## Shared core (recommended)

Factor the **chain-agnostic** derivations (config/reason/nullifier/leaf encoders,
`recipientCommitment`, `tokenType/coinId` derivation, `configHash`) into a small
`@unicitylabs/bridge-core` TS package, depended on by both
`bridge-plugin-tron-usdt` and any future per-chain plugin. The Tron-specific RPC +
event decoding stays in the per-chain plugin. This mirrors
`PLUGIN_ARCHITECTURE.md` "logic vs configuration" and keeps the conformance
surface in one place.

---

## Testing & CI

- Vitest, existing `test/` pattern (`verifier.test.ts`, `justification.test.ts`,
  helpers with a mock `TronRpc`).
- **Conformance**: consume `bridge-vectors` (`00` §10) — TS must pass `config`,
  `reason`, `nullifier`, `lock` (recipientCommitment), `public`. CI fails on
  mismatch and on `BRIDGE_PROTO_VERSION` skew.
- Round-trip: build `BridgeBackReason` → encode → assert byte-equality with the
  Rust-generated `reason` fixtures (this is the cross-stack CBOR guard).
- Integration (M2+): in the devnet e2e, mint a bridged token (bridge-in), burn it
  with a `BridgeBackReason`, and assert the wallet-derived nullifier/leaf equal
  the prover's.

## Risks

| Risk | Mitigation |
|---|---|
| TS CBOR canonicalization differs from Rust | byte-equality vectors for `reason`; reuse the SDK's existing deterministic-CBOR encoder |
| keccak vs SHA-256 misuse on a field | `00` §1 table is the only reference; leaf-encoding tests pin it |
| Wallet loses burned blob | UX warns; blob is recovery-critical (ZK_BACK3 §13) |
| Plugin trust-anchor misconfig (wrong `lockContract`) | config is part of the security model; manifest is integrity-pinned (`PLUGIN_ARCHITECTURE.md`) |
