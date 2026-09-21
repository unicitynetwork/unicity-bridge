# 09 — Testnet end-to-end plan: real USDT, current Unicity testnet, local Sphere

Status: plan, 2026-09-18. Nothing here has been started. Companion to
[`../../BRIDGING_ANALYSIS.md`](../../BRIDGING_ANALYSIS.md) (what the bridge is)
and [`07-return-service.md`](./07-return-service.md) (the service design).

## Goal

Bridge a real Nile USDT deposit into a Unicity token on the **current** Unicity
testnet, show it in a **locally run Sphere**, then bridge it back through a
**locally run return service**. Everything that is ours runs locally, in Docker
where it makes sense, reachable from the host browser. Tron Nile and Unicity
testnet2 are reached over the internet.

## Findings that shape the plan

Read-only probes on 2026-09-18.

| Fact | Consequence |
|---|---|
| `gateway.testnet2.unicity.network` is alive but **sharded**: a request without `stateId` or `shardId` is rejected. Sphere `main` uses `@unicitylabs/state-transition-sdk` **3.0.1**; `bridge-core`, `bridge-plugin-tron-usdt`, and the Sphere bridge branch pin **2.0.0-rc.68bc1e5**. | Every JS component must move to SDK 3.x before it can talk to testnet. First task. |
| The prover pins `state-transition-sdk-rust` at `branch = "token-split-2"`. That branch **no longer exists** upstream (branches: `main`, `service-time`, `archive/1.0-compat`; tag `v3.0.1`). It builds today only from the local cargo cache. The aggregator changed the inclusion-proof wire format on 2026-08-26 (`aggregator-go/docs/inclusion-proof-wire.md`). | Port the prover to Rust SDK `v3.0.1`. The guest program changes, so the verification key changes, so the vault must be **redeployed (v2)**. |
| Sphere `feat/unicity-bridge` is 18 commits ahead and **457 behind** `origin/main` (merge base 2026-06-30). `main` moved sphere-sdk 0.11 → **0.17.2** and is multi-network (testnet2 + mainnet). | The "rebase" is a re-application onto a very different `main`, in two repositories (Sphere and sphere-sdk). New branches only. |
| The bridge npm packages (`@unicitynetwork/bridge-core`, `@unicitynetwork/bridge-plugin-tron-usdt`, `@unicity-sphere/sphere-sdk@bridge-test`) are **not on the public registry** (404). They were published to GitHub Packages. | Local testing links local builds (`file:` dependencies). No registry tags. |
| The Nile vault `TTKKLyhnRRQ7XV5vsRarV8xWWEvF9225mY` exists (`UnicityBridgeVault`, deployer `TFVJsmN8SE64QsgGxP3wR9S6MsGDCwDuSu`). | The v1 vault is usable for bridge-in tests now. |
| `goggregator-test.unicity.network` (the `BRIDGE_RETURN_SERVICE_URL` in Sphere's compose, and the `/rpc` dev-proxy target in Sphere `main`) does not answer. | Stale. The return service will be ours, at `http://localhost:8787`. |
| The trust base is embedded in sphere-sdk (`assets/trustbase.ts`, `TRUSTBASE_TESTNET2`). On `main` (0.17.3) it has the **same four nodes, epoch 1, quorum 3** as our `bft-trustbase.testnet2.json`. | Testnet2 validators have not rotated. The v1 vault's allow-listed trust-base hash is still current. No file to obtain. |
| Local checkouts now present beside this repo: `sphere/` (bridge branch 18 ahead / 457 behind `main`), `sphere-sdk/` (bridge branch **9 ahead / 208 behind** `main` v0.17.3), `state-transition-sdk-js/` (`main`, 3.0.1). sphere-sdk's bridge commits import nothing from the plugin packages. | The sphere-sdk rebase and the plugin port are independent; both depend only on SDK 3.0.1. |
| The return service has no CORS handling. | Add it; the browser calls the service cross-origin. |

## URLs

| What | URL | Source |
|---|---|---|
| Sphere production | `https://sphere.unicity.network` | Sphere `main` |
| Sphere staging (auto-deployed from `main` via `sphere-infra` to ECS) | `https://sphere.staging.unicity.network` | HTTP 200 on 2026-09-18; the hostname lives in `sphere-infra`, not in the Sphere repo |
| Sphere per-branch preview (GitHub Pages) | `https://unicity-sphere.github.io/sphere/<branch>/`, e.g. `.../sphere/main/` | `.github/workflows/deploy-pages-branch.yml` |
| Unicity testnet2 gateway (sharded) | `https://gateway.testnet2.unicity.network/` | `.env.example`; answered on 2026-09-18 |
| Unicity mainnet gateway (do not use) | `https://gateway.mainnet.unicity.network` | Sphere `main` |
| Staging wallet API / quest API | `https://wallet-api.staging.unicity.network`, `https://quest-api.staging.unicity.network` | Sphere `docker-compose.yml` |
| Tron Nile | `https://nile.trongrid.io` | `.env.example` |
| Nile vault v1 / SP1 verifier / USDT | `TTKKLyhnRRQ7XV5vsRarV8xWWEvF9225mY` / `TN4nQmnVz3H3zDnN77NQZTAfBpzkEdoeBR` / `TXYZopYRdj2D9XRtbG411XZZ3kM5VkAeBf` | `deployments/nile/nile-usdt.json` |

The testnet2 aggregator API key: Sphere `main`'s `.env.example` carries one it
describes as non-secret on testnet2. Confirm that is the one to use before
relying on it.

### Network vs. environment

These are two independent axes in Sphere `main` (`deploy/runtime-config.sh`):

- **Network** is which Unicity chain the tokens live on, `testnet2` or
  `mainnet`, selected inside the app (Settings → Network; `DEFAULT_NETWORK`
  picks the start). Gateways come from sphere-sdk's per-network table. The
  bridge, the Nile vault, and our trust base are bound to **testnet2**.
- **Environment** is which deployment of the app and its backends you use:
  staging (`sphere.staging…`, `wallet-api.staging…`, `quest-api.staging…`) or
  production (`sphere.unicity.network`, `wallet-api.mainnet…`). One image
  serves both; `runtime-config.sh` rewrites the URLs at container start.

A deployment offers mainnet only when `WALLET_API_URL_MAINNET` is set and
`MAINNET_ROLLOUT_ENABLED='true'`. Staging is the pre-production environment
that exposes testnet2 (and mainnet, if configured); it is not "the testnet
environment". The legacy `WALLET_API_URL` alone means testnet2, which is what
Sphere's own `docker-compose.yml` still uses.

**For this plan:** our local Sphere is its own environment, pointed at the
testnet2 network, with `WALLET_API_URL_MAINNET` unset so mainnet cannot be
selected. If the non-bridge features (marketplace, quests) are wanted, point
`SPHERE_API_URL` and `WALLET_API_URL_TESTNET2` at the staging hosts, as
Sphere's compose already does.

## Decisions taken (2026-09-18)

- **Network:** testnet2, the same network a user gets by selecting "testnet"
  in `https://sphere.unicity.network`.
- **Base branch:** Sphere `origin/main` (`762d350d`, 2026-09-15). The deployed
  image (staging and production) is built on every push to `main`
  (`.github/workflows/docker-build.yml`); there are no release branches or
  version tags.
- **First item:** rebase the Sphere bridging branch onto that base, on a new
  branch. Existing branches are not edited.

## Order of work

### Phase 0: prerequisites (no code)

1. Confirm the testnet2 aggregator API key (Sphere `main`'s `.env.example`
   carries one described as non-secret on testnet2).
2. Confirm we hold the **Nile vault admin key** (deployer `TFVJsmN8…`). Not
   needed for bridge-in; needed in Phase 2 to decide reuse vs. redeploy.
3. Read the v1 vault's `PULL_PAYMENTS` and `spentRoot` (two constant calls) so
   we know whether payouts need a `withdraw()` claim. Phase 2 only.
4. Fund a Nile account with test TRX and faucet USDT; install TronLink on Nile
   or pick a private key for `ManagedTronSigner`.

### Phase 1: local Sphere on testnet2 with the bridge in scope

Goal of the phase: `npm run dev` in Sphere, on testnet2, with the bridge code
present and working. The dependency graph:

```
 [1] Sphere main runs locally         [2] plugin + bridge-core     [3] sphere-sdk bridge
     on testnet2, no bridge               on SDK 3.0.1                 hooks on main 0.17.3
     (baseline)                           (this repo)                  (sphere-sdk repo)
          \                                   |                            /
           \                                  |                           /
            +----------------------------> [4] Sphere feat/bridge-v2 <---+
                                              cherry-pick 18 commits onto main,
                                              file: links to [2] and [3], build, run
                                                            |
                                                            v
                                           [5] bridge-in from local Sphere (Milestone 1)
```

[1], [2], [3] have no dependencies on each other and can run in parallel. [4]
needs all three. [5] needs [4] plus Nile funds.

Why the rebase is not step one on its own: the 18 Sphere commits import
`@unicitylabs/sphere-sdk@bridge-test` (0.11 + hooks) and the plugin packages
(SDK 2.0), none of which can talk to the sharded testnet2. Cherry-picking them
onto `main` (sphere-sdk 0.17.3, SDK 3.0.1) produces a branch that cannot build
until [2] and [3] exist. Doing [1] first also proves the toolchain, the API
key, and the gateway before any bridge code is in the way.

**[1] Baseline.** In `sphere/`, check out `origin/main` (no new branch needed
yet). `.env` from `.env.example`: `VITE_AGGREGATOR_API_KEY` (testnet2 key),
`VITE_WALLET_API_URL_TESTNET2` pointed at the staging wallet API, mainnet URL
unset. `npm install`, `npm run dev`. Gate: the wallet opens on testnet2, shows
a balance, and an ordinary transfer succeeds. No bridge code involved.

**[2] Plugin port.** Branch `feat/sdk3-port` in this repository. Move
`packages/bridge-core` and `packages/bridge-plugin-tron-usdt` from
`state-transition-sdk 2.0.0-rc.68bc1e5` to 3.0.1 (the local
`state-transition-sdk-js/` checkout is the reference): sharded aggregator
client, the new certificate format (shard-tree certificate,
`ShardIdMatchesStateIdRule`), the mint-justification extension point,
payment-data format. Gate: `npm test` and `npm run vectors` green; the byte
contract does not move. Optional stronger gate: `demo/e2e.ts` adapted to lock
on the existing vault mints a token on testnet2 without any browser.

**[2] status (2026-09-18): done.** Commits `726316b`, `fdbbbab` on
`feat/sdk3-port`. Offline gates green (48/48 tests, build, typecheck, vectors
unchanged, attack matrix). Live gate passed: 1 USDT locked in the deployed
vault (nonce 19), token `d7ad6460…` minted on testnet2 through the sharded
gateway, transferred, and re-verified by a second owner at 20 confirmations.
`bridge-core` needed no code change. Two findings for later phases: every new
genesis is a VERSION 2 mint transaction (Rust port must parse it), and SDK 3's
`TokenIssuanceVerifierService` is where a per-type issuance rule could live.

**[3] redefined (2026-09-18): plugin architecture instead of cherry-picks.**
Decision: bridge code is isolated in its own packages and pulled in only when a
bridge is configured; sphere-sdk gets generic seams that any token plugin can
use. Two stacked pieces:

- *Piece 1, sphere-sdk `feat/token-plugins` (done, `47ab5008`):* `TokenPlugin`
  (mint-reason verifiers by CBOR tag) registered through
  `Sphere.init({ plugins })`; `mintDataToken` with a genesis `justification` and
  per-mint verifiers; engine `burn` with a reason; payments-v2 `mintCustom`,
  `burn`, `pendingBurns`, `acknowledgeBurn`, journal-first with crash replay.
  Nothing in it names a bridge or Tron. Upstreamable on its own.
- *Piece 2, unicity-bridge `feat/sphere-plugin` (done, `68c59ea`, on
  `feat/sdk3-port`):* `bridge-core` defines the wallet contract structurally
  (`WalletTokenPlugin`, `BridgePayments`) and the composition helpers
  `mintBridgedToken`, `burnForReturn` (persist-then-acknowledge),
  `recoverPendingBurns`; the plugin adds `bridgeTokenPlugin()` for
  `Sphere.init({ plugins })`, `selfMintVerifier()` in the mint request, and
  `burnIdentifiers()` to read a burned blob. Sphere's three call sites will
  target these instead of `sphere.payments.bridgeMint/Burn` in [4].

Decision taken (2026-09-18): **(A)**, framed as "the bridge defines no value
format; it writes and reads the network's, which today is the wallet's". Done in
piece 2 for the TypeScript side; the Rust prover follows in Phase 2 and
`BRIDGE_PROTO_VERSION` bumps then (`interop.md` §2.1). Recommendation to raise
with the SDK and wallet teams: one canonical fungible-value payload for the
network, so no future wallet, bridge or prover hits the same fork. The
background, kept for the record:
The inventory's per-token `assets` are decided by wallet-api (the server decodes
the blob; `applyDelta.added` carries only `{tokenId, key}`), and sphere-sdk `main`
classifies a bare `PaymentAssetCollection` genesis as `'bare_collection'`: stored,
value unreadable, refused for whole-send. The bridge contract currently mandates
bare payment data and the prover rejects the Sphere envelope (`0fbfb2f`). Options:
(A) bridged genesis carries `SpherePaymentData` (tag 39050); the whole wallet
stack reads it natively; the prover decodes the envelope (as the reverted
`8798b6f` did), interop.md + vectors move, BRIDGE_PROTO_VERSION bumps.
(B) keep bare payment data; needs wallet-api to classify bare collections for
registered token types, outside this repo. Recommendation: A.

