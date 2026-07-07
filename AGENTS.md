# Unicity Bridge Repository

This repository is the canonical bridge-owned monorepo for the Unicity
external-asset bridge (Tron/EVM <-> Unicity). It owns the bridge protocol
contract, conformance vectors, source-chain contracts, TypeScript bridge
packages, return prover, deployment metadata, and bridge documentation.

External Unicity SDKs and wallet applications remain independent repositories.
During local development they may sit beside this repository, but do not vendor
or commit them here.

## Layout

| Directory | Stack | Role |
|---|---|---|
| `protocol/interop.md` | spec | Normative byte-level cross-stack contract |
| `protocol/vectors/` | Rust + JSON | Reference generator and conformance fixtures |
| `contracts/tron/` | Solidity / Hardhat | Tron/TVM bridge vault and verifier integration |
| `packages/bridge-core/` | TypeScript | Chain-neutral bridge interfaces and wallet boundaries |
| `packages/bridge-plugin-tron-usdt/` | TypeScript | Tron USDT bridge plugin, verifier, wallet adapter, CLI/demo |
| `prover/` | Rust / SP1 | Return relation, host tooling, prover service |
| `deployments/` | JSON | Frozen deployment configs and trust-base metadata |
| `docs/spec/` | Markdown | Engineering specs and design references |
| `docs/dev-plan/` | Markdown | Historical and planning documents |

## Architecture

The bridge moves a real external asset in two directions:

- **Bridge-in (lock -> mint):** the source-chain vault locks the asset and
  commits to the Unicity token id and recipient. The TypeScript plugin verifies
  the lock over source-chain RPC before the wallet counts the token as spendable.
- **Bridge-back / return (burn -> release):** the holder burns the Unicity token
  to a bridge-back reason. The Rust prover turns the burned token and Unicity
  certificates into a proof. The source-chain vault verifies the proof, checks the
  replay accumulator root, and releases the asset.

The hard rule is byte identity across stacks. The contracts, TypeScript packages,
and prover must produce and consume the same encodings and hashes. A `configHash`
the circuit commits must equal the vault's `configHash`; a `nullifier` emitted by
the circuit must equal the accumulator leaf; a `BridgeBackReason` CBOR-encoded by
the wallet must decode field-for-field in the circuit.

`protocol/interop.md` is the normative contract. `protocol/vectors/` is the
machine-checkable form of that contract. Any derivation change bumps
`BRIDGE_PROTO_VERSION`, regenerates vectors, and updates all consumers.

## Commands

Install JavaScript dependencies from the root when working on TS packages or
contracts:

```bash
npm install
npm run build
npm test
```

Conformance vectors:

```bash
npm run vectors
# or:
cd protocol/vectors/gen && cargo run --offline
```

Contracts:

```bash
cd contracts/tron
npm run build
npm test
```

TypeScript bridge packages:

```bash
npm run build -w @unicitylabs/bridge-core
npm test -w @unicitylabs/bridge-plugin-tron-usdt
npx tsx --test packages/bridge-plugin-tron-usdt/test/verifier.test.ts
```

Prover:

```bash
cd prover
cargo test
cargo run -p bridge-return-host -- check-vectors ../protocol/vectors
cargo check -p bridge-return-guest --features sp1 --bin bridge-return-sp1-guest
```

## External Repositories

The following are related but intentionally not owned here:

- `state-transition-sdk-rust`
- `state-transition-sdk-js`
- `sphere-sdk`
- `sphere`
- `wallet-api`
- `unicity-yellowpaper-tex`

Bridge changes in `sphere` and `sphere-sdk` should stay on branches in those
repositories and consume this repo's packages through npm/git pins or local
development links.
