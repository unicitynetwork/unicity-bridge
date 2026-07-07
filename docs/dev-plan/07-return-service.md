# 07 — Return sequencing & proving service development plan

**Stack:** Rust — one binary `bridge-return-service` in the `prover/` workspace,
wrapping the host crate (`s1`/`s2`/`sp1`) plus an all-Rust Tron submitter. `axum`
(HTTP), `tokio` (runtime + the proving queue), `reqwest`+`k256`+keccak (Tron),
`sled`/JSON (optional cache). Design: [`integration.md`](./integration.md) Part B.
Conforms to [`00-interop-contract.md`](./00-interop-contract.md) §1, §4–9 and
reuses [`03-prover-service.md`](./03-prover-service.md).

**Owns:** the off-chain S1–S4 pipeline as one **self-hosted, trustless, disposable**
process for a **high-RAM server (no Docker, no GPU, no proving network)**, with
**strictly sequential proving** (decision #5) and **all-Rust** submission
(decision #3).

**Reuses (already built):** `s1::{precheck,precheck_wire,WitnessPackage,build_certified_guest_input,verify_certified_burn,aggregator}`,
`s2::{rebuild,next_batch,SettledBatch}`, `sp1::{real_groth16,export_onchain,program_vkey}`,
the depth-256 SMT accumulator, the certified-mode guest relation, `relayer-lib.js`
(reference oracle for the Rust event-scan tests).

---

## New crate layout (`prover/crates/`)

```
service/                       # the bridge-return-service binary (new)
  src/main.rs                  # config, axum router, spawn workers
  src/api.rs                   # POST /returns, GET /returns/:id, /accumulator, /health, /batches/:id
  src/queue.rs                 # the single-flight batch queue + max-wait window (decision #5)
  src/sequencer.rs             # scan → s2::rebuild (SYNCED gate) → s2::next_batch
  src/prover.rs               # wraps sp1::real_groth16 + export (sequential)
  src/store.rs                 # status store (public-only, rebuildable)
tron/                          # all-Rust Tron submitter (new; retires relayer.js)
  src/client.rs                # TronGrid HTTP: trigger*contract, broadcast, events, receipts
  src/tx.rs                    # build + k256-sign fulfillBatch; dry-run pre-simulation
sdk-ext/  (extend)             # verification-result cache (seal-keyed + inclusion-path)
host/     (extend)             # multi-burn certified GuestInput; best-effort anchor replace
```

---

## Phases

### R0 — Service skeleton

- `service` crate: config (gateway/TronGrid URLs, vault, `configHash`, trust-base
  path, `max_wait`, `batch_target`, gas key), `axum` router, `tokio` workers, an
  in-memory `store` (status by `returnId`/`nullifier`).
- `GET /health`, `GET /accumulator` (calls the scan+rebuild path), `POST /returns`
  (precheck-stub → enqueue), `GET /returns/:id`.
- Wallet progress contract: `POST /returns` returns the polling identity and first
  progress fields; `GET /returns/:id` returns `status`, `terminal`, `success`,
  `progress`, `message`, `next_poll_ms`, `failure`, and an ordered `events[]` log.
  Synchronous intake failures return machine-readable `error.code` values, so the
  wallet can surface malformed/unbacked/stale burns immediately instead of waiting
  for the queue.

**Exit:** the binary boots, serves `/health`, accepts a `/returns` POST whose burned
blob passes `s1::precheck`, and reports `queued`.

### R1 — Intake + S1 precheck + multi-burn certified assembly

- `POST /returns`: decode the burned blob, run `s1::WitnessPackage::precheck` /
  `precheck_wire`, **reject** malformed/unbacked/stale-predicate burns
  synchronously; idempotent on `nullifier`; persist status.
- Progress semantics are stable for wallet integration:
  `queued(20) → proving(45) → proven(70) → submitted(85) → settled(100)`, or
  `failed(100)` with `{kind,message,recoverable}`. Rebase/retry appends an event
  and moves the record back to the earliest still-valid stage; the wallet should
  follow `nextPollMs` and independently watch `Released{nullifier}`.
- **S1 extension (host):** generalize `build_certified_guest_input` from **B=1 to
  multi-burn** (assemble several certified burns into one `GuestInput`) — the
  load-bearing gap for live batching.
- Fetch inclusion proofs via `s1::aggregator` (`http` feature, already verified live).

**Tests:** accept B=1/split/B=2 certified inputs; reject tampered public-values /
truncated wire / unknown config / replayed nullifier.
**Exit:** a real `e2e:back`-style burned blob is accepted, prechecked, and a
multi-burn certified `GuestInput` assembles + prechecks.

### R2 — Sequencer (S2) + the sequential queue (decision #5)

- **Event watcher:** Rust TronGrid client (`tron/client.rs`) fetches
  `BatchFulfilled`/`Released`, grouped into the `s2` `events.json` shape (port the
  `relayer-lib.js` grouping; keep the JS as a test oracle).
- `s2::rebuild` → **verify rebuilt root == on-chain `spentRoot`** (SYNCED gate;
  refuse to build on divergence). `s2::next_batch` → order-coupled witnesses +
  `spent_root_old/new`.
- **Queue (`queue.rs`):** a `tokio` task draining the precheck-passed set into
  batches. **Single-flight** (one batch proving at a time — no parallel proving).
  Close a batch when `len == batch_target` **or** `max_wait` elapses (demo default
  **60 s**). Rebase on `"stale root"`.

**Tests:** multi-batch rebuild matches chain (≥2-element trees); next-batch
witnesses fold to `spent_root_new`; tampered/out-of-order/double-spend logs
rejected (mirror `s2_rebuild.rs`); queue closes on count and on timeout; rebase
re-forms a batch after an external `spentRoot` advance.
**Exit:** from queued burns, the service forms a batch on the live vault's current
root and produces a prove-ready `GuestInput`; `/accumulator` reports SYNCED.

### R3 — Prover (S3), sequential, CPU/native-gnark

- `prover.rs` wraps `sp1::real_groth16` then `sp1::export_onchain` to the on-chain
  bundle (`vkey`, `publicValues`, `proofBytes`); status `proving → proven`.
- The proven no-Docker/no-GPU/no-network env (high-RAM tuning lifts the 16 GB
  single-worker floor *within* one proof; never concurrent proofs):

  ```bash
  SP1_PROVER=cpu SP1_CIRCUIT_MODE=release \
  SP1_WORKER_NUM_CORE_WORKERS=<cores> ... \
  cargo run --release -p bridge-return-service --features sp1
  ```

- Cache the v6.1.0 circuit + 5.86 GB `groth16_pk.bin` on disk (verify the SHA-256
  from `03-status.md`).

**Exit:** a queued B=1 batch produces a real Groth16 bundle end-to-end inside the
service (matches a hand-run `sp1-groth16` for the same wire).

### R4 — All-Rust submitter (S4) + retire relayer.js

- `tron/tx.rs`: build the `fulfillBatch` tx (lock-seed + leaves + lock-refs +
  proof), **dry-run via `triggerconstantcontract`** (drop reverting recipients in
  push mode; pull mode credits `owed[]`), **k256-sign**, broadcast, poll the
  receipt + `Released` events. Status `submitted → settled`.
- Rebase on `"stale root"` (re-scan → rebuild → re-prove).
- Remove the Node `relayer.js` from the live path (keep `relayer-lib.js` only as a
  grouping oracle in tests).

**Tests:** dry-run drops a `MockBlocklistTRC20` recipient (push) and credits it
(pull); tx build/sign round-trips; stale-root triggers a rebase.
**Exit (live):** the full loop **scan → next_batch → sp1-groth16 → fulfillBatch**
settles a burn on Nile **entirely from the Rust service** (replacing the
relayer.js-driven live settle already demonstrated).

### R5 — Verification-result caching + best-effort inclusion replacement

(The one new protocol feature — `integration.md` §B6.)

- **`sdk-ext`:** a content-addressed verification cache consulted by the quorum +
  inclusion verifiers (host **and** guest):
  - **seal-keyed quorum cache** — verify each distinct **unicity seal** (signature
    set) once, even across distinct UC objects / **aggregator shards** ("same seal,
    different UC");
  - **inclusion-path cache** — `(root, leaf-key, path)` verified once.
- **S1 best-effort anchor replacement:** where the aggregator can serve a shared
  later root, replace per-transition certs with anchored inclusion against it
  (fewer distinct seals); else keep certified. Degrades cleanly.
- Generalizes the §11 byte-identical-`UC*` dedup to **one quorum per distinct seal**.

**Tests:** adversarial like `b2_guest.rs` — sharded/duplicate seals verify once;
distinct seals each verify; a tampered seal still rejects. **Measure** cycle /
quorum-count saving in `sp1-execute` (as the §11 dedup was measured).
**Exit:** a multi-transition certified batch (and a simulated 2-shard set) pays one
quorum per distinct seal; in-circuit public values unchanged; saving reported.

### R6 — Ops, asset, fees

- **Disposable recovery:** restart → scan → rebuild → verify-vs-chain → resume; no
  backup needed. Document the runbook.
- **Asset (decision #4):** integrate the **Nile test USDT (which has online faucet available)**. **First task:
  probe its `transfer` return** — if it returns `false` (like the earlier
  user-supplied "USDT"), the vault's safe-transfer reverts; either pick a
  standard/true-or-void-returning token or add a vault transfer-compat shim
  (coordinate with `01`). Freeze the chosen asset into `bridges.<network>.json` +
  `bridge-vectors/deployment`.
- **Monitoring:** SYNCED flag, queue depth, last-proof duration, last `fulfillBatch`
  txid + energy, gas balance (`/health`).
- **Fees (deferred):** `feeAmount = 0` subsidized; wire the `BridgeBackReason` fee
  fields through; `GET /fees` schedule + `feeRecipient` later (§B8).

**Exit:** a fresh box recovers to SYNCED purely from chain; a live round trip uses
the real Nile faucet USDT; metrics exposed.

---

## Testing & CI

- Reuse the prover suite (`b1_guest.rs`, `b2_guest.rs`, `s1_*`, `s2_rebuild.rs`,
  `accumulator_nonmembership.rs`, `nile_config.rs`) and the cross-stack
  `check-vectors`.
- New: service integration tests against a stub gateway + a mock TronGrid; the queue
  timing/single-flight invariant; the Rust Tron tx build/sign vs `relayer-lib.js`
  grouping; the verification-cache adversarial + measurement tests.
- **Differential:** SP1 `execute` public values == S1 host precheck before any
  proof (existing invariant), now also through the service path.

## Risks

| Risk | Mitigation |
|---|---|
| Parallel proving OOM/thrash | strictly single-flight queue (decision #5); RAM spent vertically within one proof |
| Local Groth16 OOM (16 GB baseline) | high-RAM box lifts worker caps; retry on rare OOM |
| Aggregator can't serve anchored inclusion | certified mode + **seal-keyed cache** (§R5) recovers the quorum saving without the missing endpoint |
| Multi-burn certified assembly is B=1 today | explicit R1 deliverable |
| Nile faucet USDT non-standard (`false` return) | probe first (R6); standard token or vault compat shim (`01`) |
| Accumulator divergence / out-of-order events | SYNCED gate refuses to build (proven live) |
| Stale root mid-flight | rebase (re-scan/rebuild/re-prove) |
| Tron submission in Rust (no TronWeb) | `reqwest`+`k256`+keccak (SDK already pulls them); `relayer-lib.js` kept as a test oracle |
| Disposable box loses pending burns | the burned blob is the wallet's recovery material and resubmittable by anyone (ZK_BACK3 §13) |
