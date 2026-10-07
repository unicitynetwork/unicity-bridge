# @unicitylabs/bridge-plugin

Verifies bridged tokens against the lock that backs them on their source chain,
and drives bridge-in and bridge-out for a wallet. It implements the contracts in
`@unicitylabs/bridge-core` for two chain families, Tron and Ethereum-family
(`eip155`) chains, and is not tied to any one asset: a plugin instance is one
`(chain, vault, asset)` triple, and the bridged token type and coin id derive
from that triple. The live deployment is USDC on Ethereum Sepolia; a USDT on
Tron Nile deployment exists but is disabled, since Tron cannot settle a return
(see `docs/OPERATIONS.md`).

Design docs:
- [`../../docs/spec/MINT_REASON.md`](../../docs/spec/MINT_REASON.md), the mint reason format and the verification rule
- [`../../docs/spec/PLUGIN_ARCHITECTURE.md`](../../docs/spec/PLUGIN_ARCHITECTURE.md), how plugins plug into the wallet SDK
- [`../../docs/spec/ZK_BACK3.md`](../../docs/spec/ZK_BACK3.md), returning to the source chain
- [`../../docs/dev-plan/09-ethereum-sepolia.md`](../../docs/dev-plan/09-ethereum-sepolia.md), the Sepolia deployment runbook

## How security works

Minting on Unicity is permissionless (the minter key is derived from the
`tokenId`), so all bridge security comes from verification on the receiving
side. The vault's `Lock` event commits each deposit to the exact Unicity
`tokenId` and to `recipientCommitment = SHA256(recipient predicate)`. A deposit
can therefore fund exactly one token, owned only by the designated recipient.
Every recipient of a bridged token re-checks the token's mint reason (a
self-contained lock proof) against an RPC node of the source chain. There is no
trusted bridge operator.

| Attack | Rejected because |
|---|---|
| Mint without a real lock | RPC finds no matching Lock event |
| Mint a token of the bridged type with no reason at all | the type's issuance policy requires a lock or split reason |
| Replay a lock for a second token | event.unicityTokenId differs from this token's id |
| Inflate value above the locked amount | token value differs from event amount |
| Steal or front-run a lock | event.recipientCommitment differs from H(recipient) |
| Point at a rogue vault | trust anchor (chain, vault, asset) mismatch |
| Use an unconfirmed lock | confirmations below the threshold |

## Usage

A deployment is described by a `BridgeManifest`: chain family and id, vault,
asset contract, decimals, confirmations, RPC and return-service URLs, and the
frozen `configHash`, token type and coin id of the deployment. `loadBridges`
re-derives the last three and refuses a manifest that does not describe the
vault it names. Built-in manifests cover the known deployments.

```ts
import { bridgeTokenPlugin, loadBridges, SEPOLIA_USDC_BRIDGE } from '@unicitylabs/bridge-plugin/wallet';

const [bridge] = loadBridges(SEPOLIA_USDC_BRIDGE);

// bridge.plugin.tokenTypeHex and bridge.plugin.coinIdHex identify the bridged asset.
// bridge.plugin.verifier is the strict lock verifier, dispatched by CBOR tag 1330002.
const tokenPlugin = bridgeTokenPlugin(bridge);
```

A plugin can also be built from a bare `BridgeAssetConfig` when no manifest
exists yet:

```ts
import { createBridgePlugin, SEPOLIA_CHAIN_ID, SEPOLIA_USDC } from '@unicitylabs/bridge-plugin';

const plugin = createBridgePlugin({
  family: 'eip155',
  chainId: SEPOLIA_CHAIN_ID,
  lockContract: '0xYourDeployedVault',
  assetContract: SEPOLIA_USDC,
  decimals: 6,
  rpcUrl: 'https://ethereum-sepolia-rpc.publicnode.com',
});
```

### In a wallet

The wallet registers one token plugin with the Sphere SDK through its generic
plugin interface, `Sphere.init({ plugins })`; the wallet SDK has no
bridge-specific code. Several bridged assets share the lock reason tag, so their
plugins are merged into one dispatching plugin with `mergeBridgeTokenPlugins`.
Each `WalletTokenPlugin` carries:

- the mint-reason verifiers, so a received bridged token is checked against its lock;
- a `BridgedTokenIssuancePolicy` for the bridged token type, so a genesis of that
  type without a lock or split reason fails verification and the bridged coin id
  counts only inside verified tokens of that type;
- `replacedVaults` from the manifest, whose locks still verify tokens of the same
  type but take no new deposits.

Bridge-in and bridge-out are composed over the wallet's generic `mintCustom` and
`burn` by `@unicitylabs/bridge-core` (`mintBridgedToken`, `burnForReturn`,
`recoverPendingBurns`). `createSourceAdapter(bridge, wallet, rpc)` gives the
chain-specific side: it prepares the deposit steps (approve, lock), decodes the
confirmed Lock event into a `CommitInfo` that names the deposit it was made for
(nonce, position, amount, token id and recipient commitment), and builds the mint
request. `mintedAgainst` tells which vault a held token can be returned through.
The token's declared value is read with `decodeBridgePaymentData`, the wallet's
value format.

RPC clients for both families are exported from the package root
(`EvmJsonRpcClient`, `TronHttpRpcClient`); the browser signers (`InjectedEvmSigner`
over any EIP-1193 provider, `TronLinkSigner`), the adapter signers and the explorer
presentation live under `@unicitylabs/bridge-plugin/wallet`. `evmWallets()` lists the
Ethereum wallets on the page: those announcing themselves through EIP-6963, by name and
icon, or the legacy `window.ethereum` as "Browser wallet" when nothing announced itself.
`findLockTxid()` finds the transaction that locked a deposit from the vault's `Lock` events for
the signer since the deposit started (a log read, nothing is sent), for a wallet that lost track
of a lock it asked the user to sign. `queryBalance()` reads what an account holds of the asset,
so a deposit above it is refused before anything is signed.

## CLI

The CLI exercises the Tron verifier only.

```bash
npm run cli demo          # offline security demo (mock Tron RPC)
# or after build:
node lib/cli/main.js demo

# verify a serialized CertifiedMintTransaction against a live Tron node:
node lib/cli/main.js verify --token <hex> --lock <addr> --rpc <url> \
  [--asset <addr>] [--chain mainnet|nile] [--api-key <key>] [--confirmations N]
```

The `demo` exits non-zero if the valid token is rejected or any attack is
accepted.

## Develop

```bash
npm install      # from the repository root; links local workspaces
npm run typecheck
npm test         # node:test via tsx
npm run build
```

`bridge-core` must be built before this package (`npm run build -w @unicitylabs/bridge-core`),
since the workspace link resolves its types from `lib/`.