**[3] original text (superseded) — sphere-sdk hooks.** In `sphere-sdk/`, branch `feat/bridge-v2` from
`origin/main` (v0.17.3). Cherry-pick the 9 bridge commits
`origin/main..origin/feat/unicity-bridge` (`a1eab69e` 2026-06-30 to `f5d6f98e`
2026-07-09), skipping the merge commit `ab3ed379` and the two "publish" chores.
They add `SphereInitOptions.{bridgeJustificationVerifiers, bridges}`,
`Sphere.bridges` / `bridgeForCoin()`, the `bridgeMint` mint path, the burn
path, and the burned-token inventory filter. Gate: sphere-sdk builds and its
tests pass. `feat/unicity-bridge` is not touched.

**[4] Sphere rebase.** In `sphere/`, branch `feat/bridge-v2` from
`origin/main` (`762d350d`). Cherry-pick the 18 commits
`origin/main..feat/unicity-bridge` (`e0c683b3` 2026-06-30 to `6e8c2034`
2026-07-10) in order. In `package.json` replace the `bridge-test` and
`2.0.0-rc` pins with `file:../sphere-sdk`, `file:../packages/bridge-core`,
`file:../packages/bridge-plugin-tron-usdt`. Expected conflict areas:
`package.json`, `src/sdk/SphereProvider.tsx`, the L3 wallet view and
`AssetRow`, the Vite proxy config, and anything touched by the multi-network
work (per-network gateway, `DEFAULT_NETWORK`, SGW subscriptions). Gate:
`npx tsc --noEmit` clean, `npm run dev` shows the wallet on testnet2 with the
"Bridge USDT" entry point. `feat/unicity-bridge` is not touched.

