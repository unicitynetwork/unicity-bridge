# Batched proving for the bridge return service

Brief for a fresh-context agent. Its job: produce an implementation plan for
batching in `bridge-return-service`, then implement it test-first. Everything
the plan needs is here or pointed to from here; nothing in this file has to be
rediscovered. Written 2026-09-22 against `feat/sphere-plugin` at `48a5404` plus
the uncommitted changes listed in §16.

## 1. Goal

Today the service proves one burn per proof, one proof at a time, about an hour
each on CPU (§4.4). With several users the returns form a serial queue: three
burns, three hours. The yellowpaper's stage 2, "in-circuit batch", is one proof
for many burns. Guest and vault already support it (§4). The service does not.

Target behaviour, as decided with the owner:

- Burns keep arriving while a proof is being produced. When the proof finishes,
  the next proof starts at once over everything that accumulated meanwhile.
  Idle service: one burn, one proof, no waiting. Busy service: batches grow to
  whatever arrived during the previous proof. Worst-case latency is two proof
  times. No fixed collection window is needed, though one may stay as an option.
- The queue is persistent. A restart resumes: accepted burns are not lost,
  proven-but-unsettled batches are settled, interrupted proofs are re-run.
- Wallets stay agnostic. The HTTP contract in §3.6 does not change in any way
  that breaks the current wallet; additions are allowed. Each wallet still sees
  its own return's progress.
- One failure never blocks the rest. A burn that cannot be batched, proven or
  settled is set aside with a reason; the others proceed.
- The batching logic is unit-testable without a prover, a chain or a clock.
  Proof generation, settlement and chain sync are behind interfaces with fakes.
- Properly engineered and abstracted, per the repo's CLAUDE.md (§14): small
  domain types with the logic on them, thin adapters, no comments.

## 2. Where things are

All paths relative to the repository root `unicity-bridge/`.

```
prover/Cargo.toml                    workspace: core, guest, host, service, sdk-ext (feature `sp1` on host)
prover/crates/core/src/lib.rs        BridgeConfig, PublicValues, ReturnLeaf, SourceLockRef, config_hash, lock_digest, relation checks
prover/crates/guest/src/lib.rs       GuestInput / RelationWitness / BridgeBurnWitness / BurnVerification; the proving relation
prover/crates/guest/src/wire.rs      encode_guest_input / decode_guest_input (the bytes the prover consumes)
prover/crates/host/src/s1.rs         precheck_wire, verify_certified_burn, build_certified_guest_input[_batch|_from_envelope], EnvelopeIntake
prover/crates/host/src/s2.rs         SettledLog, rebuild / rebuild_verified, NextBatch, next_batch (accumulator over settled events)
prover/crates/host/src/sp1.rs        real_groth16 (feature `sp1`), program_vkey, export_onchain
prover/crates/host/src/fixture.rs    build_b1_direct_bridge_fixture, build_b2_direct_bridge_fixture, build_b2_shared_anchor_fixture,
                                     build_split_bridge_fixture, build_settlement_fixture[_continued|_b2]
prover/crates/sdk-ext/src/           accumulator.rs (nullifier tree, witnesses), verify.rs (anchored/certified token verification), trust.rs
prover/crates/service/src/main.rs    wiring: intake, store, submitter, chain events, queue, router
prover/crates/service/src/api.rs     POST /returns, GET /returns/:id, GET /returns?nullifier=, GET /batches/:id, /accumulator, /health; ApiError
prover/crates/service/src/store.rs   ReturnStore (in-memory), ReturnRecord, ReturnStatus, ReturnFailure, ErrorKind, BatchBundle, LeafHex, LockRefHex
prover/crates/service/src/queue.rs   QueueHandle, spawn (the loop), prove_single_flight, sync_and_patch, submit_batch, batch_id
prover/crates/service/src/prover.rs  Prover { prove(batch_id, wire_input) -> ProofBundle }, ProveMode
prover/crates/service/src/submitter.rs  Submitter (Backend::None | Backend::Command), SubmitOutcome, run_command
prover/crates/service/src/sequencer.rs  ChainEvents (none | command), synced_accumulator, rebuild_accumulator
prover/crates/service/src/config.rs  ServiceConfig from env (§10)
prover/crates/service/tests/service_api.rs  API tests over fixtures with Submitter::none(), ChainEvents::none()
prover/crates/service/README.md      API and status semantics; update it
prover/docker/entrypoint.sh, prover/Dockerfile, docker-compose.yml   the container; volumes sp1-artifacts:/root/.sp1, return-data:/data
contracts/tron/contracts/UnicityBridgeVault.sol   fulfillBatch (§4.3)
contracts/tron/scripts/relayer.js    `events` (chain log for s2) and `settle --stdin` (S4), the default commands the service shells out to
packages/bridge-plugin/src/wallet/return-client.ts   the wallet's client: statuses, ReturnRecord, typed errors
sphere/src/modules/bridge/bridgeOut.ts, store.ts, useBridgeOut.ts   the wallet's return records, polling, resubmission, retry
docs/spec/ZK_BACK3.md                the return-path spec: §7 relation layers, §9 batch atomicity, §10 off-chain roles
docs/dev-plan/07-return-service.md   service design; R1 "multi-burn certified assembly", R2 sequencer/queue
docs/OPERATIONS.md                   how the service is run and watched; §8 failure modes; update it
unicity-yellowpaper-tex/appendix-bridging.tex   "Batched Proving" section; B_max default 64; the three stages
deployments/nile/nile-usdt-v2.json   the live v2 deployment (config hash 0xfa77a13a…, vkey 0x0039a542…)
```

