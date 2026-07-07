# Unicity Bridge

Bridge-owned monorepo for the Unicity external-asset bridge.

```text
protocol/                 normative byte contract + conformance vectors
contracts/tron/            Tron/TVM source-chain contracts and tests
packages/bridge-core/      chain-neutral TypeScript bridge interfaces
packages/bridge-plugin-tron-usdt/
                           Tron USDT verifier, wallet adapter, CLI/demo
prover/                    Rust/SP1 return prover workspace
deployments/               frozen deployment and trust-base metadata
docs/spec/                 engineering specs
docs/dev-plan/             historical plans and status notes
```

The compatibility boundary is `protocol/interop.md` plus
`protocol/vectors/`. Contracts, TypeScript, and prover code must reproduce those
vectors exactly.

Useful commands:

```bash
npm install
npm run build
npm test
npm run vectors

cd prover && cargo test
```
