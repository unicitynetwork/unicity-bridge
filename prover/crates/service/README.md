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
4. **S3** runs SP1 → Groth16 (`prove_mode=sp1_groth16`, in a child process that runs
   `bridge-return-host sp1-groth16-files`, so a proof's memory goes back to the OS when it
   ends) or stops at the precheck (`prove_mode=precheck_only`, the default — fast, no
   proof). The published bundle
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

- **Rust**: `cargo build -p bridge-return-service --release`, and the host built with the
  `sp1` feature, since the service runs `bridge-return-host` for the guest key and every proof:
  `cargo build -p bridge-return-host --features sp1 --release`.
- **Go** (for `native-gnark` — gnark runs locally, no Docker).
- **SP1 toolchain + cached circuit artifacts**: the v6.1.0 circuit + the ~5.86 GB
  `groth16_pk.bin` on disk (downloaded once to `~/.sp1`). `SP1_PROVER=cpu`,
  `SP1_CIRCUIT_MODE=release`.
- The **guest ELF** for the deployed vkey: `SP1_GUEST_ELF=<path to bridge-return-sp1-guest>`
  (must match the vault's vkey — currently `0x002b42fa…`).
- Lots of RAM (≥ 64–128 GB to lift `SP1_WORKER_NUM_*` and cut the ~50–60 min
  wall-clock; a single proof saturates the machine, hence the **single-flight**
  queue — never run parallel proofs). Size it for one proof's peak plus the
  service: each proof is a child process and returns its memory when it ends.

```bash
SP1_PROVER=cpu SP1_CIRCUIT_MODE=release \
SP1_GUEST_ELF=/path/to/bridge-return-sp1-guest \
BRIDGE_RETURN_PROVE_MODE=sp1_groth16 \
cargo run -p bridge-return-service --release
```

---

## Configuration (environment)

| Env var | Default | Purpose |
|---|---|---|
| `BRIDGE_RETURN_BIND` | `127.0.0.1:8787` | HTTP bind address. |
| `BRIDGE_DEPLOYMENT_CONFIG` | — | Path to the frozen deployment JSON (`deployments/nile/nile-usdt.json`). **Required.** |
| `TRUST_BASE_PATH` | — | Path to the trust base JSON (`bft-trustbase.testnet2.json`). **Required.** |
| `BRIDGE_JUSTIFICATION_TAG` | `1330002` | Source-chain lock-justification CBOR tag (Tron USDT). |
| `BRIDGE_CONFIG_HASH` | — | If set, the intake cross-checks the envelope's declared `configHash`. |
| `BRIDGE_RETURN_PROVE_MODE` | `precheck_only` | `precheck_only` or `sp1_groth16`. |
| `SP1_GUEST_ELF` | — | Guest ELF path (required for `sp1_groth16`). |
| `BRIDGE_RETURN_PROOF_DIR` | `target/bridge-return-service/proofs` | Where proof bundles are written. |
| `BRIDGE_RETURN_PROVE_CMD` | `bridge-return-host sp1-groth16-files` | The command that proves one batch in a child process; it gets the ELF, wire, proof and info paths as arguments. |
| `BRIDGE_RETURN_VKEY_CMD` | `bridge-return-host sp1-vkey` | The command that prints the guest's verifying key as JSON at start, given the ELF path; the service itself loads no SP1 code. |
| `BRIDGE_RETURN_PROVE_TIMEOUT_SECS` | `7200` | A proof that runs longer is killed and counts as a failed attempt; set it well above the slowest proof, or `0` for no limit. |
| `BRIDGE_RETURN_DRAIN_TIMEOUT_SECS` | `0` | Most a drain (SIGTERM) waits for the batch in flight before abandoning its proof; `0` waits however long it takes. Keep it below the container's stop grace period. |
| `BRIDGE_RETURN_STATE_DIR` | — (in memory) | Journal directory. Unset, every restart forgets the queue. |
| `BRIDGE_RETURN_MAX_BATCH_SIZE` | `8` | Most burns in one proof. |
| `BRIDGE_RETURN_MAX_BATCH_BYTES` | `8388608` | Cap on the summed wire inputs of a batch; a single larger burn still proves alone. |
| `BRIDGE_RETURN_IDLE_WAIT_SECS` | `0` | Collection window before the first proof when the service is idle. |
| `BRIDGE_RETURN_RETRY_BASE_SECS`, `BRIDGE_RETURN_MAX_ATTEMPTS`, `BRIDGE_RETURN_MAX_REBASES` | `60`, `5`, `3` | Retry backoff (doubling from the base), attempts before a return is parked, rebases before a batch fails. |
| `BRIDGE_RETURN_FEE_AMOUNT` | `0` | Fee quoted to wallets, in the asset's smallest unit. `0` settles for free. |
| `BRIDGE_RETURN_FEE_FLOOR` | `0` | Least fee a burn must pay to be accepted. `0` quotes the fee without enforcing it. Must not exceed the amount. |
| `BRIDGE_RETURN_FEE_RECIPIENT` | — | Source-chain account (20-byte hex) the fee is paid to. Required with a fee. |
| `BRIDGE_RETURN_FEE_WINDOW_SECS` | `86400` | Time the service wants between accepting a burn and the burn's fee deadline. |
| `BRIDGE_RETURN_SUBMIT_CMD` | — | S4 submitter command (see below). Unset = `none`. |
| `BRIDGE_RETURN_EVENTS_CMD` | — | Chain log command (`relayer-eth.js events` or `relayer.js events`). Unset assumes a pristine vault. A replacement must name each batch's `txid`: a release the log names no transaction for is never confirmed. |
| `BRIDGE_RETURN_SIMULATE_CMD` | — | Settlement pre-simulation command: lock-ref and transfer checks (see below). **Required.** |
| `BRIDGE_RETURN_CONFIRM_CMD` | — | Confirms, on a second source, a release the chain log shows but the service holds no receipt for (`relayer-eth.js confirm --stdin`, which reads `ETH_CONFIRM_RPC_URL`). Unset, such a return stays `submitted` and is not checked again (see below). |
| `BRIDGE_RETURN_RECEIPT_CMD` | — | Looks up a settlement the relayer broadcast without seeing it settle (`relayer-eth.js receipt --stdin`, through `ETH_RPC_URL`). Unset, only the chain log decides such a settlement (see below). |
| `BRIDGE_RETURN_MAX_PENDING` | `1000` | Most returns held unfinished (queued, proving, awaiting settlement or a retry); beyond it `POST /returns` refuses a new burn with `queue_full` (HTTP 503, recoverable). |
| `RUST_LOG` | — | e.g. `bridge_return_service=info`. |

`BRIDGE_DEPLOYMENT_CONFIG`, `TRUST_BASE_PATH` and `BRIDGE_RETURN_SIMULATE_CMD` are
required: the service refuses to start without them (exit 2). It also refuses to start
when a configured confirmation or receipt command cannot run: each is run once at
startup on nothing to check (for `relayer-eth.js confirm`, a missing
`ETH_CONFIRM_RPC_URL` or one equal to `ETH_RPC_URL` fails it). It admits a burn only through the wallet envelope, which
it verifies against that trust base. A pre-assembled `wireInput` is refused with
`raw_witness_refused` (HTTP 400), since nothing would pin its trust base to the
deployment's. Only the in-process test router, built without an intake, takes one.

---

## API (§B4)

| Method | Path | Purpose |
|---|---|---|
| `POST` | `/returns` | Submit `{tokenCbor, configHash, reasonBytes}`. Idempotent on nullifier; S1 precheck rejects bad burns synchronously. |
| `GET` | `/returns/:id` | Rich status record (below). |
| `GET` | `/returns?nullifier=` | Lookup by nullifier (wallet idempotency); `null` if unknown. |
| `GET` | `/batches/:id` | Published bundle (`vkey`, `publicValues`, `proofBytes`, `leaves`, `lockRefs`, `settleTxid`) — anyone can self-submit it. A service with a submitter publishes it once the batch settles or every burn in it is past its deadline, and answers `not_published` (HTTP 404, recoverable) before; one without serves it as soon as it is proven. |
| `GET` | `/fees` | The fee terms a wallet writes into a burn: `feeRecipient`, `feeAmount` (decimal string) and `deadline` (unix seconds, service clock). A burn paying less than the floor, to another account, or with too little time before its deadline is refused with `fee_not_paid`. |
| `GET` | `/accumulator` | Rebuilt `spentRoot` + SYNCED flag. |
| `GET` | `/health` | `draining`, `queueDepth`, `activeBatch`, `activeBatchSize`, `provingSinceMs`, `lastProofMs`, `averageProofMs`, `unconfirmedReleases`, `unconfirmedSettlements`, `maxBatchSize`, `idleWaitMs`, `proveMode`, `chainSync`. |

All responses are **camelCase JSON**.

### Status semantics
`GET /returns/:id` returns: `status` (`queued → proving → proven → settled`,
`submitted`, or `failed`), `terminal`, `success`, `progress` (0–100), `message`,
`nextPollMs` (poll cadence hint, 0 when terminal), `batchId`, `settleTxid`,
`failure` (`{kind, message, recoverable}`), `attempts`, `notBeforeMs`,
`queuePosition`, and an `events` audit trail. `settleTxid` is set only for a
settlement the service holds a receipt for or a second source has confirmed.
Errors are typed: `{error:{code, message, recoverable}}` with HTTP 400/404/503.
Failure `kind`s: `precheck_rejected`, `proving_failed`, `submission_failed`,
`chain_rejected`, `service_unavailable`.

Every member of a batch shares its `batchId` and settles in one transaction.
`batchSize`, `totalAmount` and `publicValues` on the record describe the burn
as submitted (a batch of one); the batch's own values are on `GET /batches/:id`.
A burn is set aside on its own when its nullifier is already released on chain,
when its recipient would revert the transfer (`failed`, recoverable), or when its
stored input no longer decodes (`failed`, final). Two burns backed by the same
deposit prove in separate batches.

A release the chain log shows but the service did not submit itself (another
party submitted the bundle, or the relayer lost track of its own transaction)
comes from the one RPC the log is read from. It settles the return, with the
transaction the log names as `settleTxid`, only once `BRIDGE_RETURN_CONFIRM_CMD`
confirms that transaction on a second source: it succeeded, is final, and emitted
the vault's `Released` event for the nullifier. Until then the return is
`submitted`, not `terminal`, with no `settleTxid`; its `message` names the
transaction and why it is not confirmed, `/health` counts it in
`unconfirmedReleases`, and it is checked again every 5 minutes, costing no
attempt. A check asks the second source about the transaction the log named; only
when the source answers without confirming it does the service read the chain log
again, to see whether the release still stands (asking about the transaction the log
now names, if that changed). A return whose release the log no longer shows is queued
and proven again. Without `BRIDGE_RETURN_CONFIRM_CMD` nothing could confirm such a
release, so it is not checked again. A proven batch whose every nullifier the log
shows released settles as a batch only when one confirmed transaction released them
all and the log holds the batch's own transition; otherwise (another proof released
the same burns, say) each member settles or waits on its own and the batch is closed
as `chain_rejected`, charging no member.

