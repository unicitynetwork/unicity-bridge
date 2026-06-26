# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What this workspace is

A **multi-repo integration workspace** for the Unicity external-asset bridge
(Tron/EVM ↔ Unicity). It is not a monorepo: the `br` git repo owns only a few
directories; the rest are **independent git repos vendored as siblings**, each
with its own history, toolchain, CI, and (some) its own `CLAUDE.md`.

| Directory | Git repo | Stack | Role |
|---|---|---|---|
| `docs/`, `contracts/`, `bridge-plugin-tron-usdt/`, `bridge-vectors/` | **this (`br`)** | mixed | the bridge integration code + design |
| `state-transition-sdk-rust/` | own repo | Rust (`no_std`/zkVM) | core token protocol SDK; basis for the return circuit |
| `state-transition-sdk-js/` | own repo | TS | core token protocol SDK (mirror of the Rust one) |
| `sphere-sdk/` | own repo | TS | wallet SDK (part of a separate "wallet-api program" — see its `CLAUDE.md`) |
| `sphere/` | own repo | React/TS | wallet app |
| `wallet-api/` | own repo | REST API; TS | custodial-blob wallet backend for Unicity tokens |
| `unicity-yellowpaper-tex/` | own repo | LaTeX | formal spec; `appendix-bridging.tex` is the bridge's formal counterpart |

**Commit to the repo that owns the files.** From `br` you can only commit
`docs/contracts/bridge-plugin-tron-usdt/bridge-vectors`; changes inside the
sibling repos are committed in those repos. `git status` in `br` shows the
sibling repos as untracked directories — that is expected, do not try to add them.
When working inside a sibling dir, its own `CLAUDE.md`/`README` and workflow apply
(e.g. `sphere-sdk` and `state-transition-sdk-js` branch off `feat/wallet-api-integration`,
not `main`).

No backwards compatiblity needed. Bridging is greenfield project.

## The bridge architecture (big picture)

The bridge moves a real external asset (USDT on Tron is the first instance) in two
directions. The three implementation tracks are deliberately independent (separate
stacks), tied together only by a byte-level contract (next section).

- **Bridge-in (lock → mint) — built.** `contracts/tron/UnicityLock.sol` locks the
  asset and commits to the exact Unicity `tokenId` + recipient; a bridged token is
  minted on Unicity whose genesis *backing reason* points at that lock.
  `bridge-plugin-tron-usdt/` is a wallet-side `IMintJustificationVerifier` that
  re-checks the lock over a Tron RPC before counting the token as spendable
  (registered via the SDK's `MintJustificationVerifierService` — see
  `docs/bridge/PLUGIN_ARCHITECTURE.md`).
- **Bridge-back / return (burn → release) — in development, greenfield.** A holder
  burns the token to a `BurnPredicate(BridgeBackReason)`; a trustless prover turns
  the burned token + Unicity certificates into one SNARK; a source-chain vault
  verifies the proof, checks one replay-accumulator root, and releases the asset.
  No committee/operator/light-client. Spec: `docs/bridge/ZK_BACK3.md`.

The return path's three tracks and where they live:
`contracts/` (Solidity vault + on-chain Groth16 verify) ·
`state-transition-sdk-rust/` + a future SP1 `prover/` (the zk relation) ·
`bridge-plugin-tron-usdt/` + the TS SDKs (burn construction, verifier plugin).
The full plan is `docs/bridge/dev-plan/` (start at its `README.md`).

### Design-doc hierarchy (read these before touching bridge code)

- `docs/bridge/ZK_BACK3.md` — **the engineering reference for the return path.**
  Supersedes `ZK_BACK.md` and `ZK_BACK2.md` (kept only for history).
- `docs/bridge/dev-plan/` — the implementation plan: `00-interop-contract.md`
  (the byte-level cross-stack contract), then `01`/`02`/`03` per component.
- `unicity-yellowpaper-tex/appendix-bridging.tex` — the formal version; where it
  diverges from `ZK_BACK3.md`, **`ZK_BACK3.md` wins for development** (divergences
  are tracked in `00-interop-contract.md` §1, §5).

## The interop spine — do not let the stacks drift

The one hard rule: the contracts, the TS SDK, and the prover must produce/consume
**byte-identical** encodings and hashes (a `configHash` the circuit commits must
equal the one the vault stores; a `nullifier` the circuit emits must equal the one
the off-chain accumulator inserts; a `BridgeBackReason` the wallet CBOR-encodes
must decode field-for-field in the circuit).

- **`bridge-vectors/`** is the machine-checkable form of that contract: a
  zero-dependency Rust **reference generator** (`bridge-vectors/gen`) emits
  `config/lock/reason/nullifier/public` fixtures (input → expected bytes/hash).
  Every component's CI must reproduce them. `BRIDGE_PROTO_VERSION` (in
  `bridge-vectors/VERSION`) is pinned by all consumers; a change to any derivation
  bumps it, regenerates, and forces re-pin. `accumulator/` and `token/` are stubs
  until the Rust SDK extensions (E1–E3 in `dev-plan/03`) land.
- **Hash policy (fixed, `00` §1):** `keccak256`/`abi.encode` for values the vault
  recomputes on-chain (`configHash`, `lockDigest`, `returnRoot`, `lockRefRoot`,
  `domainTag`, `PublicValues`); **SHA-256 over deterministic CBOR** for
  Unicity-internal values (`nullifier`, `burnTransitionId`, accumulator,
  `tokenType`/`coinId`, `recipientCommitment`).
- **The three SDKs are binary-compatible** (`state-transition-sdk-{rust,js}` and a
  Java one) over SHA-256/deterministic-CBOR. Any protocol-encoding change must stay
  consistent across them; cross-SDK fixtures already exist (rust README:
  "Regenerating cross-SDK fixtures").

## Commands

TS repos need Node ≥ 22 and a per-repo `npm install`.

**bridge-vectors** (the conformance generator):
```bash
cd bridge-vectors/gen && cargo run --offline   # regenerate ../<group>/*.json; self-tests SHA-256/Keccak at startup
```

**state-transition-sdk-rust** (and the return circuit's verification core):
```bash
cargo test                                          # full host suite
cargo test --no-default-features --features alloc   # zkVM/WASM verification core only (what the guest links)
cargo test <name_substring>                         # a single test
cargo run --example mint --features http            # live aggregator example (config in e2e/.env)
```

**contracts/tron** (Hardhat is EVM; deploy to Tron via tronbox/TronWeb):
```bash
cd contracts/tron && npm install
npm run build      # hardhat compile (solc 0.8.24)
npm test           # hardhat test
```

**bridge-plugin-tron-usdt** (TS, `tsx` test runner):
```bash
cd bridge-plugin-tron-usdt && npm install
npm run build                          # tsc
npm test                               # all tests
npx tsx --test test/verifier.test.ts   # a single test file
```

**Vendored SDK/app repos** — use their own `CLAUDE.md`/`README`; quick refs:
`state-transition-sdk-js` → `npm run build|lint|test`, single test
`npx jest <path> -t "desc"`; `sphere-sdk` → `npm run build`, `npm run test:run`,
`npm run typecheck`; `sphere` → `npm run dev|build|test:run`.
