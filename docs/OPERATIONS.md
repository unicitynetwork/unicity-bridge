# Operating the bridge

For whoever runs the return path in production. How it works is in
`docs/spec/ZK_BACK3.md` and the yellowpaper appendix; this is what to run,
what to decide, what to watch, and what to do when it breaks. Everything here
is taken from the code and the Nile deployment of 2026-09-21; where the
testnet setup falls short of production, the gap is named.

## 1. What runs where

| Piece | Runs | Holds | Trust |
|---|---|---|---|
| Vault (`UnicityBridgeVault`) | the source chain (Tron) | the locked asset, the lock digests, the nullifier accumulator root, the allow-listed validator sets | the contract code plus one admin key (§3) |
| Wallet (Sphere with the bridge module and the asset plugin) | the user's browser | the recovery records for deposits and burns, including the burned blobs (§6) | the user |
| Return service (`bridge-return-service`) | a container you operate | the journal of accepted burns, batches and proofs on the `return-data` volume | none: it can only sequence and prove |
| Unicity aggregator and validators | the Unicity network | certificates for every token transition | the validator set the vault allow-lists |

Outside dependencies at run time: a Tron node (TronGrid by default) for locks,
receipts and settlement; the Unicity gateway for certificates; Succinct's
public artifact bucket, once, for the Groth16 proving key (§5).

Nobody in this list can move locked funds except through a valid proof. The
prover is untrusted: anyone may run one, and a wrong proof fails on chain.

## 2. What to provision

- **Service host.** Proving is single-worker and its memory grows with the
  burn's execution length. The 921k-cycle fixture peaked at about 14 GB
  (`docs/dev-plan/04-deployment.md`); a live 10 USDT burn measured at
  3,393,715 cycles (`bridge-return-host sp1-execute`) exceeded 14.6 GB of RAM
  plus 4 GB of swap inside Docker on a 16 GB Mac and was killed every time,
  and a live Sepolia USDC burn peaks at about 26 GiB. Each proof runs in a
  child process (`BRIDGE_RETURN_PROVE_CMD`), so that memory goes back to the
  OS when the proof ends and the service itself stays small between proofs:
  size the memory limit for one proof's peak plus the service, not for two.
  Plan 32 GB or more for live tokens, and expect a proof to take longer than
  the fixture's hour. Precheck alone needs a fraction of that. 8 GB of disk
  for the artifacts and bundles.
- **Shared memory.** SP1's executor shares the guest's memory and its trace
  ring with the prover through `/dev/shm`: an 8-burn batch uses about
  188 MiB there, and Docker gives a container 64 MB unless told otherwise.
  `docker-compose.yml` sets `shm_size` to 8 GB (`BRIDGE_RETURN_SHM_SIZE`); give
  a container started some other way at least 2 GB. Its pages count against
  the container's memory, so it is a ceiling, not a reservation.
- **Network.** Outbound HTTPS to the Tron node, the Unicity gateway and the
  artifact bucket. Inbound HTTPS from wallets on the service port (8787 in the
  container). The service speaks plain HTTP; put TLS in front of it.
- **Settlement account.** A source-chain account with enough native currency
  for gas. One settlement costs about 287,000 energy on Tron (verify plus
  transfer); price it with `docs/dev-plan/05-cost-analysis.md`.
- **The wallet.** Sphere posts each deployment's burns to the service its own
  deployment config names, and offers an asset for bridging in only when none is
  set. For Sepolia USDC (`https://bridge-usdce.testnet.unicity.network`):
  - Sphere's containers read `BRIDGE_RETURN_SERVICE_URL_SEPOLIA_USDC` from the
    task definition (unicity-sphere/sphere-infra, `aws/sphere-site/`);
  - Pages previews and local builds read
    `VITE_BRIDGE_RETURN_SERVICE_URL_SEPOLIA_USDC`;
  - the container adds the service's origin to its CSP `connect-src`.

  Set it only once the settlement account is funded. Tron Nile USDT has no
  deployed service; a local build names one with
  `VITE_BRIDGE_RETURN_SERVICE_URL_NILE_USDT`. A new deployment's service needs a
  Sphere change for its variable.

## 3. Keys

| Key | Where | Can | Cannot |
|---|---|---|---|
| Vault admin | whoever deployed the vault (on Nile: the demo account) | allow-list or remove a validator-set hash; hand the role over | change the verifying key, the config, the asset; move funds |
| Settlement account (`TRON_SK`) | the service host | submit settlement transactions and pay their gas | anything else; settlement is permissionless, so this key holds no authority |
| Unicity gateway key (`UNICITY_API_KEY`) | the service host, optional | authenticate certificate fetches | anything else |
| Wallet aggregator key | the user's wallet | mint and transfer on the user's behalf | anything on the source chain |

**The admin key is the one to protect.** Allow-listing a validator set it
controls would let its holder forge burns. The yellowpaper requires this to be
time-locked; the deployed contract applies changes immediately. For real
value, hold it in a multi-signature or hardware setup and treat
`setTrustBaseAllowed` as a governance action, or add the time-lock to the
contract and redeploy.

