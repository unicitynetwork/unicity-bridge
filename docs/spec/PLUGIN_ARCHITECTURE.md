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
`bridge-plugin-tron-usdt/`. It is pure validation logic: decode justification →
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

- **Standalone plugin:** `bridge-plugin-tron-usdt/` exports
  `createTronUsdtBridgePlugin(config)` → `{ tokenTypeHex, coinIdHex, cborTag, verifier }`.
- **sphere-sdk:** `token-engine/factory.ts` builds the
  `MintJustificationVerifierService` (today it registers
  `SplitMintJustificationVerifier`). `EngineConfig.bridgePlugins` is registered
  there too, right after. `token-engine/` stays browser/IPFS/Nostr-free; the
  plugin uses only `fetch`.
- **App → engine:** plumbing the manifest/config from the app into
  `EngineConfig.bridgePlugins` (and unknown-asset discovery) is the deferred UI
  phase; the factory hook + an integration test land now.

## Adding another bridged asset later

- Same chain, same family (e.g. Tron USDC): new config (its own `cborTag`,
  `tokenTypeHex`, `assetContract`), reuse `bridge-plugin-tron-usdt` code.
- New chain (e.g. an EVM L2): new plugin package implementing
  `IMintJustificationVerifier` with that chain's RPC + event decoding, its own
  tag, and a `lock`-style contract committing to `{tokenId, recipientCommitment}`.
