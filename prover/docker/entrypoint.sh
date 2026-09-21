#!/bin/sh
set -eu
: "${BRIDGE_RETURN_BIND:=0.0.0.0:8787}"
: "${BRIDGE_DEPLOYMENT_CONFIG:=/app/deployments/nile/nile-usdt.json}"
: "${TRUST_BASE_PATH:=/app/bft-trustbase.testnet2.json}"
: "${BRIDGE_RETURN_PROVE_MODE:=precheck_only}"
: "${SP1_GUEST_ELF:=/app/sp1/bridge-return-sp1-guest}"
: "${SP1_PROVER:=cpu}"
: "${SP1_CIRCUIT_MODE:=release}"
: "${BRIDGE_RETURN_PROOF_DIR:=/data/proofs}"
: "${BRIDGE_RETURN_MAX_WAIT_SECS:=60}"
: "${BRIDGE_RETURN_BATCH_TARGET:=1}"
: "${RUST_LOG:=bridge_return_service=info,tower_http=info}"
export BRIDGE_RETURN_BIND BRIDGE_DEPLOYMENT_CONFIG TRUST_BASE_PATH BRIDGE_RETURN_PROVE_MODE \
  SP1_GUEST_ELF SP1_PROVER SP1_CIRCUIT_MODE BRIDGE_RETURN_PROOF_DIR BRIDGE_RETURN_MAX_WAIT_SECS \
  BRIDGE_RETURN_BATCH_TARGET RUST_LOG
{
  echo "TRON_SK=${TRON_SK:-}"
  echo "TRON_VAULT=${TRON_VAULT:-}"
  echo "TRON_RPC_URL=${TRON_RPC_URL:-https://nile.trongrid.io}"
} > /app/.env
export BRIDGE_HOST_BIN=/app/bin/bridge-return-host
: "${BRIDGE_RETURN_SUBMIT_CMD:=node /app/contracts/tron/scripts/relayer.js settle --stdin}"
: "${BRIDGE_RETURN_EVENTS_CMD:=node /app/contracts/tron/scripts/relayer.js events}"
export BRIDGE_RETURN_SUBMIT_CMD BRIDGE_RETURN_EVENTS_CMD
mkdir -p "$BRIDGE_RETURN_PROOF_DIR"
echo "bridge-return-service: mode=$BRIDGE_RETURN_PROVE_MODE elf=$SP1_GUEST_ELF vkey=$(cat /app/sp1/vkey.txt 2>/dev/null || echo unknown)"
exec /app/bin/bridge-return-service "$@"
