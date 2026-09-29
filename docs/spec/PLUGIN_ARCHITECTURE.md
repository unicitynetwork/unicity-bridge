# Bridge plugin architecture

How per-asset bridged-token validation plugs into the Unicity SDKs, and how a
wallet that meets an unknown bridged asset obtains the code to validate it.

## The extension point already exists

The state-transition SDK validates a token's mint-reason through a
**tag-dispatched registry**:

- `MintJustificationVerifierService.register(verifier)` — keyed by the
  justification's CBOR tag
  ([source](../../state-transition-sdk-js/src/transaction/verification/MintJustificationVerifierService.ts)).
- `IMintJustificationVerifier` — `{ get tag(): bigint; verify(tx, service) }`
  ([source](../../state-transition-sdk-js/src/transaction/verification/IMintJustificationVerifier.ts)).
- `Token.verify()` runs it on the genesis; `sphere-sdk`'s `SphereTokenEngine`
  calls `token.verify(...)` on every receive.

A bridge plugin is therefore just an `IMintJustificationVerifier` registered into
that service. **No change to the core SDK is required.** This is the same shape
as the SDK's own `SplitMintJustificationVerifier`.

## Logic vs. configuration (kept separate)

**Logic (code, per asset family):** one `IMintJustificationVerifier`
implementation. For Tron TRC20 it lives in the standalone package
`bridge-plugin/`. It is pure validation logic: decode justification →
RPC checks → binding checks. It depends only on `@unicitylabs/state-transition-sdk`
(types) and `fetch` (Tron HTTP API is plain JSON — no `tronweb`), so it runs in
browser and Node ≥ 22 alike and stays out of the core SDK.

**Configuration (data, per asset):** distributable JSON.

```jsonc
{
  "asset": "tron-usdt",
  "cborTag": 1330002,
  "tokenTypeHex": "…32 bytes…",      // == SHA256("unicity-bridge:tron:<chainId>:<assetHex>")
  "coinIdHex":   "…32 bytes…",
  "decimals": 6,
  "chainId": "0x2b6653dc",            // Tron mainnet (Nile: 0xcd8690dc)
  "lockContract": "41…",              // canonical UnicityLock (trust anchor)
  "assetContract": "41…",             // USDT TRC20 (trust anchor)
  "confirmations": 20,
  "rpcUrls": ["https://api.trongrid.io"]
}
```

The contract/asset/chain fields are **trust anchors** — they decide which Tron
deployment is authoritative. The verifier rejects any justification that does not
match them, so shipping the right config is part of the security model.

## How a wallet handles an unknown asset

1. A token arrives with `tokenType = T` the wallet has no verifier for.
   `MintJustificationVerifierService.verify` returns FAIL
   `"Unsupported mint justification tag"` (or the verifier rejects the type).
2. The wallet looks up `T` in a **bridge plugin manifest** (a registry JSON shipped
   alongside `unicity-ids.<network>.json`, keyed by `tokenTypeHex`). The manifest
   entry names the plugin package + version and carries the config above.
3. The wallet loads the matching verifier (bundled, or fetched as a versioned,
   integrity-pinned module), constructs it with the config, and registers it.
4. Re-verify. The token now validates (or is correctly rejected).

Until the wallet trusts a plugin for `T`, the token is shown as
**"unverified asset"** and is not counted as spendable balance — degrade safe,
never assume validity.

## Where it is wired

- **Standalone plugin:** `bridge-plugin/` exports
  `createTronUsdtBridgePlugin(config)` → `{ tokenTypeHex, coinIdHex, cborTag, verifier }`.
- **sphere-sdk (generic seams, no bridge code):** `TokenPlugin`
  `{ id, mintJustificationVerifiers, tokenIssuancePolicies }` registered via `Sphere.init({ plugins })`
  / `EngineConfig.plugins` next to the SDK's split verifier; `mintDataToken`
  with a genesis `justification` and per-mint verifiers; `ITokenEngine.burn`
  with a reason; payments-v2 `mintCustom`, `burn`, `pendingBurns`,
  `acknowledgeBurn` (journal-first, crash-replayed). `token-engine/` stays
  browser/IPFS/Nostr-free; the plugin uses only `fetch`.
- **bridge-core (chain-neutral):** the structural wallet contract
  (`WalletTokenPlugin`, `BridgePayments`) and the composition helpers
  `mintBridgedToken`, `burnForReturn`, `recoverPendingBurns`. It never imports
  the wallet SDK; a sphere-sdk `PaymentsV2` satisfies `BridgePayments` as is.
- **App → engine:** the app loads manifests, builds `bridgeTokenPlugin(loaded)`
  per asset into `Sphere.init({ plugins })`, and runs bridge-in / bridge-out
  through the bridge-core helpers over `sphere.payments`.
- **Mandatory backing:** each plugin also registers a `BridgedTokenIssuancePolicy`
  for its token type. A genesis of that type must carry the lock reason or a
  split reason, whose burned source the SDK checks under the same policy, so a
  token of the bridged type minted without a lock fails verification. The policy
  also claims the bridged coin id for that type, so the wallet counts the coin
  only in verified tokens of the bridged type and shows any other holding of it
  as unverified. A vault listed in a manifest's `replacedVaults` keeps its lock
  verifier, so tokens locked there still verify after a redeploy.

## Adding another bridged asset later

A bridged asset's identity is its token type and coin id, both derived from the
chain family, the chain id and the asset contract (`deriveTokenType`,
`deriveCoinId`). The vault is not part of it. That decides what each addition
needs:

| Adding | New token type and coin id | Bridge code | Wallet |
|---|---|---|---|
| A new vault for an asset already bridged (a redeploy) | No | None: the manifest names the new vault and lists the old one in `replacedVaults`, which keeps verifying | The updated manifest |
| Another asset on a supported chain | Yes | None: a new manifest | An asset entry |
| A chain of a supported family (an Ethereum L2, Tron mainnet) | Yes | None: a manifest with that chain's id and RPC | An asset entry with the chain name |
| A new chain family (for example Solana) | Yes | A `ChainFamilyAdapter` in `packages/bridge-plugin/src/<family>/` (chain reference, address normalization, `SourceChainRpc` and `ConstantCaller` over its node, presentation), a `SourceSigner` for its wallets, and a manifest variant in the `BridgeManifest` union | An asset folder and its wallet connectors |

Every bridged asset uses the lock reason tag 1330002. A wallet registers one
merged plugin (`mergeBridgeTokenPlugins`) whose verifier dispatches on the
lock's chain id and vault, next to one issuance policy per bridged token type.
sphere-sdk needs no change for any row above. The vault is the same Solidity as
long as the family runs the EVM.

A wallet trusts exactly the vaults in the manifests it ships, one active vault
per asset and chain plus the replaced ones. A token whose lock names any other
chain or vault fails verification ("No bridge verifies chain ... vault ..."), and
a token of a bridged type without a lock or split reason fails the type's
issuance policy. Because an asset's identity does not include the vault, tokens
locked in two different vaults for the same asset are the same asset in the
wallet; a wallet that trusts one party's vault and not another's lists only the
vault it trusts.