**[4] status (2026-09-21): done, differently.** Commit `bd164c5f` on Sphere
`feat/bridge-v2` (from `origin/main`). The cherry-pick was dropped: the 18 old
commits wired the bridge into `SphereProvider`, the L3 view, `AssetRow` and the
SDK's since-deleted `bridgeMint`, and the requirement became that deleting the
bridge files leaves a wallet without a bridge. So Sphere gained a small module
system (`src/modules/`, discovered by folder glob, contract in `types.ts`) and
the bridge is its first module, assets-in only; bridge-out waits for Phase 2.
The module is pluggable in turn: one folder per bridgeable asset under
`src/modules/bridge/assets/`, the screen and flow chain-agnostic. Wallets that
can sign a deposit: TronLink, plus a dev-only "development key" signer from
`VITE_BRIDGE_DEV_TRON_KEY` (needs `tronweb`, loaded lazily; tree-shaken out of
production). Gate passed: `tsc -b`, tests typecheck, eslint, `vite build`,
vitest 1531/1531, dev server serving the whole linked import graph. Not yet
seen in a browser: the Bridge button under Top Up / Swap / Send, which is [5]'s
first step. Sphere's `.env` (gitignored) carries the staging backends and the
public testnet2 key; `VITE_REQUIRE_WALLET_API` must stay out of it, since vitest
reads `.env` and `walletApi.test.ts` fails with the flag set.
`file:` links resolve through the unicity-bridge working tree, so it must stay on
`feat/sphere-plugin` (the checkout was found on `docs/bridging-analysis` mid-session).

