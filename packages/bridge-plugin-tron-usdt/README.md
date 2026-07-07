# @unicitylabs/bridge-plugin-tron-usdt

Unicity bridge plugin that validates **bridged USDT-on-Tron** tokens. It is an
`IMintJustificationVerifier` for the Unicity state-transition SDK: every
recipient of a bridged token re-checks the token's **mint reason** (a
self-contained Tron lock proof) against a Tron RPC node. There is no trusted
bridge operator.

See the design docs:
- [`../../docs/spec/MINT_REASON.md`](../../docs/spec/MINT_REASON.md) — proof format + verification rule
- [`../../docs/spec/PLUGIN_ARCHITECTURE.md`](../../docs/spec/PLUGIN_ARCHITECTURE.md) — how plugins plug in
- [`../../docs/spec/ZK_BACK3.md`](../../docs/spec/ZK_BACK3.md) — returning to Tron

## How security works

Minting on Unicity is permissionless (the minter key is derived from the
`tokenId`), so all bridge security comes from this verifier. The Tron
`UnicityLock` contract commits each deposit to the exact Unicity `tokenId` and to
`recipientCommitment = SHA256(recipient predicate)`. A deposit can therefore fund
exactly one token, owned only by the designated recipient:

| Attack | Rejected because |
|---|---|
| Mint without a real lock | RPC finds no matching Lock event |
| Replay a lock for a second token | event.unicityTokenId ≠ this token's id |
| Inflate value vs. locked amount | token value ≠ event amount |
| Steal/front-run a lock | event.recipientCommitment ≠ H(recipient) |
| Point at a rogue lock contract | trust-anchor (chain/contract/asset) mismatch |
| Use an unconfirmed lock | confirmations < threshold |

## Usage

```ts
import { createTronUsdtBridgePlugin, tronMainnetUsdtConfig } from '@unicitylabs/bridge-plugin-tron-usdt';
import { MintJustificationVerifierService } from '@unicitylabs/state-transition-sdk/lib/transaction/verification/MintJustificationVerifierService.js';

const plugin = createTronUsdtBridgePlugin(
  tronMainnetUsdtConfig('TYourDeployedUnicityLockAddress'),
  // { rpc, extractAmount }  // optional: inject a TronRpc and a value extractor
);

const service = new MintJustificationVerifierService();
service.register(plugin.verifier); // dispatched by CBOR tag 1330002

// plugin.tokenTypeHex / plugin.coinIdHex identify the bridged asset.
```

In `sphere-sdk`, the plugin is registered in `token-engine/factory.ts` via
`EngineConfig.bridgePlugins`, with `extractAmount` backed by
`decodeSpherePaymentData` so the token's declared value is checked too.

## CLI

```bash
npm run cli demo          # offline security demo (mock Tron RPC)
# or after build:
node lib/cli/main.js demo

# verify a real serialized CertifiedMintTransaction against a live node:
node lib/cli/main.js verify --token <hex> --lock <addr> --asset <addr> \
  --chain mainnet --rpc https://api.trongrid.io [--api-key <key>]
```

The `demo` exits non-zero if the valid token is rejected or any attack is
accepted. Sample output:

```
✔  [OK  ] Valid bridged mint (lock finalized, bound to this token+recipient)
✔  [FAIL] Attack: inflate token value above locked amount
✔  [FAIL] Attack: tamper justification amount
✔  [FAIL] Attack: replay lock for a different token id
✔  [FAIL] Attack: steal lock by swapping recipient
✔  [FAIL] Attack: forged lock contract emits the event
✔  [FAIL] Attack: spend lock before finality (insufficient confirmations)
All checks behaved as expected: valid token accepted, every attack rejected.
```

## Develop

```bash
npm install      # from the repository root; links local workspaces
npm run typecheck
npm test         # node:test via tsx
npm run build
```
