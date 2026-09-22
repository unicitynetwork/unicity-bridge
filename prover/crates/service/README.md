# bridge-return-service

The all-Rust **return sequencing + proving service** (dev plan
[`07-return-service.md`](../../../docs/dev-plan/07-return-service.md), design
[`integration.md`](../../../docs/dev-plan/integration.md) Part B). It turns a
wallet's bridge-out **burn** into an on-chain **release** of USDT on Tron,
collapsing S1–S4 into one process:

```
 POST /returns ─► S1 intake (precheck) ─► journal + batch policy ─► S3 prove (SP1→Groth16) ─► S4 submit (fulfillBatch) ─► settled
   {tokenCbor,                build the certified      everything pending        one proof for         push USDT to the
    configHash,               GuestInput in-service     when the prover is free   the whole batch       Tron recipients
    reasonBytes}              (verify the burn)         (up to max_batch_size)    (/batches/:id)
```

It is **trustless and disposable**: it holds no user funds or keys (beyond its own
Tron gas account), can't steal (the vault pays only public leaves proven from the
burn reason) or forge (only a valid Groth16 proof advances `spentRoot`), and its
local state is a journal of accepted burns, batches and proofs
(`BRIDGE_RETURN_STATE_DIR`) that a restart replays; every entry is public data
anyone could resubmit.

---

## The loop, end to end

1. The **wallet** (Sphere) burns a bridged token to `BurnPredicate(H(reasonBytes))`
   and POSTs the witness envelope `{tokenCbor, configHash, reasonBytes}` to
   `POST /returns`.
2. **S1 intake** builds the certified `GuestInput` in-service: it decodes the
   burned token, **fully verifies it** against the trust base, derives the
   nullifier from the terminal burn (00 §5) and the settlement leaf from
   `reasonBytes` (00 §4), then runs the off-chain **precheck** (the exact guest
   relation) — rejecting bad burns synchronously with a typed error.
3. The return is journaled and waits for the prover. When the prover is free,
   the next batch is everything pending, oldest first, up to
   `BRIDGE_RETURN_MAX_BATCH_SIZE` burns that share one trust base and trace to
   distinct deposits; burns arriving during a proof form the next batch. The
   members' inputs are merged into one `GuestInput` chained onto the vault's
   current `spentRoot`.
4. **S3** runs SP1 → Groth16 (`prove_mode=sp1_groth16`) or stops at the precheck
   (`prove_mode=precheck_only`, the default — fast, no proof). The published bundle
   (`vkey`, `publicValues`, `proofBytes`) is exposed at `GET /batches/:id`.
5. **S4** submits `fulfillBatch` to the vault via the configured submitter, then
   every return in the batch reads `settled` with the settle txid. With **no**
   submitter the returns stay `proven` and the bundle is **self-settleable** by
   anyone.

The wallet tracks progress by polling `GET /returns/:id` (rich status, below) and,
independently, by watching the vault's `Released{nullifier}` over Tron RPC.

---

## Install / prerequisites

### Precheck mode (default — no proving)
Just Rust. This validates burns, sequences batches, and exercises the full API and
status machine without the heavy STARK→Groth16 step. Ideal for wallet integration
testing.

```bash
cargo run -p bridge-return-service
```

### SP1 Groth16 mode (real proofs)
Reuses the proven CPU-only path (see [`../../../docs/dev-plan/03-status.md`](../../../docs/dev-plan/03-status.md)
and the `sp1-real-proving` notes). Prerequisites:

- **Rust** + the `sp1` feature: `cargo build -p bridge-return-service --features sp1 --release`.
- **Go** (for `native-gnark` — gnark runs locally, no Docker).
- **SP1 toolchain + cached circuit artifacts**: the v6.1.0 circuit + the ~5.86 GB
  `groth16_pk.bin` on disk (downloaded once to `~/.sp1`). `SP1_PROVER=cpu`,
  `SP1_CIRCUIT_MODE=release`.