**[5] Bridge-in from local Sphere.** Against the existing v1 vault
(`configHash` in the manifest already equals its `CONFIG_HASH`). approve +
lock on Nile via TronLink or `ManagedTronSigner`, mint on testnet2.

**[5] status (2026-09-21): done from the UI.** Two 10 USDT deposits signed with
TronLink on Nile from the demo account, locks `e6bccb2e…` (block 71153704) and
`e20e481d…` (block 71154887), each minted a wallet-format token that Sphere shows
with the Tron badge and offers for sending. Their blobs are in the wallet-api
inventory of the test wallet. See `BRIDGING_COMMITTED_WORK.md` for the four
follow-up commits the live run surfaced (layering, picker, coin display,
exact-amount approval). Receiver check done the same day: sent to a second
derived address, accepted under the strict verifier at 20 confirmations.
**Milestone 1 met (2026-09-21).**

**Milestone 1:** the token shows in local Sphere with the bridged badge and
passes the in-wallet re-verification at 20 confirmations. Its blob is the test
data for Phase 2.

**Sizing notes (2026-09-18), after reading the code:**

- [2]: all 31 SDK import paths still exist in 3.0.1; 22 of those files changed
  (74 commits between `68bc1e5` and 3.0.1). Concrete changes:
  `IMintJustificationVerifier.verify(tx, nestedTokenCollector)`;
  `MintTransaction.create(networkId, recipient, options)` with an options
  object and a new `expiresAt` (leave unset); mint transaction is now
  **VERSION 2** with an 8th CBOR field, so every new genesis is v2 and the
  Rust prover must parse it in Phase 2; `Token.mint` / `token.verify` take one
  `IVerificationContext`; `AggregatorClient` / `StateTransitionClient` /
  `InclusionProofUtils` / `RootTrustBase` carry the shard-bound certificate;
  `PaymentAssetCollection` enforces canonical asset order (moot for one
  asset). Roughly a few dozen call sites across src, demo, test.
