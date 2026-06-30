# Integration design — wallet + return-path service

Two designs, with their detailed development plans in sibling docs:

1. **Wallet integration** — bridge Tron TRC20 (Nile USDT) in/out of Unicity from
   the *real* Sphere wallet UI. Dev plan: [`06-wallet-bridge-integration.md`](./06-wallet-bridge-integration.md).
2. **Return sequencing + proving service** — the off-chain S1–S4 pipeline as one
   self-hosted, trustless, disposable **Rust** server (no Docker, no GPU, no
   proving network). Dev plan: [`07-return-service.md`](./07-return-service.md).

This is the *design* layer over the frozen interop contract
([`00-interop-contract.md`](./00-interop-contract.md)) and the two component
plans it integrates — wallet/TS ([`02-ts-sdk-and-wallet.md`](./02-ts-sdk-and-wallet.md))
and prover ([`03-prover-service.md`](./03-prover-service.md)).

**Status of the open questions:** all resolved at review — see
[§Decisions](#decisions-resolved-at-review). This revision folds them in.

---

## 0. What already exists (the ground we build on)

Everything below the "wallet UI" line and "service daemon" line is built and, in
most cases, **live on Nile + testnet2**. Both designs are mostly *assembly and
surfacing* of proven pieces, not new cryptography. The one genuinely new
protocol-level feature added here is **verification-result caching** (§B6).

| Layer | Built artifact | State |
|---|---|---|
| Source vault | `UnicityBridgeVault` — `lock()` stores `lockDigest`, `fulfillBatch()` verifies Groth16 + advances `spentRoot` + releases TRC20; push & pull payment | Live Nile `TLXkafFeuPdNFd3XczCzHSXztbqKCytHWW`, vkey `0x002b42fa…` |
| On-chain verifier | SP1 v6.1.0 Groth16 verifier (`bn254`, ~218k energy) | Live Nile `TN4nQmnVz3H3zDnN77NQZTAfBpzkEdoeBR` |
| Bridge-in verifier | `bridge-plugin-tron-usdt` — `IMintJustificationVerifier` (re-checks the lock over Tron RPC), config/identifier derivations, `createTronUsdtBridgePlugin()` | Built; mint→receive e2e live on Nile |
| Bridge-back construction | `bridge-plugin-tron-usdt/src/bridge-back/` — `createBridgeBackBurnTransfer`, `previewReturn`, `buildWitnessRequest`, all keccak/SHA-256 derivations | Built; live burn e2e (`demo/bridge-back-e2e.ts`) |
| Prover core + guest | `prover/crates/{core,guest,sdk-ext}` — full return relation `R(x,w)`, anchored + **certified** burn verification, depth-256 SMT accumulator, structural lock-backing verifier | B=1 + B=2 real Groth16 settled live on Nile |
| Host services (lib) | `prover/crates/host` — `s1` (witness/precheck + live aggregator fetch + certified verify), `s2` (accumulator rebuild/next-batch), `sp1` (execute/groth16/export) | Built; multi-batch continuity settled live |
| Relayer (S4) | `contracts/tron/scripts/relayer.{js,-lib.js}` — `scan`, `settle` | Built (Node); **superseded by an all-Rust submitter**, §B7 |

**The honest gaps** (both designs respect these):

- **Sphere has no Tron signer.** Bridge-in needs a Tron `approve`+`lock`. This is
  the one new wallet capability — §A1.3.
- **Anchored-mode batching of *live* tokens is limited by the aggregator API**
  (`get_inclusion_proof.v2` serves only the current root, no target-root snapshot).
  Addressed by **best-effort inclusion replacement + verification-result caching**
  (§B6): live tokens prove in certified mode, but the dominant per-transition BFT
  quorum verifications are *deduplicated by seal*, recovering most of the
  anchored-mode saving without the missing endpoint.
- **Multi-burn *certified* witness assembly is B=1 today**
  (`s1::build_certified_guest_input`). Live batching needs a multi-burn certified
  `GuestInput` builder (S1 extension, §B6/§B-plan).

---

# Part A — Wallet integration (Sphere UI for Tron USDT)

Goal: from the real Sphere wallet, a user bridges Nile USDT **in** (lock → mint a
spendable bridged token) and **back** (burn → release to a Tron address), with the
return settled by the Part-B service.

## A1. Flow design

### A1.1 Bridge-in (Tron → Unicity: lock → mint)

The order is load-bearing and already proven in `demo/e2e.ts`: the lock **commits
to a specific Unicity `tokenId` + recipient**, so the wallet mints *that* token.
**Locking and minting are performed by the same entity, which trusts its own
lock**, so **lock-tx acceptance (in a block) is enough to mint** — the minter uses
`confirmations: 0`. There is *no* K-confirmation wait on the mint path.

```
 [Sphere UI]                         [Tron / Nile]                [Unicity gateway]
     │  1. user picks "Bridge in", asset = USDT, amount
     │  2. wallet derives the target token locally:
     │       salt → tokenId   (TokenId.fromSalt)
     │       ownerPredicate → recipientCommitment
     │  3. build approve + lock(amount, tokenId, recipientCommitment) ──► sign on Tron (§A1.3)
     │                                          │ approve (~23k energy)
     │                                          │ lock()  (~126k energy) → Lock{nonce,…}, stores lockDigest[nonce]
     │  4. lock tx ACCEPTED (in a block) — minter trusts its own lock ◄┘  (no K wait)
     │  5. mint the bridged token with the lock as justification ─────────────────► submitCertificationRequest
     │       (TronUsdtLockJustification CBOR; value = PaymentAssetCollection[coinId]=amount)
     │  6. Token.mint(...) — backing structurally verified (confirmations=0) ◄ inclusion proof
     │  7. token appears immediately as a spendable "USDT (bridged)" balance
```

UI is the existing `SendModal` step machine (`asset → details → confirm →
processing → success`) with a **"locking on Tron"** sub-state. Bridge-in
completes — token spendable by the minter — as soon as the lock is in a block and
the mint certifies. Fast for the demo.

**Where finality actually bites (and why mint can skip it).** External finality is
enforced **at first transfer-out**, not at mint: the bridge plugin's `confirmations`
is *the threshold the verifying party enforces*. The minter passes `0`; the **first
independent receiver** of the transferred token passes `K` (the demo uses 20). So
when the bridged token is sent to *another* identity, that receiver re-verifies the
lock over Tron RPC and **requires K confirmations on the `Lock` event** — waiting
(showing "confirming") if the K-window hasn't elapsed, and rejecting if the lock
were reorged out. The minter carries the reorg risk on its own token only; no third
party accepts it as final until K. This is the existing `e2e.ts` model — the wallet
just surfaces it: a freshly-minted bridged token is spendable by the owner
immediately, and "final for others in N blocks".

**Recovery-critical (bridge-in).** If the app dies after `lock()` but before mint,
the deposit is recoverable — `lockDigest[nonce]` is on-chain and the salt/predicate
are deterministic from the wallet. Persist the pending `{nonce, salt, recipient,
amount, lockTxid}` so the mint resumes.

### A1.2 Bridge-back (Unicity → Tron: burn → release)

The user signs only the Unicity burn; **no Tron signing** to receive (the Part-B
service submits `fulfillBatch`).

```
 [Sphere UI]                                  [Part-B service]            [Tron / Nile vault]
     │  1. user picks "Bridge out", a bridged USDT balance, enters Tron dest + amount
     │     (+ deadline, fee — §A2.4; fee=0 while subsidized)
     │  2. if partial: split first (existing TokenSplit flow), burn the child == amount
     │  3. createBridgeBackBurnTransfer(token, cfg, reason) → burn to BurnPredicate(H(reasonBytes)),
     │     reasonBytes in aux data; certify on the gateway (user signs)
     │  4. PERSIST burned-token blob (recovery-critical) + previewReturn → show nullifier/leaf
     │  5. POST buildWitnessRequest({tokenCbor, configHash, reasonBytes}) ─────────► /returns → returnId
     │  6. poll GET /returns/:id ───────────────────────────────────────────────────► queued→proving→submitted→settled
     │                                                            service proves + relays fulfillBatch ─► Released{nullifier}
     │  7. on "settled": USDT at the Tron dest (push) ◄──────────────────────────────────────────────────┘
```

UI honesty: **Returnable vs not** (only time-independent-predicate lineages are
returnable in v1, `00` §8 — others shown *not-yet-bridgeable-back*); and
**self-settle after deadline** (a passed deadline only drops the relayer fee;
principal is always releasable by the recipient/anyone — show "you can claim this
yourself", never "stuck").

### A1.3 Tron signing (TronLink first, behind a `TronSigner` interface)

**Decision:** TronLink has priority for the demo — best/prettiest UX with the least
friction on desktop — but everything sits behind a `TronSigner` interface so the
other backends drop in without touching the flow. Iterate in this order:

| Priority | Backend | How | Notes |
|---|---|---|---|
| **1 (demo)** | **TronLink extension** | injected `window.tronLink` / `tronWeb`; `tron_requestAccounts` then sign; we broadcast via TronWeb | Prettiest desktop UX, reliable on Nile, the canonical Tron dApp path |
| 2 (iterate) | **WalletConnect v2** (`@tronweb3/walletconnect-tron`, v4.x, Reown AppKit) | `connect()` → `{address}`, `signTransaction(tx)`; we broadcast | Mobile + custody-correct (USDT/gas stay in the user's wallet, e.g. TokenPocket); needs a Reown `projectId`. TronLink-over-WC is weak — use TokenPocket/OKX for the WC path |
| 3 (fallback) | **Managed key** | derive/import a Tron key (secp256k1), sign in-browser with `tronweb` (the proven `demo/tron.ts` path) | No extension; self-contained; but the wallet custodies a Tron key and the user must fund that address |

All three implement `TronSigner.signTransaction(unsignedTx) → signedTx`; the wallet
builds the tx and broadcasts (the dApp-broadcasts model). Bridge-back *receiving*
needs no Tron signing in any backend (just a destination address). Two sign
prompts per bridge-in (`approve`, then `lock`); a one-time max `approve` reduces
repeat bridges to one prompt.

## A2. Integration architecture

**Boundary principle (decision #2):** *all* bridge logic — chain-agnostic
derivations and Tron specifics alike — lives in the **plugin package(s)**. Sphere
holds **zero** chain-agnostic bridge code; it only wires a manifest + the plugin's
registered verifier through the provider factory and renders UI. Shared code across
future chains is a **plugin-side** concern (an internal module or a package that
*plugins* depend on), never something Sphere imports directly.

### A2.1 Where it plugs into Sphere / sphere-sdk (Option 1: provider-factory passthrough)

The verifier-registration hook already exists end to end: `token-engine/factory.ts`
accepts `config.bridgeJustificationVerifiers` and registers each into the
`MintJustificationVerifierService`. The missing link is **threading bridge config
through the browser provider factory** the app calls (`createBrowserProviders` /
`createUnicityAggregatorProvider`, `sphere/src/sdk/SphereProvider.tsx:122`):

1. **sphere-sdk browser build** — add an optional `bridges?: BridgeManifest[]`
   passthrough that builds plugins (via the plugin's `createTronUsdtBridgePlugin`)
   and forwards `bridgeJustificationVerifiers` to `createSphereTokenEngine`. **No
   core engine change** (the registry is generic; the factory already loops).
2. **Bridge manifest / network preset** — a JSON keyed by `tokenTypeHex` naming the
   deployed vault, asset, `chainId`, `confirmations`, decimals, `configHash`,
   `vkey`, and the **Part-B service URL**, alongside `unicity-ids.<network>.json`;
   integrity-pinned (`PLUGIN_ARCHITECTURE.md`). Sphere reads the manifest and asks
   the plugin to build everything.

### A2.2 New modules

- **In the plugin package** (chain-agnostic + Tron): the existing bridge-back
  derivations stay; add a small **wallet-facing façade** the UI calls
  (`buildBridgeInPlan`, `buildBridgeBackPlan`, `previewReturn`, manifest→plugin) so
  Sphere never recomputes a hash. Add the `TronSigner` interface + impls here too
  (TronLink / WalletConnect / managed) — they're Tron-specific.
- **In Sphere (UI + wiring only):**
  - `components/wallet/L3/modals/BridgeModal.tsx` — in/out step machine (modeled on
    `SendModal.tsx`), tabs **Bridge in** / **Bridge out**.
  - A **"Bridge"** action button in `L3WalletView.tsx` (next to Top Up / Swap /
    Send, `L3WalletView.tsx:321`).
  - `sdk/hooks/payments/`: `useBridgeIn()`, `useBridgeBack()`, `useBridgeClaims()` —
    React-Query wrappers over the plugin façade + the return-service client.
  - `BridgeStore` (persisted recovery material: pending locks, burned blobs,
    pending returns) — storage only; no protocol logic.

### A2.3 Bridged-asset display & verification

- A bridged token surfaces as an **Asset** (its `coinId`) with a **"bridged (USDT ·
  Tron)"** badge; value via the SDK `PaymentAssetCollection` (cross-stack envelope,
  not the CLI `encodeBridgedValue`).
- **Degrade-safe** (`02` M1): unknown-`tokenType` or failed-re-verify tokens shown
  **"unverified"** and **excluded from spendable balance**. The engine re-runs the
  plugin verifier on every receive (with the receiver's `confirmations: K`).
- **Returnability flag** (time-independent-predicate lineage) gates "Bridge out".

### A2.4 Bridge-back → service handoff + claim tracking

- Post the witness-request envelope from `buildWitnessRequest`
  (`{tokenCbor, configHash, reasonBytes}`) to `/returns`; store `returnId` +
  `nullifier` (idempotency).
- Track state by polling `/returns/:id` **and** independently watching the vault's
  `Released{nullifier}` over Tron RPC — never depend solely on the service to know a
  return settled (trustless display).
- `BridgeBackReason` fields the UI collects: `recipient` (Tron dest), `amount`,
  `feeRecipient`+`feeAmount` (service's Tron address + fee; **0 while subsidized**),
  `deadline` ("claim guaranteed by"). The service applies **best-effort inclusion
  replacement + verification caching** when it proves (§B6) — transparent to the
  wallet, which always sends the same envelope.
- **Push** (default demo, standard TRC20) lands USDT with no extra user action;
  **pull** (blocklistable assets) needs a recipient `withdraw()` — production option.

### A2.5 Recovery-critical material (UX)

- **Burned-token blob** (bridge-back) — lost ⇒ release unrecoverable (ZK_BACK3 §13);
  persist, export, include in backup.
- **Pending lock** (bridge-in) — `{nonce, salt, recipient, amount, lockTxid}` to
  resume a mint after a crash (the lock is already on-chain).

## A3. Demo (Nile USDT, end-to-end, real UI)

1. Point Sphere at the bridge manifest (vault `TLXkaf…`, the **Nile test USDT**
   asset, service URL). Get test USDT from the Nile faucet (decision #4).
2. **Bridge in** with TronLink: lock 5 USDT → mint → "USDT (bridged) 5.0" spendable
   immediately; optionally send a fraction to a 2nd identity to show K-confirmation
   re-verification.
3. **Bridge out**: 2 USDT → Tron address → burn (sign in Sphere) → service proves +
   settles → 2 USDT on Nile; wallet shows the `fulfillBatch`/`Released` txid.
4. Show **self-settle** and **multi-batch** (two bridge-outs caught in one batch
   window).

---

# Part B — Return sequencing + proving service (all Rust)

One long-running **Rust** binary (`bridge-return-service`) that turns submitted
burns into on-chain releases, collapsing S1–S4 (ZK_BACK3 §10) into a single
self-hosted process for **one high-RAM server, no Docker, no GPU, no proving
network**. It is **trustless and disposable**: no authority, no must-not-lose local
state. **Everything is Rust** (decision #3) — the daemon wraps the host crate's
`s1/s2/sp1` directly and includes an all-Rust Tron submitter (§B7), retiring the
Node `relayer.js`.

## B1. Service shape (S1–S4 in one Rust process)

```
                 ┌──────────────────────── bridge-return-service (Rust) ──────────────────────┐
   wallet/relayer │  Intake API ─ precheck (S1) ─► SEQUENTIAL QUEUE ─► Sequencer (S2) ─► Prover (S3) ─► Submitter (S4) │
   POST /returns ─┼─►(validate, dedupe η)        (single-flight,      (batch on        (SP1 CPU →     (Rust Tron:     │
                  │                                max-wait window)     spentRoot,        Groth16)       dry-run, send  │
                  │                                                     witnesses)                       fulfillBatch)  │
                  │   Status store (public-only, rebuildable)     Event watcher (TronGrid scan + verify-vs-chain)      │
                  └────────────────────────────────────────────────────────────────────────────────────────────────┘
        reuses: host s1::*, s2::{rebuild,next_batch}, sp1::{real_groth16,export}; new: axum API, Rust Tron client, verification cache
```

Each stage is an existing host function, not a reimplementation:

- **Intake (S1 precheck).** `s1::WitnessPackage::precheck` / `precheck_wire` — decode
  the burned blob, run the exact guest entry points off-chain, confirm committed ==
  computed public values, round-trip the wire encoding. Build a **certified**
  `GuestInput` (`s1::build_certified_guest_input`; the **multi-burn certified
  assembly is the S1 extension** to write). Inclusion proofs via `s1::aggregator`
  (`http` feature, verified live).
- **Sequencer (S2).** `scan` events (Rust TronGrid client), `s2::rebuild` the
  accumulator, **verify rebuilt root == on-chain `spentRoot`** (SYNCED gate),
  `s2::next_batch` over queued+prechecked nullifiers in return-leaf order → order-
  coupled witnesses + `spent_root_old/new`.
- **Prover (S3).** `sp1::real_groth16` then `sp1::export`. The proven no-Docker /
  no-GPU / no-network `native-gnark` path (§B3).
- **Submitter (S4).** Pre-simulate `fulfillBatch` (`triggerconstantcontract` dry-run),
  drop reverting recipients (push) so one bad transfer can't fail a batch; submit;
  rebase on `"stale root"`. **All Rust** (§B7).

## B2. Trust model & disposability

- **Can't steal** — the vault pays only public leaves proven from burn reasons; the
  destination is bound in the burn. The service holds no user funds/keys beyond its
  own Tron gas account.
- **Can't forge** — only a valid Groth16 proof advances `spentRoot`; nullifier
  non-membership + `spentRootOld == spentRoot` block replay on-chain.
- **Local state is a cache over public data**, rebuilt + verified vs chain on every
  start (`s2::rebuild`, the SYNCED gate — proven live). The only must-not-lose
  artifact is the **burned blob**, which the wallet owns (§A2.5) and anyone can
  resubmit. ⇒ **disposable box: no backup, no migration, no snapshot.**

## B3. CPU-only proving + the sequential queue (decision #5)

Reuse the exact proven path (`03-status.md`): `SP1_PROVER=cpu`,
`SP1_CIRCUIT_MODE=release`, `sp1-sdk` `native-gnark` (gnark via local Go — no
Docker), cached v6.1.0 circuit + 5.86 GB `groth16_pk.bin` on disk. No GPU, no
network key.

- **Strictly sequential proving — one batch at a time.** A single proof saturates
  the machine and parallel proving would thrash/OOM, so the queue is **single-flight
  by design** (decision #5: "avoid parallel proving; have a queue to make it
  sequential"). High RAM is spent *vertically* (lift `SP1_WORKER_NUM_*` above the
  16 GB single-worker floor to parallelize STARK shards *within* one proof and cut
  the ~50–60 min wall-clock), never on concurrent proofs.
- **Configurable max-wait batch window** (decision #5): close a batch when **B
  reaches a target** *or* **`max_wait` elapses**, whichever first. **Demo default
  `max_wait = 60 s`**; production larger (more amortization of the fixed wrap +
  ~218k on-chain verify, `1/N` — §05). A lone burn still settles after `max_wait`.
- **Cost shape `1/N`:** one proof per batch (B=1 ≈ 2.40M cycles, B=2 ≈ 4.76M,
  linear). Horizontal scale later = more boxes pulling the same queue (still one
  proof per box) or §11.4 recursion — no protocol change.

Server prereqs: Rust + Go (native-gnark), ~16+ GB disk for circuit artifacts, lots
of RAM (≥ 64–128 GB to lift worker caps), outbound HTTPS (gateway + TronGrid + the
one-time SP1 circuit bucket), a funded/staked Tron account for gas.

## B4. API surface

| Method | Path | Purpose |
|---|---|---|
| `POST` | `/returns` | submit `{tokenCbor, configHash, reasonBytes}`; returns `{returnId, nullifier, status}`; idempotent on `nullifier`; S1 precheck synchronously rejects bad burns |
| `GET` | `/returns/:id` | status `queued→proving→submitted→settled` (+`failed`/`stale` reason); on settle, the `fulfillBatch` txid + `Released` |
| `GET` | `/returns?nullifier=` | lookup by nullifier (wallet idempotency) |
| `GET` | `/batches/:id` | the published on-chain bundle (`vkey`, `publicValues`, `proofBytes`) — anyone can submit it |
| `GET` | `/accumulator` | rebuilt `spentRoot` + on-chain `spentRoot` + SYNCED flag |
| `GET` | `/health` | prover busy/idle, queue depth, last batch, gas balance |

Unauthenticated is fine (nothing can move funds): enqueue only after a passing S1
precheck, dedupe by nullifier, rate-limit.

## B5. Sequencing policy

Batch on the current verified `spentRoot`; return-leaf order; witnesses against the
running intermediate root (E2 order-coupling, `s2::next_batch`); duplicate/spent
nullifiers rejected pre-prove; reverting recipients dropped in pre-simulation (or
pull mode); `"stale root"` ⇒ rebase; deadline gates only the fee. Batch window per
§B3 (configurable `max_wait`, default 60 s demo).

## B6. Verification-result caching + best-effort inclusion replacement (new)

The expensive in-circuit work is **BFT-quorum (secp256k1) verification** of each
`UnicityCertificate`'s **unicity seal** (the validator-signature set over a root).
Today the guest dedups only **byte-identical anchor `UC*`** (§11). Generalize:

- **Seal-keyed quorum cache.** Key the quorum-verification result by the **seal**
  (the signature set / its content hash), not the whole UC. Two certificates that
  carry the **same seal verify the signatures exactly once**. This catches
  certified-mode duplicates *and* **aggregator sharding**, where distinct UC objects
  can wrap the **same seal** (the user's case: "same unicity seal, different UC").
- **Inclusion-path cache.** Cache inclusion verification by `(root, leaf-key,
  path)` — "same UCs on top of inclusion proofs → verified once."
- **Best-effort inclusion replacement (S1).** When the aggregator *can* serve a
  transition's proof against a shared later root, the witness builder **replaces**
  the per-transition certificate with an anchored inclusion proof against that
  shared root (fewer distinct seals); where it can't, it keeps the certified cert.
  Best-effort = anchor as much as the aggregator allows, then the seal cache dedups
  whatever remains. This recovers most of the anchored-mode quorum saving **without**
  the missing target-root endpoint, and degrades cleanly.

Net effect: a batch (or a single multi-transition lineage, or a sharded set) pays
**one quorum check per distinct seal** instead of one per transition — the §11
saving, generalized, and the path that makes live certified-mode batching cheap.

Implementation surface: a content-addressed cache in `sdk-ext` consulted by the
quorum + inclusion verifiers (host **and** guest); S1 grows the best-effort
replacement + the multi-burn certified `GuestInput` builder; measure the
cycle/quorum-count saving like the existing §11 dedup.

## B7. All-Rust Tron submitter (S4) + ops

- **Rust Tron client** (reqwest + k256 + keccak, deps the SDK already pulls):
  `triggersmartcontract`/`triggerconstantcontract` (dry-run), build + **k256-sign**
  the `fulfillBatch` tx, `broadcasttransaction`, poll the receipt + `Released`
  events. Replaces `relayer.js`/`relayer-lib.js` (kept only as a reference oracle in
  tests). The event-scan + grouping logic ports to the same `s2`-feeding shape.
- **Ops:** disposable restart → scan → rebuild → verify-vs-chain → resume.
  Monitoring: SYNCED flag, queue depth, last-proof duration, last `fulfillBatch`
  txid + energy, gas balance. Failure modes: divergence ⇒ SYNCED gate refuses;
  stale root ⇒ rebase; reverting recipient ⇒ drop/pull; OOM ⇒ retry (rare on the
  high-RAM box); aggregator can't anchor ⇒ certified + seal-cache; TronGrid limits ⇒
  backoff + multiple endpoints.

## B8. Fees & incentives (deferred; hooks in place)

Self-run and **subsidized** initially: `feeAmount = 0`, operator funds Tron gas (or
**stakes TRX** → ~$0 recurring, refundable; §05). Mechanism already in-protocol:
`BridgeBackReason.{feeRecipient, feeAmount}`, vault enforces `feeAmount ≤ amount`
and pays the fee only if the deadline holds. Later: a `GET /fees` schedule covering
`(267,779/N + 19,347)·Pe·Ptrx + proof_$/N + margin`; permissionless competition +
self-settle keep it honest.

---

## Decisions (resolved at review)

| # | Question | Decision |
|---|---|---|
| Bridge-in finality | wait K before mint? | **No.** Minter trusts its own lock; mints on lock-tx acceptance (`confirmations: 0`). K is enforced by the first independent receiver at transfer-out (§A1.1). |
| 1 | Tron signing backend | **TronLink first** (prettiest demo UX), behind a `TronSigner` interface; iterate to WalletConnect v2 (mobile/custody) then managed-key (fallback) (§A1.3). |
| 2 | Shared `bridge-core` in Sphere? | **No** — all bridge logic (chain-agnostic + Tron) stays in the **plugin package**; Sphere holds zero chain-agnostic code (Option 1 provider passthrough, §A2). |
| 3 | Service shell language | **All Rust** — one binary wrapping the host crate + an all-Rust Tron submitter; retire `relayer.js` (§B, §B7). |
| 4 | Demo asset | **Nile test USDT (with faucet).** Verify its `transfer` return is standard (true/void); the earlier user-supplied "USDT" returned `false` and the vault's safe-transfer rejects it — first task of the contracts/asset track (§07). |
| 5 | Batch window / proving | **Configurable `max_wait`, demo 60 s, larger in prod; strictly sequential single-flight proving queue (no parallel proving)** (§B3, §B5). |
| — | bridge-back proving efficiency | Add **best-effort inclusion replacement + verification-result caching** (seal-keyed quorum dedup, inclusion-path cache; also handles aggregator sharding) (§B6). |

Development plans: [`06-wallet-bridge-integration.md`](./06-wallet-bridge-integration.md)
· [`07-return-service.md`](./07-return-service.md).
