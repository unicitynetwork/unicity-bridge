# Bridge development plan

Implementation plan for the two-way external-asset bridge between an external
settlement chain (Tron/EVM) and Unicity.

**Reference spec:** [`../ZK_BACK3.md`](../ZK_BACK3.md) is the engineering source of
truth for the *return* path; [`unicity-yellowpaper-tex/appendix-bridging.tex`](../../../unicity-yellowpaper-tex/appendix-bridging.tex)
is its formal counterpart (kept ~in sync). Where the two diverge, **ZK_BACK3.md
wins for development** and the divergence is tracked in
[`00-interop-contract.md`](./00-interop-contract.md).

This README is the map. The detail lives in four sibling documents:

| Doc | Owns | Stack |
|---|---|---|
| [`00-interop-contract.md`](./00-interop-contract.md) | The frozen byte-level contract every component must obey, + the conformance-vector mechanism | spec + fixtures |
| [`01-source-chain-contracts.md`](./01-source-chain-contracts.md) | Lock-in vault, return vault, on-chain proof verification, settlement | Solidity / Tron (TVM) + EVM, Hardhat |
| [`02-ts-sdk-and-wallet.md`](./02-ts-sdk-and-wallet.md) | Bridge-in justification verifier (plugin), bridge-back burn construction, wallet/sphere integration | TypeScript |
| [`03-prover-service.md`](./03-prover-service.md) | The zk relation (guest circuit) and the off-chain prover pipeline | Rust + SP1 (zkVM) |
| [`04-deployment.md`](./04-deployment.md) | Nile testnet deployment runbook (verifier + vault) and the on-chain proof smoke | TronWeb / Hardhat |
| [`05-cost-analysis.md`](./05-cost-analysis.md) | Tron mainnet tx-fee + proving-fee cost model (grounded in measured energy) | analysis |
| [`integration.md`](./integration.md) | Design: wallet bridge UX (Sphere) + the return sequencing/proving service; the resolved cross-cutting decisions | design |
| [`06-wallet-bridge-integration.md`](./06-wallet-bridge-integration.md) | Dev plan: surface bridge in/out in the real Sphere UI (TronLink-first, plugin-owned logic) | TypeScript / React |
| [`07-return-service.md`](./07-return-service.md) | Dev plan: the off-chain S1–S4 pipeline as one all-Rust, self-hosted, disposable service | Rust + SP1 |

## What the bridge does

**Bridge-in (lock → mint).** A holder locks an external asset in the source-chain
vault; the vault commits to the exact Unicity `tokenId` and recipient. A bridged
token is minted on Unicity whose genesis *backing reason* points at that lock. A
wallet receiving the token re-verifies the lock over a source-chain RPC before
counting it as spendable. **Status: built** (`UnicityLock.sol`,
`bridge-plugin-tron-usdt/`); this plan hardens and freezes it.

**Bridge-back / return (burn → release).** A holder burns the Unicity token to a
`BurnPredicate(H(reasonBytes))` whose reason bytes — a canonical `BridgeBackReason`
carried in the burn's auxiliary data — fix the release destination. A
*trustless prover* turns the burned token + Unicity certificates into one
succinct proof; the source-chain vault verifies the proof, checks one replay
accumulator root, and releases the locked asset. No committee, no operator, no
light client, no challenge window. **Status: greenfield**; this is the bulk of
the work.

## Component map and the single hard rule

```
            BRIDGE-IN (built)                         BRIDGE-BACK (to build)
  ┌───────────────┐   lock event   ┌────────────┐   burn blob   ┌──────────────┐
  │ Source vault  │ ─────────────► │  TS SDK /  │ ────────────► │   Prover     │
  │ (UnicityLock) │                │   wallet   │               │  (Rust/SP1)  │
  └───────┬───────┘                └─────┬──────┘               └──────┬───────┘
          │ custody                       │ mint w/ backing reason     │ Groth16 proof
          │                               ▼                            │ + public values
          │                        ┌────────────┐                      │
          │   release ◄────────────│ Source vault│◄─────────────────────┘
          └───────────────────────►│(ReturnVault)│   fulfillBatch(publicValues, proof, …)
                                    └────────────┘
```

The three boxes are built by three teams on three stacks. **The one hard rule is
that all three must produce/consume byte-identical encodings and hashes.** A
`configHash` the circuit commits must equal the one the vault stores; a
`nullifier` the circuit puts in a leaf must equal the one the off-chain
accumulator inserts and the one the vault emits; a `BridgeBackReason` the wallet
CBOR-encodes must decode field-for-field in the circuit. That contract is
[`00-interop-contract.md`](./00-interop-contract.md), and it is frozen **before**
the component teams diverge.

## How three independent tracks stay in sync

Independence is bought with one shared artifact and one gate:

1. **One normative spec** — `00-interop-contract.md`. No component invents an
   encoding, domain separator, or hash; it cites a clause here. Changes to it are
   versioned (`BRIDGE_PROTO_VERSION`) and ripple to all three via the vectors.