- [3]: 9 commits, 18 files (+715/-31). `main` deleted the old money stack on
  2026-08-05 (`75c35776`, "P11 deletion wave") and replaced it with
  `modules/payments-v2/`. The token-engine and `core/Sphere.ts` pieces
  cherry-pick with conflicts (17-18 `main` commits on each); the payments
  piece (`PaymentsModule.bridgeMint/bridgeBurn`, burned-token inventory
  filter) must be re-implemented on `modules/payments-v2/PaymentsFacade.ts`
  and `inventory/InventoryView.ts`, keeping the method names so Sphere's
  commits still bind. Read `docs/MIGRATION-PAYMENTS-V2.md` first.
- Order: [2] first (mechanical, gives a real SDK-3 token to test [3]'s mint
  path), then [3].

Docker for Sphere (`sphere/Dockerfile`, `docker-compose.yml`) comes after [5]
works in dev mode; it changes packaging, not behaviour.

### Phase 2: return service and v2 vault

Branch: `feat/sdk3-port`, continued.

1. Port the prover workspace to Rust SDK `v3.0.1`; adopt the new
   inclusion-proof wire format. Fix the empty-burns early return
   (`BRIDGING_ANALYSIS.md` section 9) in the same change, since the vkey
   changes anyway. Regenerate vectors, `cargo test`, SP1 execute on the Phase 1
   blob.
   **Step 1 status (2026-09-21): done, `c16860e` on `feat/sphere-plugin`** (the
   tip of the sdk3-port stack). Decisions taken with it:
   - The prover reads the wallet's value payload and nothing else; the bare
     collection is refused. `BRIDGE_PROTO_VERSION` is 2. The five domain
     strings stay `:v1`, so `configHash` does not move and the v2 vault differs
     from v1 by the verifying key only.
   - Anchored (shared-root) verification stays hand-rolled in sdk-ext, mirroring
     the SDK's `verify_inclusion_proof_for` minus the quorum check; the SDK
     offers no against-root entry point. Certified mode goes through the same
     code with the proof's own certificate.
   - Bridge transactions carry no deadline (`expires_at = None`); the aggregator's
     service-assigned deadline is not recorded and not re-checked, as in the SDK.
   - The Rust SDK's `TokenSplit` commits split outputs to the bare asset
     collection, while the split protocol (and the TS SDK) binds the output
     mint's actual payload bytes. Verification in sdk-ext uses the mint's bytes
     (correct for wallet-made splits); the fixture builds its splits with its own
     wallet-style builder. Worth raising with the SDK team: a split builder that
     takes the payload encoder.
   - The three live-sample tests are ignored until a wallet-format burn is
     recorded (Milestone 2 data).
   Not done here: `SP1 execute` on a real blob (no SP1 toolchain on this
   machine; `sp1-sdk` also needs `protoc`). Runs on the prover box with step 2.
   **Step 2, first half (2026-09-21): vkey computed in Docker.** No SP1 toolchain
   goes on the laptop: `prover/Dockerfile` builds the guest ELF inside Succinct's
   own image for SP1 v6.3.1 (the reproducible build), the host and service with
   the SP1 host SDK (protoc + Go 1.24 for the native Groth16 library), derives the
   key from the ELF and runs `check-vectors`; `docker-compose.yml` at the repo
   root runs the service on :8787 (precheck mode by default, CORS open, SP1
   artifacts on a volume). Docker Desktop here has 15.6 GB, enough for the build
   and the key, borderline for a Groth16 proof.
   New guest vkey: `0x0039a5424014e57caf45d3451053e6c014547837ae09c9eb724aa569389b90d5`
   (v1 vault: `0x00c34ae0…`). Deployment is the remaining half: it needs no key
   from v1, any funded Nile account becomes v2's admin, the SP1 verifier contract
   is shared. Not done yet; it is an outward action on the testnet.
   Runtime image built and smoke-tested the same day (`f8d3fc4`): step 3's
   container exists in precheck mode; real proving in it is untested.
   **Step 2 done (2026-09-21, `65b4c68`): v2 vault `TBKJ84417jdxo6j92TxQuYpZdRZGaeZVrv`**,
   CONFIG_HASH `0xfa77a13a…`, trust base allow-listed, frozen in
   `deployments/nile/nile-usdt-v2.json`. Decisions: one active bridge per asset
   (the registry keys on chain + asset and mint-reason tags cannot repeat across
   plugins), so the manifest simply moved to v2 and v1 stays as a record; v1's
   locked test USDT is written off; the demo depositor is v2's admin, since
   nothing ties v2 to v1's keys. Next: bridge fresh USDT in against v2 from
   Sphere, then the burn side (Phase 3 UI or the plugin CLI) for Milestone 2.