## 3. How the return path works today

### 3.1 Intake (`api.rs::create_return`)

The wallet posts `{tokenCbor, reasonBytes, configHash}`. `build_wire_input`
hands the envelope to `EnvelopeIntake::build_wire_input` (`s1.rs`), which
decodes the token, derives the settlement leaf from the terminal burn and the
reason bytes (nullifier, recipient, amount, fee, deadline), verifies the token
against the trust base, and assembles a B=1 `GuestInput` against an empty
accumulator (`spent_root_old = 0`). The bytes are `precheck_wire`d, decoded
again to read the nullifier, and a `ReturnRecord` is created with status
`queued`, holding the wire input in memory (`wire_input: Vec<u8>`, not
serialized). Idempotent on nullifier via `ReturnStore::insert_or_requeue`: a
known nullifier returns the existing record with `duplicate: true`, except a
record that `failed` with `recoverable: true`, which is replaced and queued
again. Tests and relayers may post a pre-assembled `wireInput` instead of the
envelope; intake then requires exactly one return leaf (`unsupported_batch_shape`).

### 3.2 The queue (`queue.rs::spawn`)

One tokio task. It waits for the first enqueue, then collects ids until
`pending.len() >= batch_target` or `max_wait` elapses, pops up to
`batch_target` ids, and awaits `prove_single_flight` on them. Nothing else runs
in that task while it proves; enqueues during a proof land in `pending`
through the channel and are picked up on the next iteration. `active_batch`
and `pending.len()` feed `/health`.

### 3.3 Proving a "batch" (`queue.rs::prove_single_flight`)

`batch_id` is a hash of the ids. Every id is set to `proving`. Then, **for each
id separately**: `sync_and_patch` rebases that record's own wire input onto
the vault's current root, the prover runs on that input alone, the resulting
bundle is stored under the shared batch id, the record goes `proven`, and
`submit_batch` settles that one bundle. So a "batch" of N today is N proofs
and N settlements executed back to back under one id.

### 3.4 Accumulator sync (`queue.rs::sync_and_patch`, `sequencer.rs`, `s2.rs`)

