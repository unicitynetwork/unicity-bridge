# Unicity Bridge

Bridge-owned monorepo for the Unicity external-asset bridge.

```text
protocol/                 normative byte contract + conformance vectors
contracts/tron/            Tron/TVM source-chain contracts and tests
packages/bridge-core/      chain-neutral TypeScript bridge interfaces
packages/bridge-plugin-tron-usdt/
                           Tron USDT verifier, wallet adapter, CLI/demo
prover/                    Rust/SP1 return prover workspace
bft-trustbase.testnet2.json Unicity testnet2 trust base
deployments/               frozen deployment metadata
docs/spec/                 engineering specs
docs/dev-plan/             historical plans and status notes
```

Copy `.env.example` to `.env` for local deploy, relayer, demo, and return-service
commands. The example keeps deployment-specific public constants aligned with the
Nile USDT deployment and uses placeholders for signer/API secrets.

The compatibility boundary is `protocol/interop.md` plus
`protocol/vectors/`. Contracts, TypeScript, and prover code must reproduce those
vectors exactly.

Front-end is provided by specific branches of
- [github.com/unicity-sphere/sphere](https://github.com/unicity-sphere/sphere/tree/feat/unicity-bridge)
- [github.com/unicity-sphere/sphere-sdk](https://github.com/unicity-sphere/sphere-sdk/tree/feat/unicity-bridge)

Useful commands:

```bash
npm install
npm run build
npm test
npm run vectors

cd prover && cargo test
```