2. Compute the new vkey. Deploy **v2 vault** on Nile with
   `contracts/tron/scripts/deploy-nile.js real-vault` (reuses the existing SP1
   verifier contract). Allow-list the current trust base hash. Freeze
   `deployments/nile/nile-usdt-v2.json`; update the manifest `configHash`.
   Tokens minted against v1 cannot return through v2; on testnet, mint fresh
   tokens against v2 rather than migrating.
3. Dockerize the service: Rust binary, Node for `relayer.js`, SP1 artifacts on
   a volume (the Groth16 proving key is about 6 GB). Start in
   `BRIDGE_RETURN_PROVE_MODE=precheck_only`; switch to `sp1_groth16` once the
   pipeline is green. Add CORS to the axum router.

**Milestone 2:** a burned blob from Sphere is accepted, proven, and settled on
the v2 vault; USDT lands on the Tron address.

### Phase 3: full loop from Sphere

**UI status (2026-09-21, `b7938a6d` on Sphere `feat/bridge-v2`): the assets-out
screen exists.** Burn from the wallet, blob recorded before the wallet releases
its copy, hand-off to the return service, status tracking with resubmission.
Not yet exercised live. The return-service container runs from the repo root
(`docker compose up -d return-service`, reads TRON_SK / TRON_VAULT from `.env`)
in precheck mode; a real release needs `BRIDGE_RETURN_PROVE_MODE=sp1_groth16`.

