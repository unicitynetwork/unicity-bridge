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
| `docs/bridging-analysis` | (latest on branch) | 2026-09-18 | tracking | `BRIDGING_COMMITTED_WORK.md`, this log; its own hash is whatever the latest commit on this branch is. | no |

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
| (not started) | | | Phase 1 [3] | | |

`feat/unicity-bridge` (9 commits, 208 behind `main`) is untouched.

## Not committed anywhere

- Running state: the Sphere baseline container (`sphere-frontend`) and Docker
  image; `demo/.demo-state.json` from any e2e run (gitignored).
- Sibling checkouts at this repo's root (`sphere/`, `sphere-sdk/`,
  `state-transition-sdk-js/`, `aggregator-go/`, `unicity-yellowpaper-tex/`)
  are separate repositories and are never committed here.
