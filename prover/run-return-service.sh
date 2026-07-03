#!/bin/bash
set -euo pipefail
export BRIDGE_RETURN_BIND=127.0.0.1:8787
export BRIDGE_DEPLOYMENT_CONFIG="$HOME/br/prover/deployment/nile-usdt.json"
export TRUST_BASE_PATH="$HOME/br/prover/deployment/bft-trustbase.testnet2.json"
export BRIDGE_RETURN_PROVE_MODE=sp1_groth16
export SP1_GUEST_ELF="$HOME/br/prover/target/elf-compilation/riscv64im-succinct-zkvm-elf/release/bridge-return-sp1-guest"
export SP1_PROVER=cpu
export SP1_CIRCUIT_MODE=release
export BRIDGE_RETURN_SUBMIT_CMD="PATH=\"$HOME/relayer-node/bin:\$PATH\" node $HOME/br/contracts/tron/scripts/relayer.js settle --stdin"
# S2/S3 chain sync: emit the vault's settlement log so proofs chain onto the
# vault's current spentRoot (else fulfillBatch reverts with "vault: stale root").
export BRIDGE_RETURN_EVENTS_CMD="PATH=\"$HOME/relayer-node/bin:\$PATH\" node $HOME/br/contracts/tron/scripts/relayer.js events"
export BRIDGE_RETURN_MAX_WAIT_SECS=60
export BRIDGE_RETURN_BATCH_TARGET=1
export RUST_LOG=bridge_return_service=info,tower_http=info
exec "$HOME/br/prover/target/release/bridge-return-service"
