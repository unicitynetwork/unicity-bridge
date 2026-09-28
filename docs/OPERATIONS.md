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
| Return service (`bridge-return-service`) | a container you operate | nothing durable; proof bundles on disk | none: it can only sequence and prove |
| Unicity aggregator and validators | the Unicity network | certificates for every token transition | the validator set the vault allow-lists |

Outside dependencies at run time: a Tron node (TronGrid by default) for locks,
receipts and settlement; the Unicity gateway for certificates; Succinct's
public artifact bucket, once, for the Groth16 proving key (§5).

Nobody in this list can move locked funds except through a valid proof. The
prover is untrusted: anyone may run one, and a wrong proof fails on chain.

## 2. What to provision

- **Service host.** Proving is single-worker and needs about 16 GB of memory
  for the duration of one proof (about an hour of CPU on a 12-core machine;
  see `docs/dev-plan/04-deployment.md` for the measured runs). Precheck alone
  needs a fraction of that. 8 GB of disk for the artifacts and bundles.
- **Network.** Outbound HTTPS to the Tron node, the Unicity gateway and the
  artifact bucket. Inbound HTTPS from wallets on the service port (8787 in the
  container). The service speaks plain HTTP; put TLS in front of it.
- **Settlement account.** A source-chain account with enough native currency
  for gas. One settlement costs about 287,000 energy on Tron (verify plus
  transfer); price it with `docs/dev-plan/05-cost-analysis.md`.
- **The wallet.** Sphere's image needs the service URL as a runtime setting
  (today it is a build-time variable, `VITE_BRIDGE_RETURN_SERVICE_URL`; wire it
  through `deploy/runtime-config.sh` like the other URLs before a production
  build).

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
| `BRIDGE_RETURN_BATCH_TARGET`, `BRIDGE_RETURN_MAX_WAIT_SECS` | `1`, `60` | close a batch at this many burns or after this long |
| `SP1_GUEST_ELF` | `/app/sp1/bridge-return-sp1-guest` | the program whose key the vault holds |
| `BRIDGE_RETURN_PROOF_DIR` | `/data/proofs` | proof bundles |
| `TRON_SK`, `TRON_VAULT`, `TRON_RPC_URL` | none, none, Nile | settlement and chain sync, through the relayer |
| `BRIDGE_RETURN_SUBMIT_CMD`, `BRIDGE_RETURN_EVENTS_CMD` | the Node relayer | settlement and event scan hooks; replaceable by any program with the same stdin/stdout contract |
| `BRIDGE_RETURN_BIND` | `0.0.0.0:8787` | listen address |

## 5. Running the service

```
docker compose up -d return-service        # from the repository root; reads .env
docker logs -f bridge-return-service
curl localhost:8787/health
```

Start in `precheck_only`. In that mode every burn is fully verified against
the trust base and the deployment, queued and batched, and nothing is proven.
Switch to `sp1_groth16` once the pipeline is confirmed. The first proof
downloads the Groth16 circuit and proving key, about 5.9 GB, into the
`sp1-artifacts` volume; keep the volume across restarts or every restart pays
the download. Never set `SP1_CIRCUIT_MODE=dev`: it selects a private artifact
bucket and a key the vault does not accept.

Proofs run one at a time. A batch is closed by `BRIDGE_RETURN_BATCH_TARGET`
burns or `BRIDGE_RETURN_MAX_WAIT_SECS`, whichever comes first; a larger target
amortises the hour of proving over more returns and makes each user wait
longer.

Before proving, the service rebuilds the vault's accumulator from the on-chain
settlement events (`BRIDGE_RETURN_EVENTS_CMD`) and checks it against the live
root. `/accumulator` reports `synced: true` when they agree. A proof built on a
stale root is rejected by the vault with `vault: stale root`; the service then
rebases and proves again (§8).

Settlement goes through `BRIDGE_RETURN_SUBMIT_CMD` with the bundle on stdin.
The default is `contracts/tron/scripts/relayer.js settle --stdin`, which needs
the settlement key and the vault address. The all-Rust submitter the plan
calls for is not built; until it is, the container carries Node for this.

## 6. Where the money-critical state lives

- **The burned blob is the claim.** Once a token is burned, the only thing
  that can release the funds is a proof over that blob. The wallet writes the
  blob to its own records before it releases the wallet SDK's copy, and only
  then sends it to the service. Anyone holding the blob can resubmit it; the
  service is idempotent on the burn's nullifier.