`recoverable` says whether the same blob may go through later. A precheck
rejection is deterministic for that blob and is never recoverable. A batch-wide
fault (`proving_failed`, `submission_failed`, `chain_rejected`,
`service_unavailable`) marks every member `failed` with `recoverable: true` and
a retry scheduled at `notBeforeMs`, doubling from
`BRIDGE_RETURN_RETRY_BASE_SECS`. While that retry is pending the record is not
`terminal`, `success` is null and `nextPollMs` stays non-zero. The service retries on its own; a settlement retry reuses the proof; after
`BRIDGE_RETURN_MAX_ATTEMPTS` the return is parked as a final, `terminal` `failed`
whose message says so. The chain log is read before proving and again before
settling; when `BRIDGE_RETURN_EVENTS_CMD` fails to run, times out or exits
non-zero, the batch reads it again after 15, 30 and 60 seconds before it fails
as `service_unavailable`, so a single failed RPC request costs no attempt and
does not throw away a finished proof. A log that is read but does not reproduce
the vault's root fails the batch at once as `chain_rejected`. `POST /returns` is
idempotent on the nullifier, with one exception: a recoverably failed return is
queued again by a resubmit (`duplicate: false`) without shortening its schedule.

---

## S4 submitter

Pluggable so the relayer of any chain family (or a future in-process client) drops in
without touching the queue:

- **`none`** (default): the return stays `proven`; the published bundle is
  **self-settleable** by anyone, so the principal is never stuck (06 §A1.2).
- **`command`**: set `BRIDGE_RETURN_SUBMIT_CMD` to a program that receives the
  bundle JSON (`{batchId, mode, vkey, publicValues, proofBytes, leaves, lockRefs}`)
  on **stdin**, submits `fulfillBatch`, and prints the settle **txid** on stdout
  (exit 0). Exit **75** with the txid on stdout means the transaction was
  broadcast but not seen to settle. The service journals `batch_submitted` with
  the txid and waits on that transaction, never sending another for the batch: it
  looks it up through `BRIDGE_RETURN_RECEIPT_CMD` every 30 s for up to 15 minutes,
  starting no other batch meanwhile, and then every 5 minutes while other batches
  run. Settled, the batch settles with that txid. Reverted, or unknown to the chain
  2 minutes after the broadcast, it will not settle the batch, and the chain log
  decides: the batch settles if every member was released (confirmed as below), is
  re-proven if the vault root moved past it, and only otherwise fails as
  `submission_failed`. A transaction still pending, or one nothing can read, decides
  nothing and costs nothing: the batch waits, its members `proven`, and `/health`
  counts it in `unconfirmedSettlements`. A restart looks the transaction up again
  instead of resubmitting, and a drain does not wait for it. Without a receipt
  command only the chain log decides (released, or overtaken), so a transaction that
  never lands leaves the batch waiting. A non-zero exit
  whose stderr contains `stale root` makes the service re-prove the batch on the
  new root; any other failure schedules a retry that reuses the proof. The
  relayers sign the settlement before sending it, so its txid is known even when
  the send fails without an answer, and send the same transaction again after such
  a failure; only a node refusing it outright fails the submission. They then poll
  the receipt themselves, reading it again after a failed request, and stop 30 s
  before `BRIDGE_RETURN_COMMAND_TIMEOUT_SECS` so the txid still reaches the
  service. This is the seam for the existing
  `contracts/tron/scripts/relayer-eth.js settle` (Ethereum family), `relayer.js settle` (Tron) and for the all-Rust submitter (07 §B7).