- The **guest ELF** for the deployed vkey: `SP1_GUEST_ELF=<path to bridge-return-sp1-guest>`
  (must match the vault's vkey — currently `0x002b42fa…`).
- Lots of RAM (≥ 64–128 GB to lift `SP1_WORKER_NUM_*` and cut the ~50–60 min
  wall-clock; a single proof saturates the machine, hence the **single-flight**
  queue — never run parallel proofs).

```bash
SP1_PROVER=cpu SP1_CIRCUIT_MODE=release \
SP1_GUEST_ELF=/path/to/bridge-return-sp1-guest \
BRIDGE_RETURN_PROVE_MODE=sp1_groth16 \
cargo run -p bridge-return-service --features sp1 --release
```

---

## Configuration (environment)

| Env var | Default | Purpose |
|---|---|---|
| `BRIDGE_RETURN_BIND` | `127.0.0.1:8787` | HTTP bind address. |
| `BRIDGE_DEPLOYMENT_CONFIG` | — | Path to the frozen deployment JSON (`deployments/nile/nile-usdt.json`). **Required for envelope intake.** |
| `TRUST_BASE_PATH` | — | Path to the trust base JSON (`bft-trustbase.testnet2.json`). **Required for envelope intake.** |
| `BRIDGE_JUSTIFICATION_TAG` | `1330002` | Source-chain lock-justification CBOR tag (Tron USDT). |
| `BRIDGE_CONFIG_HASH` | — | If set, the intake cross-checks the envelope's declared `configHash`. |
| `BRIDGE_RETURN_PROVE_MODE` | `precheck_only` | `precheck_only` or `sp1_groth16`. |
| `SP1_GUEST_ELF` | — | Guest ELF path (required for `sp1_groth16`). |
| `BRIDGE_RETURN_PROOF_DIR` | `target/bridge-return-service/proofs` | Where proof bundles are written. |
| `BRIDGE_RETURN_STATE_DIR` | — (in memory) | Journal directory. Unset, every restart forgets the queue. |
| `BRIDGE_RETURN_MAX_BATCH_SIZE` | `8` | Most burns in one proof. |
| `BRIDGE_RETURN_MAX_BATCH_BYTES` | `8388608` | Cap on the summed wire inputs of a batch; a single larger burn still proves alone. |
| `BRIDGE_RETURN_IDLE_WAIT_SECS` | `0` | Collection window before the first proof when the service is idle. |
| `BRIDGE_RETURN_RETRY_BASE_SECS`, `BRIDGE_RETURN_MAX_ATTEMPTS`, `BRIDGE_RETURN_MAX_REBASES` | `60`, `5`, `3` | Retry backoff (doubling from the base), attempts before a return is parked, rebases before a batch fails. |
| `BRIDGE_RETURN_SUBMIT_CMD` | — | S4 submitter command (see below). Unset = `none`. |
| `BRIDGE_RETURN_EVENTS_CMD` | — | Chain log command (`relayer.js events`). Unset assumes a pristine vault. |
| `BRIDGE_RETURN_SIMULATE_CMD` | — | Transfer pre-simulation command (see below). Unset = no simulation. |
| `RUST_LOG` | — | e.g. `bridge_return_service=info`. |

Without `BRIDGE_DEPLOYMENT_CONFIG` + `TRUST_BASE_PATH` the service runs in
**wireInput-only** mode (accepts a pre-assembled guest input; used by fixtures /
relayers) and rejects the wallet `{tokenCbor, reasonBytes}` envelope with
`intake_unconfigured`.

---

## API (§B4)

| Method | Path | Purpose |
|---|---|---|
| `POST` | `/returns` | Submit `{tokenCbor, configHash, reasonBytes}` (or `{wireInput}`). Idempotent on nullifier; S1 precheck rejects bad burns synchronously. |
| `GET` | `/returns/:id` | Rich status record (below). |
| `GET` | `/returns?nullifier=` | Lookup by nullifier (wallet idempotency); `null` if unknown. |
| `GET` | `/batches/:id` | Published bundle (`vkey`, `publicValues`, `proofBytes`, `settleTxid`) — anyone can self-submit it. |
| `GET` | `/accumulator` | Rebuilt `spentRoot` + SYNCED flag. |
| `GET` | `/health` | `queueDepth`, `activeBatch`, `activeBatchSize`, `provingSinceMs`, `lastProofMs`, `averageProofMs`, `maxBatchSize`, `idleWaitMs`, `proveMode`, `chainSync`. |

All responses are **camelCase JSON**.

### Status semantics
`GET /returns/:id` returns: `status` (`queued → proving → proven → settled`, or
`failed`), `terminal`, `success`, `progress` (0–100), `message`, `nextPollMs`
(poll cadence hint, 0 when terminal), `batchId`, `settleTxid`, `failure`
(`{kind, message, recoverable}`), `attempts`, `notBeforeMs`, `queuePosition`,
and an `events` audit trail. `submitted` stays in the enum for older clients
but is no longer emitted: the submitter returns only once the receipt is in.
Errors are typed: `{error:{code, message, recoverable}}` with HTTP 400/404.
Failure `kind`s: `precheck_rejected`, `proving_failed`, `submission_failed`,
`chain_rejected`, `service_unavailable`.

Every member of a batch shares its `batchId` and settles in one transaction.
`batchSize`, `totalAmount` and `publicValues` on the record describe the burn
as submitted (a batch of one); the batch's own values are on `GET /batches/:id`.
A burn is set aside on its own when its nullifier is already released on chain
(it becomes `settled` without a txid), when its recipient would revert the
transfer (`failed`, recoverable), or when its stored input no longer decodes
(`failed`, final). Two burns backed by the same deposit prove in separate
batches.

`recoverable` says whether the same blob may go through later. A precheck
rejection is deterministic for that blob and is never recoverable. A batch-wide
fault (`proving_failed`, `submission_failed`, `chain_rejected`) marks every
member `failed` with `recoverable: true` and a retry scheduled at `notBeforeMs`,
doubling from `BRIDGE_RETURN_RETRY_BASE_SECS`. The service retries on its own; a
settlement retry reuses the proof; after `BRIDGE_RETURN_MAX_ATTEMPTS` the return
is parked as a final `failed` whose message says so. `POST /returns` is
idempotent on the nullifier, with one exception: a recoverably failed return is
queued again by a resubmit (`duplicate: false`) without shortening its schedule.

---

## S4 submitter

Pluggable so the proven relayer (or a future in-process Tron client) drops in
without touching the queue:

- **`none`** (default): the return stays `proven`; the published bundle is
  **self-settleable** by anyone, so the principal is never stuck (06 §A1.2).
- **`command`**: set `BRIDGE_RETURN_SUBMIT_CMD` to a program that receives the
  bundle JSON (`{batchId, mode, vkey, publicValues, proofBytes, leaves, lockRefs}`)
  on **stdin**, submits `fulfillBatch`, and prints the settle **txid** on stdout
  (exit 0). A non-zero exit whose stderr contains `stale root` makes the service
  re-prove the batch on the new root; any other failure schedules a retry that
  reuses the proof. This is the seam for the existing
  `contracts/tron/scripts/relayer.js settle` and for the all-Rust submitter (07 §B7).

Example (wrapping the proven relayer):
```bash
BRIDGE_RETURN_SUBMIT_CMD='node /path/contracts/tron/scripts/relayer.js settle --stdin'
```

### Transfer pre-simulation

The vault pays every leaf in one transaction, so a recipient whose transfer
reverts would revert the whole batch. Set `BRIDGE_RETURN_SIMULATE_CMD` to a
program that receives `{"leaves": [...]}` (the bundle's leaf objects) on
**stdin** and prints `{"rejected": [{"nullifier", "reason"}]}` on stdout (exit 0;
exit 1 when the node is unreachable). The service runs it before every proof
and sets rejected burns aside as recoverable failures. Unset, nothing is
simulated. `contracts/tron/scripts/relayer.js simulate --stdin` implements it
with a constant call of the asset's `transfer` from the vault.

---

## Integration with the wallet (Sphere)

1. Run the service (precheck mode is enough to exercise the wallet flow):
   ```bash
   BRIDGE_DEPLOYMENT_CONFIG=$PWD/deployments/nile/nile-usdt.json \
   TRUST_BASE_PATH=$PWD/bft-trustbase.testnet2.json \
   cargo run -p bridge-return-service
   ```
2. Point the wallet at it: set `VITE_BRIDGE_RETURN_SERVICE_URL=http://localhost:8787`
   in `sphere/.env` (or override the manifest's `returnServiceUrl`). The plugin's
   `ReturnServiceClient` is typed to this API.
3. Bridge out in the UI → the wallet POSTs the envelope → tracks
   `queued→proving→…→settled` and shows the settle txid.

The wallet derives the **same** nullifier/leaf the service does — the live e2e
backstop is `packages/bridge-plugin-tron-usdt/demo/bridge-back-e2e.ts` (asserts wallet ==
Rust agreement on the burned token).

---

## Ops & disposability (§B7)

A restart replays `journal.jsonl` from `BRIDGE_RETURN_STATE_DIR`: queued burns
stay queued, a batch that was proving is re-formed after the retry backoff (an
interruption counts as an attempt, so a service that dies on every proof parks
the return instead of looping), a proven batch is settled or resubmitted after
a chain check, scheduled retries keep their schedule. The
journal is one JSON object per line (`jq -c 'keys[0]' journal.jsonl` lists the
event types) and is compacted to one snapshot line at start. The **burned blob**
is still the claim, and the wallet still resubmits it on a 404. Monitor:
`/health` (queue depth, the active batch, its size and start, the last proof
duration), `/accumulator` (SYNCED), last settle txid. Failure modes: precheck
rejection (typed 400), stale root (re-prove on the new root), reverting
recipient (excluded before proving), OOM (retried with backoff, then parked).

## Tests

```bash
cargo test -p bridge-return-service   # domain, journal, orchestrator with fakes, API
cargo test -p bridge-return-host      # S1 precheck, EnvelopeIntake, certified build
```