- **Those records are in the user's browser**, keyed by wallet identity, in
  local storage. Deleting the wallet deletes them. The service keeps nothing
  durable, so a burn whose blob is lost on the user's side is lost unless the
  service already produced a bundle for it. For production, persist the intake
  (a database behind `POST /returns`) so the service is a second copy.
- **Proof bundles** land in `BRIDGE_RETURN_PROOF_DIR`. A bundle is public data;
  anyone may submit it, and the wallet can fetch it from `/batches/:id`.
- **The accumulator** is reconstructable from the vault's `Released` and
  `BatchFulfilled` events alone. Nothing off chain needs a backup for it.
- **The lock digests** are per-vault state on chain. Value locked in a vault
  can only ever leave through that vault (§9).

## 7. Monitoring

`GET /health` returns `status`, `queueDepth`, `activeBatch`, `batchTarget`,
`maxWaitMs`, `proveMode`, `chainSync`. `GET /accumulator` returns `synced`,
`spentRoot`, `spentCount`. Logs are structured (`RUST_LOG`, default info).

Worth alerting on:

- `/health` unreachable, or `proveMode` not what you configured.
- `/accumulator` `synced: false` for more than a few minutes: the event scan
  or the Tron node is failing, and nothing will settle.
- `queueDepth` growing with `activeBatch` unchanged for longer than a proof
  takes: proving is stuck or out of memory.
- A return in `proving` for more than two hours, or in `submitted` for more
  than a few minutes.
- Settlement account balance below a few settlements' worth of gas.
- On chain: every `BatchFulfilled` should match a bundle the service wrote.

## 8. Failure modes

| Symptom | Cause | What to do |
|---|---|---|
| Wallet shows a return as `burned` that never becomes `queued` | service unreachable | fix the service; the wallet resubmits on its own |
| Service refuses a burn with a config-hash mismatch | the token was minted against another vault (a v1 token after the v2 redeploy) | nothing to do here; only that vault's program can release it. The wallet does not offer such tokens for a burn; a blob that reaches the service anyway is refused as final |
| Return `failed` with a non-recoverable message | the burn will never be accepted (wrong config, malformed reason) | the funds stay on Unicity as a burned token; investigate before telling the user |
| Return `failed` with a recoverable message (`submission_failed`, `proving_failed`, `chain_rejected`) | settlement or proving hit a transient fault | the wallet resubmits the blob after a minute, or at once from the row's retry button, and the service queues it again; fix the cause (gas, memory, node) meanwhile |
| Service restarted, wallet's `returnId` unknown | the service holds nothing durable | the wallet detects the 404 and resubmits the blob |
| Settlement reverts with `vault: stale root` | another batch settled first, or the event scan lagged | the service rebases and proves again; costs another proof |
| Settlement reverts with `vault: trust base not allowed` | the validator set changed and the new hash is not allow-listed | admin allow-lists it (§9); proofs under the old set still settle if that hash remains allowed |
| A recipient's transfer reverts and takes the batch with it | push payments, a hostile or blocked recipient | drop the leaf and rebuild; a deployment expecting this uses pull payments (`PULL_PAYMENTS`, immutable, chosen at deploy) |
| First proof fails with a truncated proving key | interrupted download | delete the artifact volume and let it download again; verify the size against `docs/dev-plan/03-status.md` |
| Proof out of memory | under 16 GB available | more memory; there is no smaller mode |

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

1. Build the program in `prover/Dockerfile` (the guest stage is the
   reproducible build), derive the key, and record the program hash.
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

**Service upgrade.** Rebuild the image; the key must not change unless a vault
redeploy is intended, so compare `/app/sp1/vkey.txt` in the new image with the
deployment file before starting it against a live vault.

## 10. Known gaps before real value

- The admin key has no time-lock (§3).
- The service holds nothing durable; the users' browsers are the only copy of
  unsettled burns (§6).
- The API is open: CORS is permissive and there is no rate limiting. Every
  route is permissionless by design, and a submitted burn costs the submitter
  their own token, but precheck is CPU that anyone can spend.
- Settlement runs through a Node script, not the planned Rust submitter.
- Milestone 2 has not run: no burn has been proven with the current program
  and settled on the v2 vault. The three prover tests that use a live sample
  are ignored until one exists.
- The wallet's service URL is a build-time setting.
- The three wallet packages are linked by path, not published (see
  `sphere/docs/WALLET-MODULES.md`).
- The wallet's development-key signer is compiled out of production builds;
  the demo account's key sits in the repository's `.env` and must not reach a
  production host.