Example (wrapping the proven relayer):
```bash
BRIDGE_RETURN_SUBMIT_CMD='node /path/contracts/tron/scripts/relayer-eth.js settle --stdin'
```

### Release confirmation

`BRIDGE_RETURN_CONFIRM_CMD` receives `{"releases": [{"nullifier", "txid"}]}` on
**stdin** and prints `{"confirmed": [{"nullifier", "txid"}], "unconfirmed":
[{"nullifier", "txid", "reason"}]}` on stdout (exit 0; non-zero when it cannot
answer). Only an exact `confirmed` match confirms a release.
`relayer-eth.js confirm --stdin` reads the receipt through `ETH_CONFIRM_RPC_URL`,
which must be a different endpoint from `ETH_RPC_URL`, and confirms a release
whose transaction succeeded, is `ETH_SETTLE_CONFIRMATIONS` deep, and carries the
vault's `Released` event with the nullifier as topic 1. `relayer.js confirm
--stdin` does the same through a Tron node at `TRON_CONFIRM_RPC_URL`, whose
solidity API reports only final transactions. A second RPC may trail the first by a
block or two, so `relayer-eth.js` reads a release that falls short only of depth once
more, 15 s later, before reporting it unconfirmed. Asked about no releases, a command
reads nothing and answers with empty lists, which is how the service checks at startup
that it runs.

### Settlement lookup

`BRIDGE_RETURN_RECEIPT_CMD` receives `{"txids": ["0x…"]}` on **stdin** and prints
`{"settlements": [{"txid", "state", "confirmations"}]}` on stdout (exit 0; non-zero
when it cannot read the chain). `state` is `confirmed` (succeeded, at least
`ETH_SETTLE_CONFIRMATIONS` deep), `reverted` (failed, that deep), `pending` (shallower,
or known to the node but not mined) or `unknown` (the node has neither the receipt nor
the transaction). `relayer-eth.js receipt --stdin` reads through `ETH_RPC_URL`, the
RPC the settlement was sent through; `relayer.js receipt --stdin` reads a Tron node's
solidity API, which reports only final transactions, and its full node for pending ones.

### Settlement pre-simulation

Nothing but the vault knows whether a burn's source locks exist, and the vault
pays every leaf in one transaction, so a recipient whose transfer reverts would
revert the whole batch. `BRIDGE_RETURN_SIMULATE_CMD` (required) is a program
that receives `{"leaves": [...]}` (the bundle's leaf objects, each with the
`lockRefs` it draws on) on **stdin** and prints
`{"rejected": [{"nullifier", "reason", "kind"}]}` on stdout (exit 0; exit 1 when
the node is unreachable). The service runs it before every proof. `kind` is
`lock` for a lock ref that is not the digest the vault stored: that return fails
for good (`chain_rejected`, not recoverable) and never re-enters a batch.
`unconfirmed` (the lock is not yet as deep as the relayer's confirmations) and
`recipient` (the default when `kind` is absent) set the burn aside as a
recoverable failure that is retried. A burn whose stored input no longer decodes
is not simulated and fails for good. `contracts/tron/scripts/relayer-eth.js simulate --stdin`
(and `relayer.js` for Tron) implements it: lock refs are checked for pull-payment
vaults too, `relayer-eth.js` reads them at the block `ETH_SETTLE_CONFIRMATIONS`
makes final (`relayer.js` reads them at the head, as TronWeb constant calls take no
block), and for a push vault the asset's `transfer` from the vault is constant-called
for the principal and for the fee, skipping a payout of zero or to the vault itself,
with a returned `false` counted as a failure on EVM chains (not on Tron, where USDT
returns false on success).

---

## Integration with the wallet (Sphere)

1. Run the service (precheck mode is enough to exercise the wallet flow):
   ```bash
   BRIDGE_DEPLOYMENT_CONFIG=$PWD/deployments/nile/nile-usdt.json \
   TRUST_BASE_PATH=$PWD/bft-trustbase.testnet2.json \
   cargo run -p bridge-return-service
   ```
2. Point the wallet at it: set `VITE_BRIDGE_RETURN_SERVICE_URL_NILE_USDT=http://localhost:8787`
   in `sphere/.env` (`VITE_BRIDGE_RETURN_SERVICE_URL_SEPOLIA_USDC` for a service
   running the Sepolia deployment), or override the manifest's `returnServiceUrl`.
   The plugin's `ReturnServiceClient` is typed to this API.