Bridge-in from Sphere → transfer → bridge-out from Sphere → `Released`
observed → balance on Tron. Then the negative cases: replayed blob, tampered
leaf, wrong config.

## Docker topology

```
docker compose (repo root)
  sphere            nginx serving the built app        host :3010
  return-service    Rust binary + Node relayer.js      host :8787
                    volume: SP1 circuit + groth16 pk
                    env: gateway URL, API key, trust base path, TRON_SK,
                         BRIDGE_RETURN_EVENTS_CMD, prove mode
```

- The browser reaches the service at `http://localhost:8787`;
  `BRIDGE_RETURN_SERVICE_URL` is set to that, and the service needs CORS for
  the Sphere origin.
- Testnet2 and Nile are reached over the internet; nothing else runs locally.
- Proving needs 16 GB or more of RAM inside the container. If the Docker host
  cannot give that, run the service natively for the proving step and keep
  Sphere in Docker.

## Decisions needed before starting

1. **Vault:** reuse v1 for Phase 1 and redeploy v2 in Phase 2 (recommended,
   since the vkey must change), or go straight to v2 before Phase 1?
2. **Sphere strategy:** cherry-pick the 18 commits onto `main` (recommended)
   or re-implement the UI against the new SDK from scratch?
3. **Keys:** do we hold the Nile deployer key, and which testnet2 aggregator
   key do we use?
4. **Proving machine:** what RAM does the Docker host have?

## Risks

| Risk | Mitigation |
|---|---|
| SDK 3.x changed token serialization or the justification extension point | Port the plugin first, in isolation, with the conformance vectors as the guard |
| Rust SDK `v3.0.1` changed `Token::verify` inputs or the certificate format | Port against the Phase 1 blob; the aggregator's `inclusion-proof-wire.md` is the reference |
| Validators rotated since epoch 1 | New trust base hash allow-listed on the v2 vault at deploy time |
| Sphere cherry-picks conflict heavily | Cherry-pick the plugin-facing files first (`src/bridge/*`, hooks), UI last |
| Proving too heavy for the Docker host | Native service for proving; `precheck_only` in Docker for everything else |
| v1 vault holds test USDT that cannot return through v2 | Return it under v1 before abandoning, or accept the loss on testnet |