`ChainEvents::synced_accumulator` runs `BRIDGE_RETURN_EVENTS_CMD` (the Node
relayer's `events`, which scans `BatchFulfilled` and `Released` on the vault),
parses the settled log, rebuilds the nullifier tree and checks it against the
live `spentRoot`. `s2::next_batch(acc, nullifiers)` produces `spent_root_old`,
one ordered non-membership witness per nullifier against the running root, and
`spent_root_new`; it rejects the whole call if any nullifier is already in the
tree. The patched input is `precheck_wire`d again before proving. Failures here
become `failed` with kind `chain_rejected`, recoverable.

### 3.5 Prover and settlement

`Prover::prove` (`prover.rs`) is a concrete struct switching on `ProveMode`:
`PrecheckOnly` re-runs `precheck_wire` and returns a bundle with empty proof
bytes; `Sp1Groth16` calls `bridge_return_host::sp1::real_groth16` in
`spawn_blocking`, writing the proof under `BRIDGE_RETURN_PROOF_DIR`.
`Submitter::submit` (`submitter.rs`) pipes the `BatchBundle` JSON to
`BRIDGE_RETURN_SUBMIT_CMD` (`relayer.js settle --stdin`), which builds and
sends `fulfillBatch`; a bundle without proof bytes is refused before spawning.
Outcomes: `Submitted{txid}` → `submitted` then `settled`; `Failed` → `failed`
with kind `submission_failed`, recoverable; `Skipped` (no submitter) → stays
`proven`, the bundle is self-settleable from `GET /batches/:id`.

### 3.6 The wallet contract (must not break)

`POST /returns` → `{returnId, nullifier, status, terminal, success, progress,
message, nextPollMs, duplicate}`. `GET /returns/:id` → `ReturnRecord`:
`returnId, nullifier, status, terminal, success, progress, message, nextPollMs,
batchId, settleTxid, failure{kind,message,recoverable}, events[], batchSize,
totalAmount, publicValuesDigest, publicValues, createdAtMs, updatedAtMs`.
Statuses: `queued(20) → proving(45) → proven(70) → submitted(85) → settled(100)`
or `failed(100)`. Errors: `{error:{code,message,recoverable}}`;
`precheck_rejected` is final, `queue_closed`, `host_error`, `chain_unsynced`
are recoverable. `GET /returns?nullifier=` looks a record up; `GET /batches/:id`
returns the bundle. The wallet (`bridgeOut.ts`) polls every 8 s while a return
is not terminal, resubmits the blob on a 404, resubmits a recoverably failed
return after 60 s or at once from a retry button, and treats
`failed && !recoverable` as final and dismissable. `returnId` is derived from
`(public_values_digest, nullifier)` of the B=1 input; if the derivation changes
the wallet does not care, it stores whatever `POST` returned.

### 3.7 State that is durable today

Nothing. `ReturnStore` is two `HashMap`s behind an `RwLock`. Proof bundles are
written as files under `/data/proofs` (volume `return-data`). A restart
forgets every return and batch; wallets re-post their blobs on the next poll.

## 4. What already exists for batching

### 4.1 The guest relation takes a batch

`GuestInput { config, public_values, return_leaves: Vec<ReturnLeaf>,
sorted_lock_refs: Vec<SourceLockRef>, witness: RelationWitness {
accumulator_witnesses: Vec<NonMembershipWitness>, bridge_burns:
Vec<BridgeBurnWitness> } }`. `public_values.batch_size` is a `u32`;
`return_root` and `total_amount` are over all leaves; `lock_ref_root` over
the sorted refs. Each `BridgeBurnWitness` carries its token, its trust base
and a `BurnVerification`: `Certified` (every transition carries its own
Unicity certificate, what a live aggregator serves; one BFT quorum check per
transition) or `Anchored(UC*)` (one shared certificate; the guest verifies a
distinct anchor once and reuses its root). Live tokens are `Certified`;
`Anchored` waits for the aggregator to serve historical proofs against a
shared root and is out of scope here. The relation (`guest/src/lib.rs`)
checks witnesses in leaf order against the running root, requires lock refs
sorted strictly ascending by nonce (`DuplicateLockRefNonce`,
`LockRefsNotSorted` in `core`), and rejects empty batches.

### 4.2 The host can assemble a certified multi-burn input

`s1::build_certified_guest_input_batch(config, Vec<CertifiedBurnInput{token,
trust_base, lock_justification_tag, leaf}>) -> GuestInput` verifies every
token, concatenates their lock refs and sorts by nonce (it does **not**
deduplicate: two burns backed by the same lock, e.g. two outputs of a split
deposit, would produce a duplicate nonce and the guest would reject it), builds
the ordered accumulator transition from an empty tree, and derives the public
values. It requires all burns to share one trust-base hash, because the public
values commit to a single `trust_base_hash`. `s2::next_batch` then supplies the
real `spent_root_old/new` and witnesses for any nullifier list. Fixtures with
two burns exist (`build_b2_direct_bridge_fixture`, `build_b2_shared_anchor_fixture`,
`build_settlement_fixture_b2`) and the guest's adversarial tests cover them.

### 4.3 The vault settles a batch

`fulfillBatch(publicValues, proof, leaves[], lockRefs[])`: verifies the proof
against the immutable `VKEY`, checks domain, `CONFIG_HASH`, allow-listed
trust-base hash, `spentRootOld == spentRoot` (else `vault: stale root`),
non-empty, `batchSize == leaves.length`, `returnRoot`, `lockRefRoot`, lock refs
strictly ascending and each digest equal to the stored `lockDigest[nonce]`,
per-leaf `fee <= amount`, `sum == totalAmount`; then sets `spentRoot`, emits
`BatchFulfilled`, and pays each leaf (fee only before the deadline, principal
always). The Nile v2 vault is push-payment: a recipient whose transfer reverts
reverts the whole batch (ZK_BACK3 §9 "batch atomicity"; the spec's answer is
to pre-simulate and drop the offending leaf). Measured energy on Nile: about
267,779 fixed plus 19,347 per leaf (`docs/dev-plan/05-cost-analysis.md`); a
2-leaf batch has been settled on Nile from fixtures. `B_max` default 64 in the
yellowpaper, "bounded by external block gas/energy"; the practical bound on
Tron has not been measured above 2.

### 4.4 Cost shape

Proof time is a fixed Groth16 wrap, the dominant part on CPU (about 50 minutes
measured, single worker, peaks near 16 GB), plus trace work that grows with
the number and history length of the tokens in the batch. A batch amortises
the wrap and the on-chain fixed cost; it does not shorten the wait.

## 5. What is missing

1. A batch assembler in the service: take N accepted burns and produce one
   `GuestInput`. Two routes: decode each stored B=1 wire input and merge
   (`bridge_burns`, `return_leaves`, deduplicated sorted lock refs, then
   `s2::next_batch` for the accumulator fields and recomputed
   `return_root`, `lock_ref_root`, `batch_size`, `total_amount`), or keep the
   tokens and reasons and call `build_certified_guest_input_batch`. The first
   route works from fixtures alone, which matters for tests (§11).
2. Lock-ref deduplication and a conflict check (same nonce, different digest
   is corrupt data) before the guest sees the list.
3. Partitioning by trust-base hash: burns verified under different validator
   sets cannot share a proof.
4. A queue policy that forms the next batch from everything pending the moment
   the prover is free, with an upper bound on size, instead of the
   count-or-timeout window.
5. Per-burn failure isolation inside a batch: an already-spent nullifier
   (`next_batch` rejects the whole list today), a token that no longer
   verifies, a leaf that would revert settlement, must each be set aside
   individually. A proof or settlement failure that is genuinely batch-wide
   must mark every member with the same failure and let each be retried.
6. Durable state: returns with their inputs, batches with their bundles and
   status, and the pending order, on the `return-data` volume; a recovery
   procedure at start.
7. Interfaces for the prover, the settler and the chain log so the
   orchestrator can be driven by fakes; the batch policy and the assembler as
   pure functions.
8. Retry with backoff on the service side for recoverable batch failures,
   rather than relying on wallets resubmitting.
9. Observability: batch size, queue depth, last proof duration, per-return
   position, in `/health` and logs.

## 6. Protocol constraints the design must respect

- One trust-base hash per batch (§4.2). Partition, never mix.
- All live burns are `Certified`. Do not attempt anchored batching.
- Witness order equals leaf order equals the order the vault inserts. The
  accumulator fields must be computed against the vault's current root at the
  time of proving, not at intake, and every batch after the first chains onto
  the previous settlement; the loop already re-syncs before each proof.
- Lock refs strictly ascending, unique. Deduplicate identical refs across
  burns; refuse conflicting ones.
- `batch_size` fits `u32`; cap by configuration far below that, starting from
  the yellowpaper's 64, and treat the Tron energy ceiling as unmeasured.
- Nullifier already spent on chain: that burn is already settled (or
  double-submitted); mark it `settled` if a `Released` event exists for it,
  else `failed` final, and drop it from the batch.
- `vault: stale root` on settlement: another settlement moved the root; rebase
  the same burns onto the new root and prove again. The current code names this
  case but the loop does not implement the re-prove.
- Push-mode settlement atomicity: pre-simulate `fulfillBatch` before sending
  (a constant-call against the node; `relayer.js` does not do this today, so
  it is a small addition to the settle command or a new `simulate` command)
  and exclude a leaf whose transfer would revert; the excluded burn is failed
  with a reason, not retried blindly.
- One proof at a time. Proving needs about 16 GB; parallel proving is out.
- Intake precheck stays synchronous and per burn; an accepted burn is one the
  guest will accept, so batch-time verification failures are exceptional and
  must be surfaced loudly.
- No change to the guest, the vault or the deployment config. The verifying
  key and config hash stay as deployed; this is service-only work.
- `precheck_rejected` stays final; `chain_rejected`, `proving_failed`,
  `submission_failed` stay recoverable; `POST` of a recoverably failed
  nullifier re-queues it (implemented today; keep).

## 7. Required properties

**Persistence.** A single-process, single-writer store on the `/data` volume;
crash-safe for the sequence accept → batch → proven → settled. Candidates: an
embedded SQLite (`rusqlite` with `bundled`, no system library), or an
append-only JSON journal with snapshots. The plan chooses and says why. The
store holds per return: the record as served today, the B=1 wire input (or the
token, reason and trust-base hash), attempts and last failure; per batch: the
member ids in leaf order, the patched wire input, the bundle once proven, the
settle txid, status; and the pending order. Proof files under
`BRIDGE_RETURN_PROOF_DIR` stay as they are.

**Restart.** On start, before serving: load state; returns `queued` go back to
pending in original order; a batch `proving` when the process died is
re-formed from its members (the proof is lost, the members return to pending);
a batch `proven` or `submitted` without a settle txid is checked against the
chain log (its nullifiers `Released` → mark settled; else resubmit the bundle,
rebasing if the root moved); `failed` recoverable returns are retried on their
schedule. Wallets' 404 resubmission remains a fallback, not the mechanism.

**Failure isolation.** Per-burn faults (§5.5) remove only that burn. Batch-wide
faults (prover crash, out of memory, settlement command failure) mark every
member `failed` recoverable with the same message, keep the members' inputs,
and schedule a retry with backoff; a member that fails N times in a row is
parked final with a message rather than retried forever. A parked or failed
member never delays the others.

**Wallet agnosticism.** No status is removed or renamed. `batchId`, `progress`,
`nextPollMs`, `message` keep their meaning. Optional additions: `queuePosition`,
`batchSizeSoFar`, an estimate of when proving starts. `returnId` may keep its
derivation; the wallet stores what it is given.

**Testability.** The batch policy (which pending burns form the next batch,
given limits, trust-base hashes, attempt counts, and the clock) and the
assembler (from member inputs and an accumulator state to one guest input)
are pure and tested with fixtures. The orchestrator is tested with a fake
prover that can be scripted to succeed after a delay, fail, or hang; a fake
settler that returns a txid, fails, or reports stale root; a fake chain log
that can be advanced to simulate an external settlement; and a controllable
clock. The existing API tests keep passing.

## 8. Suggested shape (the plan may refine it)

Domain (`service/src/domain/` or similar, no I/O):

- `Burn`: id, nullifier, trust-base hash, leaf, lock refs, input bytes,
  attempts, status. Methods answer questions the policy asks:
  `is_batchable_at(now)`, `shares_trust_base(&other)`.
- `BatchPolicy`: `next_batch(pending: &[Burn], limits, now) -> Vec<BurnId>`.
  Deterministic; oldest first; same trust base; size and byte caps.
- `BatchAssembler`: `assemble(members: &[Burn], acc: &RebuiltAccumulator) ->
  Result<Assembled{ input: GuestInput, excluded: Vec<(BurnId, Reason)> }>`.
  Deduplicates lock refs, drops already-spent nullifiers individually by
  consulting `acc` before calling `s2::next_batch`, recomputes public values,
  runs `precheck_wire` on the result.
- `Batch`: id, members in order, status, bundle, txid, attempts, and the
  transitions (`proven`, `settled`, `failed_batch_wide`, `rebase`).

Ports (traits with an `async fn`, object-safe or generic):

- `ProofBackend::prove(batch_id, wire) -> Result<ProofBundle>` with
  implementations `PrecheckOnly`, `Sp1Groth16`, and a test `Scripted`.
- `Settler::submit(&BatchBundle) -> SubmitOutcome` (existing `Submitter`
  behind it) plus an optional `simulate` for §6 pre-simulation.
- `ChainLog::synced_accumulator()` (existing `ChainEvents`) plus
  `released(nullifier) -> Option<txid>` for recovery.
- `Store`: load, save return, save batch, pending order; SQLite and an
  in-memory implementation for tests.
- `Clock` for the policy and backoff.

Orchestrator: one task. Loop: take the next batch from the policy; if none,
wait for an enqueue or a retry deadline; assemble against the synced
accumulator; persist the batch; prove; persist the bundle; settle; on stale
root rebase and prove again with a bounded count; on batch-wide failure mark
and schedule; on success mark settled. Intake only appends to pending and
wakes the loop. `/health` reads counts from the store.

## 9. Restart and recovery matrix

| Found at start | Action |
|---|---|
| return `queued` | back to pending, original order |
| batch `proving`, no bundle | members back to pending; batch marked interrupted |
| batch `proven`, no txid | if all nullifiers `Released` on chain: settled; else resubmit, rebasing on stale root |
| batch `submitted`, txid, no receipt | check receipt; settled or resubmit |
| return `failed` recoverable | retry per schedule |
| return `failed` final | untouched |
| pending order references unknown id | drop with a warning |

## 10. Configuration

Existing (`config.rs`, `entrypoint.sh`): `BRIDGE_RETURN_PROVE_MODE`,
`BRIDGE_DEPLOYMENT_CONFIG`, `TRUST_BASE_PATH`, `UNICITY_GATEWAY`,
`UNICITY_API_KEY`, `BRIDGE_JUSTIFICATION_TAG`, `BRIDGE_RETURN_BATCH_TARGET`
(default 1), `BRIDGE_RETURN_MAX_WAIT_SECS` (default 60), `SP1_GUEST_ELF`,
`BRIDGE_RETURN_PROOF_DIR` (`/data/proofs`), `BRIDGE_RETURN_SUBMIT_CMD`,
`BRIDGE_RETURN_EVENTS_CMD`, `BRIDGE_RETURN_BIND`, `BRIDGE_CONFIG_HASH`
(optional cross-check), `TRON_SK`, `TRON_VAULT`, `TRON_RPC_URL`.

Proposed: `BRIDGE_RETURN_BATCH_TARGET` becomes the maximum batch size (default
from the yellowpaper's 64, lowered until the Tron ceiling is measured);
`BRIDGE_RETURN_MAX_WAIT_SECS` becomes an optional minimum collection time
before the first proof when the service is idle (default 0); new:
`BRIDGE_RETURN_STATE_PATH` (default under `/data`), retry backoff and the
maximum attempts. Keep the names backward compatible or document the change
in `README.md`, `OPERATIONS.md` §4 and `docker-compose.yml`.

## 11. Tests the plan must include

Unit, with fixtures from `host/src/fixture.rs` and no network:

- policy: forms one batch from all pending when idle; respects the size cap;
  never mixes trust-base hashes; oldest first; skips burns whose retry is not
  due; empty pending yields nothing.
- assembler: two B=1 inputs merge into a B=2 input whose public values match
  `build_b2_direct_bridge_fixture` semantics (roots recomputed, size 2, total
  summed); duplicate identical lock refs collapse to one; conflicting refs are
  rejected; an already-spent nullifier is excluded and the rest proceeds;
  output passes `precheck_wire`; witnesses fold to `spent_root_new`.
- batch transitions: proven, settled, batch-wide failure marks all members,
  rebase increments attempts and is bounded.

Orchestrator, with fakes and a controllable clock:

- burns arriving during a proof form the next batch immediately after it.
- a scripted prover failure fails all members recoverably; the next batch
  still runs; the failed ones are retried after backoff.
- stale root from the settler triggers one rebase and a second proof.
- a settler failure leaves the bundle self-settleable and the members
  recoverable.
- restart: persist, drop the orchestrator, rebuild from the store, and check
  every row of §9.
- the existing `service_api.rs` cases still pass; add one where two posts
  become one batch and both records report the same `batchId` and settle.

Live checks after implementation (documented, not automated): a 2-burn batch
proven in `sp1_groth16` and settled on the Nile v2 vault; a measured energy
figure for the largest batch the deployment will allow.

## 12. Out of scope

Recursion (yellowpaper stage 3), anchored verification, GPU or network
proving, fees, any change to the guest, the vault, the deployment config or
the wallet's API, token or vault versioning, parallel proving, a Rust
replacement for `relayer.js` (keep shelling out; keep the stdin/stdout
contract so it can be swapped later).

## 13. Decisions the plan must make and justify

- Storage engine and schema; migration story for the in-memory store's tests.
- Whether the assembler merges stored wire inputs or re-runs
  `build_certified_guest_input_batch` from tokens.
- Batch trigger: prove-when-free only, or also a minimum idle wait.
- Size caps: count, total input bytes, and how a token's history length
  should weigh (a long-history token makes a slow proof for everyone in its
  batch; consider a per-token cycle estimate later, out of scope now).
- Retry policy: backoff schedule, maximum attempts, what "parked" looks like
  to the wallet (final `failed` with a message, or a new terminal state; a new
  state would need a wallet change, so prefer the message).
- Whether `returnId` derivation changes; it may stay.
- How `/health` and the record expose position and batch size.

## 14. Conventions in force

From `CLAUDE.md`: test first, then compilable code, then fill in; domain
logic on domain types; no comments in new code; small types and functions;
linear complexity; no AI-style prose in docs; feature branch, never `main`;
one-line lowercase commit messages, no prefixes, no trailers; no squashing
without asking. Rust: `cargo fmt`, `cargo clippy`, `cargo test -p
bridge-return-service --offline` works without the `sp1` feature and runs in
seconds; the `sp1` path is exercised only in Docker. The image is rebuilt with
`docker compose build return-service` (minutes, cached guest build) and rolled
with `docker compose up -d return-service`. Docs to update with the change:
`prover/crates/service/README.md`, `docs/OPERATIONS.md` (§4, §5, §7, §8),
`docs/dev-plan/07-return-service.md` (R1/R2 status), `BRIDGING_COMMITTED_WORK.md`.

## 15. Reading order for the fresh agent

1. This file.
2. `prover/crates/service/src/queue.rs`, `store.rs`, `api.rs`, `main.rs`.
3. `prover/crates/host/src/s1.rs` (assembly, intake) and `s2.rs` (accumulator).
4. `prover/crates/guest/src/lib.rs` (the relation; what a batch must look like).
5. `docs/spec/ZK_BACK3.md` §7, §9, §10; `docs/dev-plan/07-return-service.md` R1, R2.
6. `unicity-yellowpaper-tex/appendix-bridging.tex`, section "Batched Proving".
7. `contracts/tron/contracts/UnicityBridgeVault.sol::fulfillBatch`;
   `contracts/tron/scripts/relayer.js` for the `events` and `settle` contracts.
8. `packages/bridge-plugin/src/wallet/return-client.ts` and
   `sphere/src/modules/bridge/bridgeOut.ts` for what the wallet expects.

Glossary: **burn** a Unicity token spent with a bridge-back reason; **nullifier**
its replay key; **leaf** the settlement tuple (nullifier, recipient, amount,
fee, deadline); **lock ref** (nonce, digest) of the source-chain deposit a
token traces to; **accumulator / spent set** the vault's indexed tree of
nullifiers, root `spentRoot`; **anchor** a Unicity certificate a batch shares;
**bundle** the public output of a proof plus the calldata to settle it;
**S1..S4** the service stages in `07-return-service.md`: precheck, sequence,
prove, submit.

## 16. State of the working tree on 2026-09-22

Uncommitted, on the branches named in the resume note, all tests green:

- unicity-bridge `feat/sphere-plugin`: service `api.rs` (precheck rejection
  final), `store.rs` (`insert_or_requeue`, `failed_recoverably`), tests,
  README; `docs/OPERATIONS.md`; bridge-core `BridgePayments.tokenJustification`;
  plugin `wallet/backing.ts` (`mintedAgainst`).
- sphere-sdk `feat/token-plugins`: `readTokenJustification` on the engine and
  `tokenJustification` on payments-v2, with fakes and tests.
- sphere `feat/bridge-v2`: picker offers only tokens the active vault backs;
  recoverable failures stay active with retry and backoff; returns row with
  expandable address and retry button; status-aware post-burn message.

The container `bridge-return-service` runs the rebuilt image in
`precheck_only`. On Nile: v2 vault `TBKJ84417jdxo6j92TxQuYpZdRZGaeZVrv`. The
owner bridged USDT in against v2 and burned once on 2026-09-22; in precheck
mode that return ended as a recoverable `submission_failed` and sits in the
wallet's records, retryable once the service proves for real. One v1 token
was burned earlier by mistake and is unreturnable. Commit the above before
starting the batching branch, or branch from it; do not mix the two changes.