Users' deposit keys are their own; the bridge never holds them.

## 4. Configuration

The single tracked record of a deployment is `deployments/<net>/<asset>.json`
(the current one is `deployments/nile/nile-usdt-v2.json`). Three
implementations read the same values and a test in the prover
(`crates/host/tests/nile_config.rs`) checks the file against the hash the
contract committed on chain. If they ever disagree, nothing settles.

| Value | Wallet | Service | Vault |
|---|---|---|---|
| config hash | manifest, checked at load | deployment file | `CONFIG_HASH`, immutable |
| verifying key | manifest, display only | the guest ELF in the image | `VKEY`, immutable |
| validator-set hash | from the SDK's trust base | `TRUST_BASE_PATH`, hashed at start | `trustBaseAllowed[hash]`, admin-set |
| vault address | manifest | deployment file, `TRON_VAULT` for settlement | itself |

Service environment (defaults from `prover/docker/entrypoint.sh`):

| Variable | Default | Meaning |
|---|---|---|
| `BRIDGE_RETURN_PROVE_MODE` | `precheck_only` | `sp1_groth16` for real proofs |
| `BRIDGE_DEPLOYMENT_CONFIG` | `/app/deployments/nile/nile-usdt-v2.json` | the frozen deployment |
| `TRUST_BASE_PATH` | `/app/bft-trustbase.testnet2.json` | validator set the burns must verify under |
| `UNICITY_GATEWAY`, `UNICITY_API_KEY` | testnet2 gateway, none | certificate source |
| `BRIDGE_JUSTIFICATION_TAG` | `1330002` | the mint-reason tag of the bridged asset |
| `BRIDGE_RETURN_STATE_DIR` | `/data/state` | the journal, on the `return-data` volume |
| `BRIDGE_RETURN_MAX_BATCH_SIZE`, `BRIDGE_RETURN_MAX_BATCH_BYTES` | `8`, `8388608` | most burns, and most summed input bytes, in one proof |
| `BRIDGE_RETURN_IDLE_WAIT_SECS` | `0` | collection window before the first proof when the service is idle |
| `BRIDGE_RETURN_COMMAND_TIMEOUT_SECS` | `600` | a relayer command (settle, simulate, events, confirm, receipt) that runs longer is killed and counts as a failed attempt. The relayers' `settle` stops waiting for the receipt 30 s before it and reports the transaction as unconfirmed, and the service then waits on that transaction (§5) |
| `BRIDGE_RETURN_VKEY_CMD` | `bridge-return-host sp1-vkey` (the image sets its full path) | the command that prints the guest's verifying key at start, in a child process like the proofs, so the service itself loads no SP1 code |
| `BRIDGE_RETURN_PROVE_CMD`, `BRIDGE_RETURN_PROVE_TIMEOUT_SECS` | `bridge-return-host sp1-groth16-files` (the image sets its full path), `7200` | the command that proves one batch in a child process, given the ELF, wire, proof and info paths; a proof that runs longer is killed and counts as a failed attempt, so set the limit well above the slowest proof the host makes (a full batch on a slow host can pass two hours), or `0` for no limit. The child leads its own process group: a timeout, a failed attempt and an abandoned proof (§5, Stopping) kill all of it, so a restart never proves next to an orphan. Before each proof, and after a failed one, the service removes the SP1 files in `/dev/shm` that no process has open or mapped, which a killed prover leaves behind. `bridge-return-host` exits with status 70 when SP1 fails a task the proof needs, instead of waiting for the timeout: SP1 logs the failure but never reports it to the caller |
| `BRIDGE_RETURN_DRAIN_TIMEOUT_SECS` | `0` (`2640` in `docker-compose.yml`) | the most a drain waits for the batch in flight before it abandons the proof (§5, Stopping); `0` waits however long the batch takes. Keep it below the container's stop grace period, so the service abandons the proof itself, which costs no attempt, before Docker kills it, which costs one |
| `BRIDGE_RETURN_RETRY_BASE_SECS`, `BRIDGE_RETURN_MAX_ATTEMPTS`, `BRIDGE_RETURN_MAX_REBASES` | `60`, `5`, `3` | retry backoff, attempts before a return is parked, rebases before a batch fails |
| `BRIDGE_RETURN_FEE_AMOUNT`, `BRIDGE_RETURN_FEE_FLOOR`, `BRIDGE_RETURN_FEE_RECIPIENT`, `BRIDGE_RETURN_FEE_WINDOW_SECS` | `0`, the amount, none, `86400` | the fee quoted to wallets, in the asset's smallest unit; the least a burn must pay to be accepted; the source-chain account that receives it; the time the service wants between accepting a burn and the burn's fee deadline |
| `SP1_GUEST_ELF` | `/app/sp1/bridge-return-sp1-guest` | the program whose key the vault holds; checked against the deployment's `vkey` at start |
| `BRIDGE_RETURN_PROOF_DIR` | `/data/proofs` | proof bundles |
| `TRON_SK`, `TRON_VAULT`, `TRON_RPC_URL` | none, none, Nile | settlement and chain sync, through the relayer |
| `ETH_CONFIRM_RPC_URL`, `TRON_CONFIRM_RPC_URL` | none | a second RPC, independent of `ETH_RPC_URL`/`TRON_RPC_URL`, through which the relayer confirms a release the service did not see settle (§5). The image's entrypoint sets `BRIDGE_RETURN_CONFIRM_CMD`, so without this the service refuses to start; set `BRIDGE_RETURN_CONFIRM_CMD` empty to run without a second RPC, in which case such returns stay `submitted` |
| `BRIDGE_RETURN_SUBMIT_CMD`, `BRIDGE_RETURN_EVENTS_CMD`, `BRIDGE_RETURN_SIMULATE_CMD`, `BRIDGE_RETURN_CONFIRM_CMD`, `BRIDGE_RETURN_RECEIPT_CMD` | the Node relayer | settlement, event scan, pre-simulation (lock refs, transfers), release-confirmation and settlement-lookup hooks; replaceable by any program with the same stdin/stdout contract (a replacement events command must name each batch's `txid`). The service refuses to start without a simulate command, and when a configured confirmation or receipt command cannot run |
| `BRIDGE_RETURN_MAX_PENDING` | `1000` | most returns held unfinished; beyond it a new burn is refused with `queue_full` (HTTP 503, recoverable) |
| `BRIDGE_RETURN_BIND` | `0.0.0.0:8787` | listen address |
| `SP1_WORKER_NUM_*` | `1` | single-worker proving; the only setting that fits a 16 GB host (`docs/dev-plan/03-status.md`) |

## 5. Running the service

```
docker compose up -d return-service        # from the repository root; reads .env
docker logs -f bridge-return-service
curl localhost:8787/health
```

Start in `precheck_only`. In that mode every burn is fully verified against
the trust base and the deployment, queued and batched, and nothing is proven.
Switch to `sp1_groth16` once the pipeline is confirmed, and empty
`BRIDGE_RETURN_STATE_DIR` when you do: batches proven in precheck mode carry
no proof and would otherwise sit at `proven` for good (the wallets re-post
their burns on the resulting 404). The first proof
downloads the Groth16 circuit and proving key, about 5.9 GB, into the
`sp1-artifacts` volume; keep the volume across restarts or every restart pays
the download. Never set `SP1_CIRCUIT_MODE=dev`: it selects a private artifact
bucket and a key the vault does not accept.

**Stopping.** SIGTERM drains the service; `docker stop`, `docker compose
down` and the recreate in `docker compose up -d` all send it. The batch in
flight runs to its end, settled or failed. No other batch starts, and the process
exits 0 once nothing is in flight, at once when nothing was. Until then it keeps
answering: `/health` shows `draining: true`, and `POST /returns` keeps accepting
burns for the next start. A second SIGTERM, a SIGINT (Ctrl-C) or
`BRIDGE_RETURN_DRAIN_TIMEOUT_SECS` abandons the proof instead. Its batch goes
back to the head of the queue and is proven again at the next start, at no
attempt to its members; only a crash, an OOM kill or a SIGKILL costs one. Docker
sends SIGKILL ten seconds after SIGTERM unless told otherwise, so
`docker-compose.yml` gives the service a 45-minute `stop_grace_period` and a
44-minute drain timeout. Change the two together, and size them for the
slowest proof plus its settlement. To stop at once while a stop is waiting on
a drain, `docker kill -s TERM bridge-return-service` abandons the proof.

Proofs run one at a time. When the prover is free, the next batch is
everything pending, oldest first, up to `BRIDGE_RETURN_MAX_BATCH_SIZE` burns
that share one validator set and trace to distinct deposits. Burns that arrive
during a proof form the next batch, so a return waits at most two proof times.
`BRIDGE_RETURN_IDLE_WAIT_SECS` adds a collection window before the first proof
when the service is idle; the default is none. Raise the batch size only after
measuring the energy of the largest batch on the target chain
(`docs/dev-plan/05-cost-analysis.md`).

Before proving, the service rebuilds the vault's accumulator from the on-chain
settlement events (`BRIDGE_RETURN_EVENTS_CMD`) and checks it against the live
root. `/accumulator` reports `synced: true` when they agree. A proof built on a
stale root is rejected by the vault with `vault: stale root`; the service then
rebases and proves again (§8).

Settlement goes through `BRIDGE_RETURN_SUBMIT_CMD` with the bundle on stdin,
and the chain log through `BRIDGE_RETURN_EVENTS_CMD`. Both default to the
relayer of the deployment's chain family, chosen by `BRIDGE_RELAYER`:
`relayer-eth.js` (Ethereum-family, ethers over `ETH_RPC_URL`, key `ETH_SK`,
vault `ETH_VAULT` scanned from `ETH_VAULT_DEPLOY_BLOCK`) or `relayer.js`
(Tron, TronWeb over `TRON_RPC_URL`). Both live in `contracts/tron/scripts`. The
all-Rust submitter the plan calls for is not built; until it is, the container
carries Node for this.

The chain log comes from one RPC, so a release in it that the service did not
submit itself (someone else submitted the public bundle first, or the relayer
lost track of its own transaction) does not settle a return on its own say.
`BRIDGE_RETURN_CONFIRM_CMD` checks the transaction the log names through
`ETH_CONFIRM_RPC_URL` (`TRON_CONFIRM_RPC_URL`): it must have succeeded, be
`ETH_SETTLE_CONFIRMATIONS` deep (on Tron, solidified), and carry the vault's
`Released` event for the nullifier. Only then is the return `settled`, with that
transaction as its `settleTxid`. Until then it reads `submitted`, the service asks
the second RPC again every 5 minutes at no attempt's cost, reading the log again
only when that RPC answers without confirming, and a return whose release the log
no longer shows is proven again. Point the second RPC at a different provider from
the first: the same provider behind another URL confirms nothing it did not
already say.

The relayer prints a settlement's txid as soon as it has signed it, before sending
it, and the service journals it as `batch_submitted` then. If the relayer does not
report the settlement settled (it exits 75 because the RPC rate-limited its receipt
reads, fails, or outlasts its timeout), or the service restarts meanwhile, the
service waits on that transaction rather than sending another:
`BRIDGE_RETURN_RECEIPT_CMD` looks it up through `ETH_RPC_URL` every 30 s for 15
minutes, with no other batch started meanwhile, then every 5 minutes while other
batches run. The batch settles once the transaction is final; it fails as
`submission_failed`, charging its members an attempt, only once the transaction
reverted, or the chain has not known it at any lookup for 10 minutes, and the vault
log shows the batch neither released by another transaction nor overtaken (overtaken,
it is re-proven at no attempt's cost). Reads that fail cost nothing, and a drain
does not wait for the lookups.

The service settles for free unless `BRIDGE_RETURN_FEE_AMOUNT` is set. The fee
is a flat amount of the bridged asset per burn. The owner's burn names it, the
vault takes it out of the owner's payout and pays it to
`BRIDGE_RETURN_FEE_RECIPIENT` when the batch settles on or before the burn's
deadline; a return that settles later pays the owner in full and the service
nothing. The fee does not replace gas: the settlement account still pays in the
chain's native currency.

Wallets read the terms from `GET /fees`: the recipient, the amount, and a
deadline dated by the service clock, `BRIDGE_RETURN_FEE_WINDOW_SECS` plus seven
days ahead. The floor is what the service enforces, and it is `0` unless set:
quoting a fee and enforcing it are two separate steps, because a burn refused
at intake is already burned. With a floor, a burn is accepted when it names
the recipient, pays at least the floor and still has the window left before its
deadline, so a wallet has seven days to submit a burn it built from a quote.
A burn that pays less or names another account is refused with `fee_not_paid`;
one submitted so late that its deadline no longer leaves the window, because
the service was down or the wallet was away, is refused with
`fee_deadline_passed`. Both refusals are recoverable: the wallet keeps the
burned token and resubmits it on its own, and the burn is taken once the floor
is lowered, at `0` for a late one. The vault still pays a late burn's owner in
full and the service nothing, so taking one is the operator's call, and the
service does not take it unasked: a wallet could otherwise skip the fee by
dating its deadline in the past. A return the service already holds is not
checked again. Set the window above the longest a return may wait in the queue,
since a burn may carry a deadline only that far ahead.

Use the floor to change the terms without stranding burns already made. Quotes
live seven days, so a change the floor does not cover needs the floor at `0`
for seven days first:

- First start: deploy the service, then a wallet that reads `/fees`, then set
  the amount. Wallets pay the fee and burns from older wallets are still taken.
  Set the floor to the amount once the older wallets are gone.
- Raising: raise the amount and keep the floor at the old amount for a week,
  then raise the floor.
- Lowering: lower both at once.
- A new recipient, or a longer window: set it together with the floor at `0`,
  and raise the floor a week later. Burns from the old quotes are taken
  meanwhile and pay the account they name.

The recipient must be an account that can send a transaction to the vault: on a
pull-payment vault the fee accrues in `owed` and only the recipient can collect
it, with `withdraw`. The settlement account is the natural choice; do not use
the vault admin, whose key should stay out of routine use. A burn whose fee
lapsed shows as a `Released` event with a zero fee.

A settlement counts as settled only once it is final, because the service
records it in its journal and rebuilds the accumulator from that record when
the RPC no longer serves the log. `relayer-eth.js` waits for
`ETH_SETTLE_CONFIRMATIONS` confirmations (default 12, the Sepolia lock finality;
raise `BRIDGE_RETURN_COMMAND_TIMEOUT_SECS` with it on a slower chain) and reads
the settled log and the vault's root only up to the height those confirmations
make final, so a restart during the wait cannot record the settlement early.
`relayer.js` waits for the solidity node, which answers only for solidified
blocks, and reads only confirmed events.

The Ethereum RPC must serve logs and receipts back to the vault's deploy
block: the wallet reads old lock transactions to verify tokens, and a fresh
service rebuilds the spent-nullifier accumulator from every settlement since
the deployment. publicnode keeps only about the last 10,000 blocks (some 33
hours) and answers with empty results beyond that, which stalled the Sepolia
service on 2026-09-25 once the first settlement aged out. A running service
fills gaps in the log from the batches it settled itself, so it survives a
short-history RPC as long as every settlement went through it; a fresh service
on an older vault needs a provider with full history. Tenderly's public gateway
(`https://sepolia.gateway.tenderly.co`) has the history and is what the wallet
manifest reads from, but it throttles bursts with HTTP 429, which left a
settlement broadcast retrying for half an hour, so it is not the service's
default. Commands the service runs against the chain are cut off after
`BRIDGE_RETURN_COMMAND_TIMEOUT_SECS` (default 600) and count as a failed attempt.

`BRIDGE_RETURN_SIMULATE_CMD` (`relayer-eth.js simulate --stdin` by default)
checks each leaf's lock refs against the vault's `lockDigest`, read at the block
`ETH_SETTLE_CONFIRMATIONS` makes final, before proving the batch. A lock ref the
vault does not hold fails that return for good; a lock not yet that deep is
retried. In push-payment mode it also drops a leaf whose transfer would revert;
the reason is the token's own revert string (USDC: `ERC20: transfer amount
exceeds balance`), and the return is retried. Against a pull-payment vault, such
as the Sepolia one, only the lock refs are checked, since settlement only
credits what the vault owes. The Tron relayer's reading of the constant-call response was
never checked against Nile (§8).

## 6. Where the money-critical state lives

- **The burned blob is the claim.** Once a token is burned, the only thing
  that can release the funds is a proof over that blob. The wallet writes the
  blob to its own records before it releases the wallet SDK's copy, and only
  then sends it to the service. Anyone holding the blob can resubmit it; the
  service is idempotent on the burn's nullifier.
- **Those records are in the user's browser**, keyed by wallet identity, in
  local storage. Deleting the wallet deletes them. The service journals every
  accepted burn with its input to `BRIDGE_RETURN_STATE_DIR`, so it is a second
  copy of unsettled burns for as long as the `return-data` volume lives; back
  the volume up if that copy matters.
- **Proof bundles** land in `BRIDGE_RETURN_PROOF_DIR`. A bundle is public data;
  anyone may submit it. `/batches/:id` serves it once the batch has settled or
  every burn in it is past its deadline, when the spec leaves settling to anyone;
  before that a third party submitting it first would only make the relayer's own
  transaction fail. A service without a submitter serves it at once.
- **The accumulator** is reconstructable from the vault's `Released` and
  `BatchFulfilled` events alone. Nothing off chain needs a backup for it.
- **The lock digests** are per-vault state on chain. Value locked in a vault
  can only ever leave through that vault (§9).

## 7. Monitoring

`GET /health` returns `status`, `draining`, `queueDepth`, `activeBatch`, `activeBatchSize`,
`provingSinceMs`, `lastProofMs`, `averageProofMs`, `unconfirmedReleases`, `unconfirmedSettlements`, `maxBatchSize`,
`idleWaitMs`, `proveMode`, `chainSync`. `GET /accumulator` returns `synced`, `spentRoot`, `spentCount`.
Logs are structured (`RUST_LOG`, default info). The journal itself is readable:
`jq -c 'keys[0]' /data/state/journal.jsonl` lists the event types.

Worth alerting on:

- `/health` unreachable, or `proveMode` not what you configured.
- `/accumulator` `synced: false` for more than a few minutes: the event scan
  or the Tron node is failing, and nothing will settle.
- `queueDepth` growing with `activeBatch` unchanged for longer than a proof
  takes: proving is stuck or out of memory.
- A batch proving for more than two proof times (`provingSinceMs` says when it
  started), or a return whose `failure.message` says it is parked.
- `unconfirmedReleases` above zero for more than a few checks (5 minutes each):
  the chain log shows a payout the second RPC does not confirm, or no second RPC
  is configured. Each such return's `message` names the transaction.
- `unconfirmedSettlements` above zero for more than about 20 minutes (it is 1
  while any settlement is in flight): a settlement transaction is stuck unmined
  (gas price, a nonce gap) or `ETH_RPC_URL` cannot be read. Its members'
  `message` names the transaction.
- Settlement account balance below a few settlements' worth of gas.
- On chain: every `BatchFulfilled` should match a bundle the service wrote.

## 8. Failure modes

| Symptom | Cause | What to do |
|---|---|---|
| Wallet shows a return as `burned` that never becomes `queued` | service unreachable | fix the service; the wallet resubmits on its own |
| Service refuses a burn with a config-hash mismatch | the token was minted against another vault (a v1 token after the v2 redeploy) | nothing to do here; only that vault's program can release it. The wallet does not offer such tokens for a burn; a blob that reaches the service anyway is refused as final |
| Return `failed` with a non-recoverable message | the burn will never be accepted (wrong config, malformed reason) | the funds stay on Unicity as a burned token; investigate before telling the user |
| Return `failed` with `fee_not_paid` or `fee_deadline_passed` | the burn pays less than the floor, names another account, or came in too late for its deadline | lower the floor (to `0` for a late burn), and the wallet's own resubmit is accepted; raise it again afterwards |
| Return `failed` with `service_unavailable` and `accumulator not synced to chain` | the events command could not read the chain for about two minutes: the batch already read it again after 15, 30 and 60 seconds (`chain log unavailable` warnings in the log), and `relayer-eth.js events` repeats each failed read three times before it exits non-zero | the node or RPC endpoint is down or rate-limiting; fix or replace `ETH_RPC_URL`/`TRON_RPC_URL`. The retry reuses the proof as long as the vault root has not moved |
| Return `failed` with a recoverable message (`submission_failed`, `proving_failed`, `chain_rejected`) | settlement or proving hit a transient fault | the service retries on its own at `notBeforeMs` (doubling backoff; a settlement retry reuses the proof) and parks the return as final after `BRIDGE_RETURN_MAX_ATTEMPTS`; a resubmit from the wallet re-queues it without shortening the schedule; fix the cause (gas, memory, node) meanwhile |
| Return `failed` with a message saying it is parked | the same fault repeated `BRIDGE_RETURN_MAX_ATTEMPTS` times | fix the cause, then see §9, Un-parking returns |
| Service restarted | the journal is replayed | nothing to do: queued burns stay queued, a proof the service abandoned to stop (§5, Stopping) re-runs at once without an attempt, one a crash cut off re-runs after the retry backoff, a proven batch is settled or resubmitted after a chain check. A 404 on the wallet's `returnId` means the state directory was lost; the wallet resubmits the blob |
| Return `failed` with `proving_failed` saying the proving command `was killed by signal 9` (the Docker VM's `dmesg` shows an `oom-kill` of `bridge-return-h`) | the proof outgrew the memory limit: a live Sepolia USDC burn peaks at about 26 GiB, 23 GB killed it in a 24 GB WSL VM on 2026-09-23 | the service keeps running and retries with backoff; each kill counts as an attempt, so the return parks after `BRIDGE_RETURN_MAX_ATTEMPTS`. Size the limit for one proof's peak plus the service; keep the single-worker `SP1_WORKER_NUM_*=1` settings and the `GOMEMLIMIT`/`GOGC` defaults from `docker-compose.yml` (a soft heap limit the Go collector honours) |
| Return `failed` with `proving_failed` saying the proving command `exited with status 70`, and the log above it shows `CoreExecute: task failed: … Child native executor crashed, details: CrashDetails { signal: 7, … }` | SP1's executor died of SIGBUS: `/dev/shm` is too small for the batch (Docker's default 64 MB fits batches of one or two burns), or full of files killed provers left | give the container more shared memory (§2) and restart it. The host exits on the failure at once and the service retries after the backoff; each failure counts as an attempt, so returns may park meanwhile (§9, Un-parking returns). Files a killed prover left in `/dev/shm` are removed before the next proof; `removed SP1 shared-memory files no process holds` in the log says how many |
| Service restarts while proving; the return says `Proof interrupted by a service restart` | the service process died while proving: a crash, an OOM kill of the service itself, or a SIGKILL (Docker's included, once the stop grace period runs out) | each such interruption counts as an attempt, so the return parks after `BRIDGE_RETURN_MAX_ATTEMPTS` instead of looping forever; a proof is never interrupted by the memory it took itself, since it runs in a child |
| Return `submitted`, its message saying the vault log shows it released in a transaction a second source has not confirmed | the service did not submit that release itself (someone submitted the public bundle first, or the relayer lost track of its own transaction), and `BRIDGE_RETURN_CONFIRM_CMD` could not confirm it: the second RPC is unset, the same endpoint as the first, down, behind, or disagrees | look the transaction up on an explorer. Paid: fix `ETH_CONFIRM_RPC_URL`, and the next check settles the return with that transaction. Not paid: the first RPC is wrong; replace `ETH_RPC_URL`, and once the log no longer shows the release the return is proven again |
| Return `proven`, its message saying `Settlement 0x… broadcast; waiting to see it settle`, and `unconfirmedSettlements` above zero for long | the relayer signed and sent the settlement but did not see it settle, and the transaction is still pending, or the receipt command cannot read it | nothing is charged while it waits. Look the transaction up on an explorer: pending, it settles the batch once mined (if it is stuck, replace it at the same nonce; the vault accepts only one settlement of the batch); mined, check `ETH_RPC_URL` and `BRIDGE_RETURN_RECEIPT_CMD` |
| Return `failed` with `submission_failed` saying a settlement `reverted on chain` or `was dropped` | the relayer signed the settlement, which then reverted, or which the chain did not know at any lookup for 10 minutes (never sent, refused, or dropped from the mempool), and the vault log shows the batch neither released nor overtaken | the retry resubmits the same proof; a dropped transaction that lands later anyway makes the retry find the batch released, and it settles from the log |
| Settlement reverts with `vault: stale root` | another batch settled first, or the event scan lagged | the service re-proves the same batch on the new root, up to `BRIDGE_RETURN_MAX_REBASES` times, then fails it recoverably |
| Settlement fails with `OUT_OF_TIME: CPU timeout` and is charged the whole fee limit | the network's per-transaction CPU limit (`getMaxCpuTimeOfOneTx`) is below what the Groth16 verification needs. Nile lowered it from 160 ms to 80 ms by proposal 20699 on 2026-09-08; the July settlement ran under 160 ms, and since the change even the repository's published bundle times out in a free `verifyProof` call | nothing on the service side; stop the service so retries do not each pay the fee limit, keep `feeLimit` in `relayer.js` just above a real settlement's cost so a timeout is cheap, and check the free `verifyProof` call before starting again; a retry reuses the proof |
| Settlement reverts with `vault: trust base not allowed` | the validator set changed and the new hash is not allow-listed | admin allow-lists it (§9); proofs under the old set still settle if that hash remains allowed |
| A recipient's transfer would revert and take the batch with it | push payments, a hostile or blocked recipient | the simulate command excludes the leaf before proving and the service fails that return recoverably; a deployment expecting this uses pull payments (`PULL_PAYMENTS`, immutable, chosen at deploy) |
| Return `failed` for good with `lock refs are not the vault's` | the burn draws on a lock the vault never stored (a fabricated or foreign burn) | nothing; the return never re-enters a batch |
| `POST /returns` answers `queue_full` (HTTP 503) | `BRIDGE_RETURN_MAX_PENDING` returns are unfinished | wait for batches to settle (the wallet's resubmit is accepted then), or raise the limit |
| First proof fails with a truncated proving key | interrupted download | delete the artifact volume and let it download again; verify the size against `docs/dev-plan/03-status.md` |
| Proof out of memory | under one proof's peak available (about 26 GiB for a live burn on the current guest) | more memory; there is no smaller mode |

## 9. Procedures

**Validator set change.** Compute the hash of the new trust base
(`bridge-return-host emit-trust-base-hash <file>`). The admin calls
`setTrustBaseAllowed(hash, true)`; keep the old hash allowed until every burn
made under it has settled, then remove it. Point `TRUST_BASE_PATH` at the new
file and restart the service. The wallet takes the trust base from the SDK's
network table and needs a release.

**Prover program change.** Any change to the guest, including reading a new
token format, changes the verifying key, and the key is immutable in the vault.
The paper's procedure, followed on 2026-09-21:

1. Build the program with the `guest-from-source` target of
   `prover/Dockerfile`, commit its output to `prover/guest-elf/`, derive the
   key, and record the program hash. The source build is not reproducible, so
   the committed ELF is the program from then on.
2. Deploy a new vault with `contracts/tron/scripts/deploy-nile.js real-vault
   <asset> <vkey>`; the same verifier contract is reused. Freeze
   `deployments/<net>/<asset>-vN.json` with the config the host derives
   (`emit-config`) and check it equals the on-chain `CONFIG_HASH`.
3. Allow-list the current validator set on the new vault. Point the plugin
   manifest, the wallet and the service at it.
4. Stop bridge-in against the old vault. **Return every outstanding token
   through the old vault under the old program first**: the new vault knows
   nothing about the old locks. Value left in the old vault when it is
   abandoned is gone. On the testnet this was written off; in production it is
   a migration window with the old service kept running.

**Asset or configuration change.** The config hash is immutable, so this is
the same procedure as a program change.

**Admin hand-over.** `transferAdmin(newAdmin)`, once, from the current admin.

**Service upgrade.** Rebuild the image. It carries the guest ELF committed in
`prover/guest-elf/`, and the build fails unless that ELF's key is the one the
deployment file pins. In `sp1_groth16` mode the service makes the same
comparison at start against `BRIDGE_DEPLOYMENT_CONFIG` and exits on a mismatch.
`/app/sp1/vkey.txt` in the image shows the key. Replacing the container sends
SIGTERM, so the deploy waits for the batch in flight to finish (§5, Stopping). Set
`ETH_CONFIRM_RPC_URL` (`TRON_CONFIRM_RPC_URL`) before moving to an image whose
entrypoint sets `BRIDGE_RETURN_CONFIRM_CMD`: images that check the command at start
refuse to start without it, and the earlier one leaves a return released by a
transaction the service did not submit `submitted` instead of `settled`. A
replacement `BRIDGE_RETURN_EVENTS_CMD` must name each batch's `txid`, as the Node
relayers do: since the image that confirms releases, a log without it leaves every
release the service did not submit `submitted`, a breaking change for such a
command.

Rolling back to an older image has limits, and getting them wrong can lose the
ledger. An image from before release confirmation reads an unconfirmed release as
settled, without the second-source check, and treats a `submitted` return in the
journal's snapshot as neither pending nor done, so it never moves again. An image
from before `batch_submitted` cannot read that event or a `submitted` batch, and
skips a last journal line it cannot read as if a write had been cut off: a trailing
`batch_submitted` line is dropped, so the batch is sent again, and a journal
compacted to one snapshot line holding a `submitted` batch is dropped whole, so the
older image starts with an empty ledger and compacts that. Roll back only while
`/health` shows `unconfirmedReleases` and `unconfirmedSettlements` at 0, and only
after a restart of the current image has compacted the journal in that state; copy
`journal.jsonl` aside first. Images from this one on refuse a complete line they
cannot read instead of skipping it.

**Un-parking returns.** A return parks when a failure brings its attempts to
`BRIDGE_RETURN_MAX_ATTEMPTS`. Parking is not recorded as such: the journal records
the failure, the limit decides it each time the journal is replayed, and every
start then compacts the journal into a snapshot that keeps the outcome. So the
first start after the cause is fixed can un-park the returns that parked while the
service last ran:

1. Fix the cause (shared memory, memory, gas, the node).
2. Copy `journal.jsonl` aside.
3. Start the service with `BRIDGE_RETURN_MAX_ATTEMPTS` above the parked returns'
   attempts, for example 10 instead of 5. Their failures replay under the new
   limit, so they come back as recoverable failures and are retried once their
   backoff, counted from the failure, has passed (usually at once), with the
   attempts the new limit leaves them.
4. Keep the higher limit until they settle: a later failure parks a return
   against the limit then in force.

Any start compacts the journal, so a start with the old limit, even a routine
restart before the fix, keeps them parked for good. A return that parked before
an earlier start, or at a start (a restart interrupting its proof for the last
allowed time), is parked in the snapshot already, and a higher limit does not
change it. For such a return: stop the service, remove the return from the
journal's snapshot line, start it, and the wallet's next resubmit is accepted as a
new return.

## 10. Known gaps before real value

- The admin key has no time-lock (§3).
- The SP1 Groth16 verification does not finish within Tron's 80 ms
  per-transaction CPU limit. Mainnet and Shasta run 80 ms, and Nile matched
  them on 2026-09-08 (it was 160 ms when the July settlement succeeded).
  Until the verification is made faster or the limit raised, no batch settles
  on Tron; every attempt is charged the fee limit.
  Tron has no shipped fix (java-tron PR 5507 closed unmerged, issue 6374 open).
  The same verification runs on Ethereum Sepolia through Succinct's SP1 gateway
  at about 267k gas with no time bound, checked on 2026-09-23; the vault
  deploys there unchanged (`docs/dev-plan/09-ethereum-sepolia.md`).
- The journal is one file on one volume, with no backup and no pruning: done
  returns keep their inputs in it until a pruning step exists.
- Transfer pre-simulation is checked for `relayer-eth.js` against the vault
  bytecode and Sepolia; the Tron relayer's was never checked against Nile. Off
  by default.
- The API is open: CORS is permissive and there is no rate limiting. Every
  route is permissionless by design, and a submitted burn costs the submitter
  their own token, but precheck is CPU that anyone can spend.
- Settlement runs through a Node relayer per chain family, not the planned Rust submitter.
- Milestone 2 ran on Ethereum Sepolia on 2026-09-23 (a wallet burn proven
  with the current program and settled on the Sepolia v2 vault,
  `docs/dev-plan/09-ethereum-sepolia.md`); it never ran on the Tron v2 vault.
  The three prover tests that use a live sample are still ignored.
- The wallet's service URL is a build-time setting.
- The three wallet packages are linked by path, not published (see
  `sphere/docs/WALLET-MODULES.md`).
- The repository's `.env` holds throwaway testnet keys for the scripts
  (`TRON_SK`, `ETH_SK`); they must not reach a production host. The wallet
  signs only through browser wallets.

## 11. Cost analysis

The cost model is `docs/dev-plan/05-cost-analysis.md`: gas per settlement, how
batching divides it, and proving. It was measured on Tron; the first Sepolia
settlement used 324,439 gas (`docs/dev-plan/09-ethereum-sepolia.md`).

**The operator fronts the gas.** The settlement account pays for every
`fulfillBatch` in the chain's native currency, and proving runs on the
operator's hardware. Nothing on chain refunds either.

**The token owner can be charged a fee in the bridged asset.** The service
quotes a flat fee per burn at `GET /fees` and refuses a burn that pays less
than its floor (§5). Sphere shows the fee before the burn and writes it into
the burn; the vault takes it out of the owner's payout and pays the operator's
account when the batch settles on or before the burn's deadline
(`docs/spec/ZK_BACK3.md` §4). The fee is off until `BRIDGE_RETURN_FEE_AMOUNT`
is set, and until then the operator pays everything and charges nothing. Only
the service can enforce a minimum: the vault and the proof program check only
that the fee does not exceed the amount.

The fee arrives in the bridged asset (USDC on Sepolia), so it reimburses the
operator but does not replace gas: the settlement account still needs native
currency. It is a flat amount, so set it against the gas price and the batch
sizes actually seen. On a pull-payment vault the operator collects it with
`withdraw`.

**Letting the user pay the gas** is possible and needs larger changes.
`fulfillBatch` accepts any sender and the service publishes each proof at
`GET /batches/:id` (at once when it runs without a submitter), so the token
owner can submit the settlement from their own wallet. The docs describe this only as the self-settle fallback that keeps
funds from being stuck (`docs/spec/ZK_BACK3.md` §4 and §13,
`prover/crates/service/README.md` under "S4 submitter"). Making it the normal
path needs a wallet step that submits the proof and a service started without
its relayer, which the container entrypoint and `run-return-service.sh` always
set. Whoever submits a batch pays for every burn in it.