3. Bridge out in the UI → the wallet POSTs the envelope → tracks
   `queued→proving→…→settled` and shows the settle txid.

The wallet derives the **same** nullifier/leaf the service does — the live e2e
backstop is `packages/bridge-plugin/demo/bridge-back-e2e.ts` (asserts wallet ==
Rust agreement on the burned token).

---

## Ops & disposability (§B7)

SIGTERM drains the service: the batch in flight runs to its end (settled, or
failed as any batch fails), no other starts, and the process exits 0 once
nothing is in flight. Meanwhile `/health` reports `draining: true` and intake
keeps journaling burns for the next start. A second SIGTERM, SIGINT or
`BRIDGE_RETURN_DRAIN_TIMEOUT_SECS` abandons the proof instead: the service kills
it, journals the interruption as orderly, and the batch is proven again at the
next start without costing its members an attempt.

A restart replays `journal.jsonl` from `BRIDGE_RETURN_STATE_DIR`: queued burns
stay queued, a batch the service abandoned to stop is proven again at once, a
batch a crash left proving is re-formed after the retry backoff (that
interruption counts as an attempt, so a service that dies on every proof parks
the return instead of looping), a batch whose settlement was broadcast is looked
up by its transaction and never resubmitted, a proven batch is settled or
resubmitted after a chain check, scheduled retries keep their schedule. The
journal is one JSON object per line (`jq -c 'keys[0]' journal.jsonl` lists the
event types) and is compacted to one snapshot line at start. The **burned blob**
is still the claim, and the wallet still resubmits it on a 404. Monitor:
`/health` (queue depth, the active batch, its size and start, the last proof
duration, `unconfirmedReleases`, `unconfirmedSettlements`), `/accumulator`
(SYNCED), last settle txid. An `unconfirmedReleases` above zero that does not
clear within a few checks means the chain log and the second source disagree
about a payout, or that no second source is configured: check the transaction a
`submitted` return's message names on an explorer. An `unconfirmedSettlements`
above zero for more than a few minutes means a settlement transaction is stuck
unmined or its RPC cannot be read: the members' message names the transaction.
A return waiting on confirmation journals a `return_excluded` with
`already_settled` and an `unconfirmed` reason when it starts waiting and whenever
a check finds something new; a check that finds the same adds nothing. Failure
modes: precheck rejection (typed 400), stale root (re-prove on the new root),
reverting recipient (excluded before proving), OOM (retried with backoff, then
parked).

## Tests

```bash
cargo test -p bridge-return-service   # domain, journal, orchestrator with fakes, API
cargo test -p bridge-return-host      # S1 precheck, EnvelopeIntake, certified build
```