2. **Cross-stack conformance vectors** — a versioned `bridge-vectors/` fixture set
   (input → expected bytes/hash/root) generated once by a designated reference
   implementation and consumed as a test input by the Solidity suite, the TS
   suite, and the Rust suite. This is the same pattern the repo already uses for
   Rust↔JS SDK parity (`state-transition-sdk-rust` "Regenerating cross-SDK
   fixtures"). A component is "in sync" iff its CI passes the current vectors.
   See `00-interop-contract.md` §"Conformance vectors".

3. **CI gate** — each repo fails the build on a vector mismatch or a
   `BRIDGE_PROTO_VERSION` skew. The end-to-end devnet test (M2+) is the integration
   backstop, but the vectors catch divergence per-commit without a full proving run.

## Milestone roadmap (synchronization points)

Milestones are cross-cutting: each is a state in which all three tracks
interoperate. Per-component phases (in each plan) are tagged with the milestone
they serve.

| # | Goal | Contracts | TS | Prover | Exit criterion |
|---|---|---|---|---|---|
| **M0** | Freeze the contract | scaffold fresh ZK_BACK3 vault; measure Tron Groth16 energy + 80 ms dry-run | review reason/derivations | review SDK reuse + circuit shape | `00-interop-contract.md` v1 + `bridge-vectors` v1 published; all repos pin `BRIDGE_PROTO_VERSION=1` |
| **M1** | Bridge-in frozen | vault `lock()` stores `lockDigest`; audit | finalize `bridge-plugin-tron-usdt`, manifest plumbing | (n/a) | bridge-in vectors green in TS + Solidity; mint→receive e2e on Nile |
| **M2** | Return path, B=1, *mocked* proof | `ReturnVault` with a mock verifier + accumulator root + settlement | burn construction + nullifier/leaf derivation lib | circuit relation for one burn, run in SP1 *execute* (no proof) | one burn settles end-to-end on a local devnet using a mock proof; all vectors green |
| **M3** | Return path, B=1, *real* proof | real Groth16 verifier wired (EVM + Tron) | — | SP1 prove + Groth16 wrap; anchored inclusion + non-membership | a real proof settles one burn on testnet |
| **M4** | Batching B>1 | unchanged vault interface | sequencer client helpers | ordered accumulator insertions; sequencer + accumulator-builder services | a batch of N burns settles in one tx on testnet |
| **M5** | Production | timelock/governance, audit, mainnet cfg | unknown-asset discovery UX | recursion/aggregation; optional settlement aggregation | audited; mainnet `configHash` + `vkey` + trust-base allow-list published |

## Current state (grounded in the repo)

- **`contracts/tron/`** — Hardhat (solc 0.8.24). The existing `UnicityLock.sol`
  return model (`unlock`/`withdrawn`) is **superseded and not reused**; we build
  one fresh vault implementing ZK_BACK3 (lock-in with `lockDigest` + accumulator
  return). Only its TRC20 safe-transfer (no-return USDT) and reentrancy-guard
  patterns are lifted — see `01-source-chain-contracts.md`.
- **`bridge-plugin-tron-usdt/`** — a complete TS `IMintJustificationVerifier`
  (`TronUsdtMintJustificationVerifier`) plus config/identifier derivations and a
  Tron RPC client. This is the bridge-in verifier; the return path adds new TS.
- **`state-transition-sdk-rust/`** — `no_std`/zkVM-ready. Has `Token::verify`,
  per-transition inclusion-proof verification, `BurnPredicate`, RSMST split
  (value conservation), `MintJustificationRegistry`. **Lacks**: anchored-mode
  inclusion (one shared certificate for many transitions), a nullifier
  accumulator with non-membership, and the structural backing verifier. These
  are the prover track's SDK extensions.
- **`state-transition-sdk-js/`** — mirror SDK; source of the plugin extension
  point and `BurnPredicate`.
- **`docs/bridge/`** —  `ZK_BACK3.md`, `MINT_REASON.md`, `BRIDGE_BACK.md`,
  `PLUGIN_ARCHITECTURE.md`, `TRUST_MODELS.md`, `OPTIMISTIC_UNLOCK.md`.

## Decisions (fixed)

Resolved in `00-interop-contract.md`; recorded here so they are not lost:

1. **On-chain hash policy** — **keccak256/ABI** for vault-recomputed commitments
   (`configHash`, `lockDigest`, `returnRoot`, `lockRefRoot`, `domainTag`);
   **SHA-256/CBOR** for Unicity-internal values (`nullifier`, accumulator,
   `trustBaseHash`). This is the efficient split: native `keccak256` is cheaper
   on-chain than the SHA-256 precompile, and the SDK-matching values are forced to
   SHA-256 but never recomputed on-chain (`00` §1).
2. **Nullifier shape** — **nested `burnTransitionId`** (ZK_BACK3); the tex adopts
   it.
3. **Vault topology** — **one greenfield vault per ZK_BACK3** (lock-in +
   accumulator return in a single contract). No backwards compatibility with the
   old `UnicityLock` return model.
4. **lockDigest provenance** — **store the digest at lock time**.
5. **Proof system** — **SP1 (Groth16-wrapped STARK)**. bn254 pairing works on Tron
   (precompiles `0x06/0x07/0x08`, same alt_bn128 params as Ethereum); the open
   items are *energy budgeting* and the *~80 ms `triggerconstantcontract` dry-run
   CPU limit*, both mitigated because SP1's Groth16 wrap exposes a single public
   input (the public-values digest). See `01` and `03`.
6. **v1 return scope** — anchored verification is **time-independent predicates
   only** (signature/burn/split); time-dependent predicates (timelocks/HTLCs) are
   out of scope until the relation carries authenticated original validation time
   or per-transition certs. The burn reason is **self-contained**: `reasonBytes`
   (canonical `BridgeBackReason`) live in the terminal burn's auxiliary data, bound
   by `BurnPredicate(H(reasonBytes))` (`00` §4, §8).
