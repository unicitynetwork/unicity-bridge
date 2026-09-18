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

`feat/unicity-bridge` (18 commits, 457 behind `main`) is untouched.

## sphere-sdk (`sphere-sdk/`, github.com/unicity-sphere/sphere-sdk)

Base: `origin/main` at `be75bc9a` (v0.17.3, 2026-09-17).

| Branch | Commit | Date | Plan step | Summary | Pushed |
|---|---|---|---|---|---|
| `feat/token-plugins` | `0300cc97` | 2026-09-18 | Phase 1 [3], piece 1 | Generic token-plugin seams, no bridge code: `TokenPlugin` (mint-reason verifiers by tag) registered via `EngineConfig.plugins` / `SphereInitOptions.plugins`; `mintDataToken` gains `justification` + per-mint verifiers; `ITokenEngine.burn` (BurnPredicate(sha256(reason)), deterministic realization); payments-v2 `mintCustom`, `burn`, `pendingBurns`, `acknowledgeBurn`, both journal-first with crash replay. 24 files, +1156/−58. typecheck, typecheck:tests, lint 0 errors, build, vitest 140 files / 2732 tests. | no |

`feat/unicity-bridge` (9 commits, 208 behind `main`) is untouched. The cherry-pick
approach was dropped in favour of the plugin architecture (see the plan).

## Not committed anywhere

- Running state: the Sphere baseline container (`sphere-frontend`) and Docker
  image; `demo/.demo-state.json` from any e2e run (gitignored).
- Sibling checkouts at this repo's root (`sphere/`, `sphere-sdk/`,
  `state-transition-sdk-js/`, `aggregator-go/`, `unicity-yellowpaper-tex/`)
  are separate repositories and are never committed here.
