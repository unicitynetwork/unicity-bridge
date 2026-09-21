# Bridging: committed work log

Tracks every commit made for the bridging effort, across the repositories
checked out beside each other. Updated after each commit. "Pushed" is recorded
per row; nothing is pushed until it says so.

Plan reference: [`docs/dev-plan/09-testnet-e2e.md`](docs/dev-plan/09-testnet-e2e.md).
Background: [`BRIDGING_ANALYSIS.md`](BRIDGING_ANALYSIS.md).

## unicity-bridge (this repository)

Base: `main` at `2eb36bb` ("slides").

| Branch | Commit | Date | Plan step | Summary | Pushed |
|---|---|---|---|---|---|
| `docs/bridging-analysis` | `abed67c` | 2026-09-18 | analysis | `BRIDGING_ANALYSIS.md` (theory, trust list, demo divergences, prover soundness problem, actors chapter with connection map) and `docs/dev-plan/09-testnet-e2e.md` (findings, URLs, dependency-ordered plan). | no |
| `feat/sdk3-port` | `7eb60be` | 2026-09-18 | Phase 1 [2] | `bridge-core` and `bridge-plugin-tron-usdt` moved from `state-transition-sdk` 2.0.0-rc to 3.0.1. Verifier, mint/transfer/verify call sites, demos ported; `demo/sdk3.ts` verification stack; `e2e attach` command for the deployed Nile vault; `DEMO.md`. 48/48 tests, build, typecheck, vectors unchanged, offline attack matrix green. | no |
| `feat/sdk3-port` | `96ae51c` | 2026-09-18 | Phase 1 [2] | Demo artifact loader pointed at the pre-restructuring path; now resolves `contracts/tron/artifacts/`. Surfaced by the live run. | no |
| `feat/sphere-plugin` (on `feat/sdk3-port`) | `54dda89` | 2026-09-18 | Phase 1 [3], piece 2 | Bridge as a wallet plugin over the sphere-sdk seams. `bridge-core`: structural wallet contract (`WalletTokenPlugin`, `BridgePayments`) + `mintBridgedToken`, `burnForReturn` (persist-then-acknowledge), `recoverPendingBurns`; own test suite. Plugin: value payload in the wallet's format (`SpherePaymentData`, pinned byte-for-byte against sphere-sdk's encoder), `bridgeTokenPlugin()`, `selfMintVerifier()`, `burnIdentifiers()`; demos on the new format; `interop.md` §2.1, `PLUGIN_ARCHITECTURE.md`, README/DEMO updated. 21 files, +566/−90. bridge-core 5/5, plugin 55/55, attack matrix green. | no |
| `feat/sphere-plugin` | `548f7b3` | 2026-09-21 | Phase 2 step 1 | Prover ported to Rust SDK `v3.0.1` (from the `token-split-2` branch): sdk-ext verifier mirrors the SDK-3 inclusion rules (reference time in the leaf value, deadline check, shard prefix) for anchored mode, v2 transaction/certified-data formats, fixture SMT rebuilt in the new shape. `decode_bridged_payment_data` reads the wallet envelope (tag 39050 v1) and rejects the bare collection; `encode_bridged_payment_data` pinned to the wallet's bytes. Empty burn batches rejected in the guest (§9 hole closed). `BRIDGE_PROTO_VERSION` 2; token vectors regenerated; domain strings and configHash unchanged. 21 files, +368/−134. cargo test 43/43 (4 ignored: 3 old-format live-sample tests, 1 pre-existing), check-vectors ok, SP1 guest binary compiles. vkey not yet computed (no SP1 toolchain / protoc here). | no |
| `feat/sphere-plugin` | `2b30bba` | 2026-09-21 | Phase 2 step 2 (vkey) + step 3 (container) | `prover/Dockerfile` (guest ELF in Succinct's SP1 v6.3.1 image, host + service with the SP1 host SDK, vkey + check-vectors gate, hardhat artifact, node-slim runtime with relayer), root `docker-compose.yml` (service on :8787, precheck mode, SP1 artifacts volume), permissive CORS on the service router. Built and smoke-tested (/health 200 with CORS, preflight 200). Guest vkey `0x0039a542…`, ELF sha256 `ce8b6ddb…`; local record in the gitignored `sp1-vkey.json`. Not done: `sp1_groth16` mode never exercised in the container (needs the 6 GB artifact download and ~16 GB); relayer still Node. | no |
| `feat/sphere-plugin` | `08316b8` | 2026-09-21 | Phase 2 step 2 (deploy) | **v2 vault deployed on Nile**: `TBKJ84417jdxo6j92TxQuYpZdRZGaeZVrv`, tx `6a660031…`, CONFIG_HASH `0xfa77a13a…`, vkey `0x0039a542…`, trust base `0x72a67260…` allow-listed (tx `f22aae9a…`, block 71158544), admin = demo account. `deployments/nile/nile-usdt-v2.json` frozen (config derived by `emit-config`, equal to the chain); `NILE_USDT_BRIDGE` describes v2, `NILE_USDT_BRIDGE_V1` kept for the record; plugin version pin 2 (55/55); entrypoint, compose and run script default to v2; host test checks both freezes against their on-chain hashes. `.env` (gitignored) `TRON_VAULT`/`BRIDGE_VAULT` moved to v2 with v1 kept as a comment. Sphere needs no change: the dev server already serves the rebuilt manifest. Deployer balance 962 TRX before. | no |
| `docs/bridging-analysis` | (latest on branch) | 2026-09-18 | tracking | `BRIDGING_COMMITTED_WORK.md`, this log; its own hash is whatever the latest commit on this branch is. | no |

Live gate for [2] (2026-09-18, not a commit): `e2e attach → lock → mint → transfer → verify`
against the deployed Nile vault `TTKKLyh…` and testnet2. Lock nonce 19
([approve](https://nile.tronscan.org/#/transaction/f05e4fad9c93a9c1ddb08673d25d36612fcb63a6dbcca79ec97513d97b4416f5),
[lock](https://nile.tronscan.org/#/transaction/e72e3d1fc17e27f2779c490e6f978a2fc40e15abe74442e1974f77cbf6685cd5),
block 71070991); token `d7ad6460cbf6be0f06ee9e4ec0ab3773377b2b938a1158b85ae0ce44395b06c3`
minted, transferred, re-verified at 20 confirmations. Depositor account
`THkA8JuurBh19mMamdHCoTXtwETzSAfgaj` (throwaway, key in the gitignored `.env`).
Token blob: `packages/bridge-plugin-tron-usdt/demo/.demo-state.json` (gitignored;
Phase 2 test data, keep it).
Both branches fork from `main` and are independent of each other.

## sphere (`sphere/`, github.com/unicity-sphere/sphere)

Base: `origin/main` at `762d350d` (2026-09-15).

| Branch | Commit | Date | Plan step | Summary | Pushed |
|---|---|---|---|---|---|
| `test/baseline-testnet2` | (none, checkout of `origin/main`) | 2026-09-18 | Phase 1 [1] | Baseline run: `docker compose build && up`, image `sphere-frontend` on `localhost:3010`, testnet2 only. Gate passed: gateway calls 200, transfer completed. No source change was needed. | n/a |
| `feat/bridge-v2` (from `origin/main` `762d350d`) | `ec5966de` | 2026-09-21 | Phase 1 [4] | The bridge as a **wallet module**, not a cherry-pick: `src/modules/registry.ts` globs `src/modules/<name>/module.ts`; the core knows the contract only (token plugins for `Sphere.init`, `describeCoin` for rows, actions with their own screen). `src/modules/bridge/` is assets-in only and is itself pluggable: `assets/<name>/index.ts` per bridgeable asset (`tron-usdt` today), the screen and `bridgeIn.ts` name no chain; mint via bridge-core `mintBridgedToken`. Core touch points: `SphereProvider` (plugins), `L3WalletView` (`ModuleActions`, `describeCoin`), `AssetRow` badge, `vite.config` dedupe of state-transition-sdk. `package.json`: sphere-sdk + both bridge packages as `file:` links (tripwire in `dependency-hygiene.test.ts`); `tronweb` for the dev-key signer. Docs: `docs/WALLET-MODULES.md`. 29 files, +2347/−214. Gate: `tsc -b`, `typecheck:tests`, eslint 0 errors, `vite build`, vitest 1531/1531 (1 skipped tripwire), `npm run dev` serves the module graph with every linked import resolving. The 18 old UI commits stay unused. | no |
| `feat/bridge-v2` | `7ce057af` | 2026-09-21 | Phase 1 [4] | Module screens mounted with the built-in modals (the tab bar bled through); signer availability re-checked while the screen is open (TronLink injects late). | no |
| `feat/bridge-v2` | `47f6c73a` | 2026-09-21 | Phase 1 [4] | Picker: network → asset → form, every step shown even with one option; `BridgeAsset.chain` descriptor (id, family, network name, testnet flag); component test walks the steps. | no |
| `feat/bridge-v2` | `68ab6cc0` | 2026-09-21 | Phase 1 [5] | A module's coin shown as the module says everywhere (`moduleAssetView` / `moduleTokenView` in `useAssets` / `useTokens`), reference price for USDT; long names expand on click; send dialog header wraps. Fixes "Available 10000000 F16348", the $0 column and the 64-hex header. | no |
| `feat/bridge-v2` | `bcbe1356` | 2026-09-21 | Phase 1 [5] | Approval is exactly the deposit amount; max-approve option removed. | no |
| `feat/bridge-v2` | `0120e635` | 2026-09-21 | Phase 3 (UI) | **Assets out.** Direction step in front of the picker; bridge-out form (tokens of the coin, each burned whole, destination validated by the asset); returns list on the first screen with the service's status and the settle tx. `bridgeOut.ts`: burn → record the blob → release the wallet copy → submit; `syncReturns` resubmits unacknowledged or forgotten returns; `recoverBurns` on wallet start. `BridgeAsset.out` (reasonFor / identify / returns) filled by the Tron asset from the plugin's bridge-back surface; service URL from the manifest or `VITE_BRIDGE_RETURN_SERVICE_URL`. 9 new tests. | no |

Live gate for [5] (2026-09-21, not a commit): bridge-in from the local Sphere UI
(`npm run dev`, testnet2, TronLink on Nile with the demo depositor account). Two
deposits of 10 USDT: lock `e6bccb2e…` (block 71153704, 10:17 UTC, approval
reused from `5ae0487a…`) and, after the exact-amount change, approve
`97214259…` + lock `e20e481d…` (block 71154887, 11:16 UTC). Both tokens showed
under Assets and Tokens with the Tron badge, correct decimals and a dollar
value, and were offered for sending. A first attempt at 10:06 was killed by a
hot reload after its approval and before its lock; nothing was locked and the
record was discarded from the UI. Receiver check (2026-09-21, later the same day): one of the bridged tokens was
sent from the wallet's first address to a second derived address (a separate
identity with its own inventory); the receiver accepted it under the strict
20-confirmation verifier and shows it with the Tron badge. **Milestone 1 met.**
Phase 2 (assets out) starts here.

`feat/unicity-bridge` (18 commits, 457 behind `main`) is untouched.

## sphere-sdk (`sphere-sdk/`, github.com/unicity-sphere/sphere-sdk)

Base: `origin/main` at `be75bc9a` (v0.17.3, 2026-09-17).

| Branch | Commit | Date | Plan step | Summary | Pushed |
|---|---|---|---|---|---|
| `feat/token-plugins` | `0300cc97` | 2026-09-18 | Phase 1 [3], piece 1 | Generic token-plugin seams, no bridge code: `TokenPlugin` (mint-reason verifiers by tag) registered via `EngineConfig.plugins` / `SphereInitOptions.plugins`; `mintDataToken` gains `justification` + per-mint verifiers; `ITokenEngine.burn` (BurnPredicate(sha256(reason)), deterministic realization); payments-v2 `mintCustom`, `burn`, `pendingBurns`, `acknowledgeBurn`, both journal-first with crash replay. 24 files, +1156/−58. typecheck, typecheck:tests, lint 0 errors, build, vitest 140 files / 2732 tests. | no |

`feat/unicity-bridge` (9 commits, 208 behind `main`) is untouched. The cherry-pick
approach was dropped in favour of the plugin architecture (see the plan).

## Deployed contracts (Tron Nile testnet)

Live on chain, outside git. The frozen record of each vault is
`deployments/nile/<file>`; `prover/crates/host/tests/nile_config.rs` checks
every freeze against the hash the contract committed on chain.

| Contract | Address | Deployed | Facts | Status |
|---|---|---|---|---|
| **Vault v2** `UnicityBridgeVault` | `TBKJ84417jdxo6j92TxQuYpZdRZGaeZVrv` (EVM `0x0ec4b82f…`) | 2026-09-21, tx `6a66003106eaaafb692a4f18a3078bd92641b2c2025a8be21a6a4e62801f7f5d` | vkey `0x0039a5424014e57caf45d3451053e6c014547837ae09c9eb724aa569389b90d5` (BRIDGE_PROTO_VERSION 2 guest, ELF sha256 `ce8b6ddb…`); CONFIG_HASH `0xfa77a13a6fb24658fa75377b0eef7cf3e92f8caede9ea127afe21be7a036cb1b`; trust base `0x72a67260…` allow-listed in tx `f22aae9acd1a5e65c41740b4145ccc1dea0d963605ac178abe7456b984dd90a5` (block 71158544); admin `THkA8JuurBh19mMamdHCoTXtwETzSAfgaj`; push-payment; freeze `nile-usdt-v2.json` | **active** (wallet manifest, service, env) |
| Vault v1 `UnicityBridgeVault` | `TTKKLyhnRRQ7XV5vsRarV8xWWEvF9225mY` (EVM `0xbe47c0b7…`) | 2026-07-03 | vkey `0x00c34ae0ebb63e86218a754892813f4744b2f6c9ed613c085ea40999b16ce3ad` (BRIDGE_PROTO_VERSION 1 guest); CONFIG_HASH `0x7f376b16b3bff3455f375e7cf30b9d29d2a14332912f0ffb69d78e1b31d5193f`; freeze `nile-usdt.json` | superseded: its key names the old guest, which cannot read SDK-3 tokens; holds ~13 test USDT from the 2026-09-18/21 locks, written off |
| SP1 Groth16 verifier | `TN4nQmnVz3H3zDnN77NQZTAfBpzkEdoeBR` | before 2026-07-03 | stateless, shared by every vault; SP1 v6 circuit | active |
| Test USDT (asset) | `TXYZopYRdj2D9XRtbG411XZZ3kM5VkAeBf` (EVM `0xeca9bc82…`) | Nile's own | non-standard TRC20 (false-returning transfer; the vault's R6 safe-transfer covers it); token type `0x6f2d10d2…`, coin id `0xf1634862…` are derived from it and are the same for v1 and v2 | active |

Deployer for v2: the demo depositor account, funded with test TRX (962 TRX before
the deployment). Nothing ties v2 to v1's keys; settlement on either vault is
permissionless.

## Not committed anywhere

- Running state: the Sphere baseline container (`sphere-frontend`) and Docker
  image; `demo/.demo-state.json` from any e2e run (gitignored).
- Sibling checkouts at this repo's root (`sphere/`, `sphere-sdk/`,
  `state-transition-sdk-js/`, `aggregator-go/`, `unicity-yellowpaper-tex/`)
  are separate repositories and are never committed here.
