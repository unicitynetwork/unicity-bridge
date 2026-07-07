#!/usr/bin/env bash
#
# Real end-to-end bridge demo, run as one script: Tron Nile -> Unicity testnet2.
# See ../DEMO.md for prerequisites. Requires TRON_PRIVATE_KEY (a funded Nile
# account) in the environment.
#
set -euo pipefail

cd "$(dirname "$0")/.."

if [[ -z "${TRON_PRIVATE_KEY:-}" ]]; then
  echo "TRON_PRIVATE_KEY is required (a Nile-testnet account funded with test TRX)." >&2
  echo "Get test TRX: https://nileex.io/join/getJoinPage" >&2
  exit 1
fi

run() {
  echo
  echo "=================================================================="
  echo "  \$ npm run e2e $1"
  echo "=================================================================="
  npm run --silent e2e "$1"
}

run deploy     # 1. deploy MockTRC20 + UnicityLock to Tron Nile
run lock       # 2. approve + lock USDT, bound to a Unicity tokenId + recipient
run mint       # 3. mint immediately — minter trusts its own lock (in a block)
run transfer   # 4. transfer the token to a second Unicity owner
run verify     # 5. receiver re-verifies, enforcing K confirmations (retries to finality)

echo
echo "✔ End-to-end demo complete. See demo/.demo-state.json for all artifacts."
