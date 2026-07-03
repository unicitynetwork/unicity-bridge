# 08 — Wallet abstraction, bridge-in friction, and multi-chain readiness

**Scope:** de-friction bridge-in (fewer TronLink confirmations), make the wallet
layer wallet-agnostic (official `tronwallet-adapter` + WalletConnect), and remove
Tron-specific assumptions so future EVM chains / stablecoins are additive rather
than a fork. Builds on [`06-wallet-bridge-integration.md`](./06-wallet-bridge-integration.md)
(§A1.1 lock→mint, §A1.3 `TronSigner`) and the live Nile deployment
(`NILE_USDT_BRIDGE` in `bridge-plugin-tron-usdt/src/wallet/manifests.ts`).

**Motivation (review findings, all confirmed against code):**

1. Allowance-skipping is documented but never implemented — `runBridgeIn` always
   sent `approve` then `lock`, and `TronHttpRpcClient` had no read path at all.
2. `TronLinkSigner.getAddress()` called `tron_requestAccounts` on **every** call.
3. `getChainId()` returned the manifest's expected id, not the wallet's real
   network, and was never called — the wrong-network guard was fictitious.
4. Approve and lock were broadcast back-to-back; `sendCall` resolves on
   **broadcast**, not inclusion.
5. The token salt is random and persisted only to `localStorage`, whose write
   swallows failures; the `Lock` event carries the derived `tokenId`, not the
   salt, so chain history alone cannot recover a stranded mint.
6. `_safeTransferFrom` ignores all return data (incl. explicit `false`).
7. No WalletConnect / adapter dependency exists yet.
8. The abstraction is Tron-specific throughout (`TronSigner`/`TronCall`,
   hardcoded `"tron"` in `deriveTokenType`, Nile explorer URLs + `T…` validation
   in the modal, hardcoded Unicity `networkId = 4`, a CBOR justification tag
   **per asset**).

## Architecture direction — three boundaries

Sphere must execute **opaque, adapter-produced steps**. It must *not* contain an
ostensibly generic `if allowance then approve` flow — that hardcodes the ERC-20
approve/lock authorization model and will not fit EIP-3009/EIP-2612 or a
single-signature deposit. Split the boundary three ways:

```
Sphere / useBridgeIn / BridgeModal      chain-neutral: runs opaque Step[]; shows previews
  │  BridgeSourceAdapter   prepareDeposit()->Step[] · decodeLock · validateAddress · explorerUrl
  │  ChainWallet           connect · getAccount · getNetwork · signAndSend · on(change)
  │  ChainClient           readCall · getReceipt · getLogs · getNowBlock
  ▼
tronwallet-adapter · WalletConnect · injected EVM providers · node HTTP
```

- **`ChainWallet`** — connect, account/network, sign/send, wallet events. No reads.
- **`ChainClient`** — node reads: constant calls, receipts, logs, tip. No signing.
- **`BridgeSourceAdapter`** — turns "deposit `amount` for `recipientCommitment`"
  into an ordered list of opaque `Step`s (each a "sign this" or "wait for this"),
  decodes the lock event, validates addresses, and builds explorer links.

`Step` is the unit Sphere runs blindly: `{ kind: 'sign', build(): tx } |
{ kind: 'await-receipt', txid } | …`. An ERC-20 asset yields
`[approve?, awaitApprove?, lock]`; an EIP-3009 asset yields a single
`[transferWithAuthorization]`; Sphere's loop is identical for both.

`BridgeManifest` becomes a **discriminated union** keyed on chain family, with the
chain identified by a **string/hex reference** (CAIP-2 style, e.g.
`tron:0xcd8690dc`, `eip155:1`) — *not* a JavaScript `number` as the generic id.
Tron's numeric chainId lives only inside the Tron variant.

The interface is defined with the *real* capabilities the correct flow needs
(real `getNetwork`, `getReceipt`, `readCall`, wallet events) so swapping in the
official adapter (Phase 3) and adding EVM (Phase 5) touch only the adapter, never
Sphere.

---

## Phase 0 — Neutral wallet interface (enabling)

Reshape the signer surface to what the correct flow needs, without changing
vendor internals. `TronSigner` keeps its name for now but gains
`connect()`/`getNetwork()`; `getAddress()`/`getNetwork()` read the wallet's
**live** state (no stale cache) so pre-signature guards are meaningful;
`TronHttpRpcClient` gains `triggerConstantContract` (the missing read path).

## Phase 1 — Correctness & friction (ships on TronLink) — *in progress*

- **1.1 Allowance skip (#1):** `triggerConstantContract` + `queryAllowance`; in
  `runBridgeIn`, query `allowance(owner, vault)` and drop `approve` when it
  covers `amount`. Keep the one-time large approve so repeats are a single
  `lock` prompt; reconsider default-unlimited exposure. *(landed)*

- **1.2 Connection lifecycle (#2):** `connect()` prompts once; `getAddress()` is
  silent and reads the **current** account live. Subscribe to
  account/network/disconnect via `onChange`. *(landed)*

- **1.3 Real network guard (#3):** `getNetwork()` derives the wallet's actual
  chainId from its connected node's genesis block. This is the correct method —
  TRON defines `eth_chainId` as the last four bytes of the genesis hash
  ([ref](https://developers.tron.network/reference/eth_chainid)). *(landed)*

- **1.4 Guard-before-every-signature + terminal tx handling (#4, #5):** the
  once-before-approval check is **insufficient** — the user can switch
  account/network while the approval receipt is pending. Require:
  - **Pin `{account, network}` at flow start.**
  - **Re-read and compare live immediately before approval and immediately
    before lock**; abort on any drift (and on `accountsChanged` / `chainChanged`
    / `disconnect`).
  - **Do not assume `switchChain` exists** — TRON documents it as wallet-specific
    ([ref](https://developers.tron.network/docs/tronwallet-adapter)). Offer it
    where advertised; otherwise block with instructions.
  - **Fail fast on a mined-but-reverted transaction** — both approve *and* lock.
    A reverted lock must throw immediately, not wait out the 120 s timeout.
  - **Reject broadcast failures** — a `sendRawTransaction` result flagged failed
    (no/`false` result) throws rather than returning a phantom txid.
  - **Terminal pending-record handling:** a failure with **no confirmed on-chain
    lock** removes the pending intent (nothing is locked; retry starts fresh with
    a new salt). Only a **confirmed** lock keeps the record for mint recovery. A
    lock that broadcast but is later found reverted (interactive or on resume) is
    marked `failed`, not left as a zombie `pending mint`. *(landing with this
    revision)*

- **1.5 Fail-closed recovery + salt on-chain (#5):** two independent parts.
  - **Fail-closed persist (no contract change):** the pre-lock persist must be
    fail-closed — `runBridgeIn` refuses to broadcast `lock` if the intent did not
    durably persist. Ships in Phase 1.
  - **Salt on-chain (contract change):** add the 32-byte salt to `lock` calldata
    and a **new** `Lock` event so a mint is recoverable from chain history — but
    "recoverable" is only real with the whole recovery path:
    - a **scanner** that discovers the identity's `Lock` events (by
      `recipientCommitment` and/or `from`) when local storage *and* txid are gone,
      with **pagination/cursors** and **duplicate handling**;
    - validation that `TokenId.fromSalt(unicityNetworkId, event.salt) ==
      event.unicityTokenId`;
    - validation that `event.recipientCommitment` belongs to the **active Sphere
      identity** (else it's someone else's deposit);
    - a **defined new event signature** with updated decoder (`decodeLockEvent`)
      and the mint justification/verifier.
    - **Vault-enforced salt↔tokenId (preferred):** the vault should enforce
      `tokenId == derive(salt)` so malformed deposits can't be committed. If the
      exact Unicity derivation in Solidity is undesirable, **state explicitly**
      that only the official client enforces it and that malformed third-party
      deposits may be unrecoverable.
  - *Contract change → rides a vault redeploy; see “Protocol versioning” below.*

- **1.6 Multi-asset-safe transfers (#6):** the current draft was contradictory —
  "explicit `false` must fail" cannot coexist with the live non-standard Nile
  USDT that returns `false` on **success**. Resolve by **choosing one**:
  - **(a)** drop that non-standard token and require **empty-or-`true`** return
    data (clean, standard TRC20/ERC-20 only); or
  - **(b)** an **integrity-pinned per-asset return policy** where a `false`
    "success" is accepted **only** when exact balance deltas prove the transfer.

  Either way, bind actual movement:
  - **Deposit:** vault asset balance must increase by **exactly** `amount`.
  - **Release:** vault balance must decrease by exactly `amount` **and** the
    recipient balance increase by exactly `amount`.
  - **Explicitly reject** fee-on-transfer and rebasing assets (delta ≠ amount ⇒
    revert).
  - **Test** a token that returns `false` *without* transferring — it must fail.
  - *Contract change → same redeploy.*

1.1–1.4 (incl. the fail-closed persist in 1.5) need **no** contract change and
ship on the current TronLink path. The salt-on-chain half of 1.5 and 1.6 batch
into one vault redeploy — see the coordinated cut below.

## Phase 2 — Tests (the absent §W2 tests)

`runBridgeIn` against a fake `ChainWallet`/`ChainClient`: allowance-covers ⇒ no
approve; allowance-short ⇒ approve+wait+lock; approve revert ⇒ lock never sent;
lock revert ⇒ fails fast + record removed; account/network drift before lock ⇒
abort; persist failure ⇒ refuses to lock; **crash recovery must be proven from
chain history** — the test drives the **scanner** against a fake log source and
reconstructs the mint from discovered `Lock` events, *not* from an event handed
to it directly (that only proves decoding). Plus adapter allowance/receipt/
network-mismatch and contract false-return/balance-delta tests (incl. the
“false without transfer” case).

## Phase 3 — `tronwallet-adapter` + WalletConnect (#7)

Replace the hand-rolled TronLink integration with `@tronweb3/tronwallet-adapters`
/ `@tronweb3/walletconnect-tron` v4 behind the Phase-0 interface — they supply
real `connect`/`network()`/events. `switchChain` is used only where the adapter
advertises it. Wallet picker replaces the hardcoded "Bridge in with TronLink"
button. **Do not** advertise Trust-on-Tron via WalletConnect (Trust's WC list is
EVM+Solana, not Tron) — gate behind an acceptance test.

## Phase 4 — De-Tron-ify (#8) — with exit criteria

A single Tron implementation cannot prove the abstraction, so Phase 4 is
**done only when**:

- **Sphere bridge orchestration contains no Tron** RPC, address, explorer,
  allowance, or event-decoding logic — only `Step[]` execution + previews.
  *(landed — `bridgeIn.ts` imports only the neutral `ChainWallet` /
  `ReceiptReader` / `BridgeSourceAdapter` interfaces; the concrete Tron wiring
  (TronLink signer, HTTP node client, source adapter) is assembled in the
  composition root `loadBridges.createBridgeInDeps` and injected. `grep -i tron
  bridgeIn.ts` matches only comments + the type-only interface import.)*
- A **fake second-chain adapter** passes the **same** orchestration contract
  tests as Tron. *(landed — `bridgeIn.test.ts` "runs a single-signature deposit
  strategy through the same orchestration".)*
- The suite exercises **at least two deposit strategies** through that seam:
  1. allowance → approval → lock (ERC-20 style); *(landed)*
  2. single-step / signature-based deposit (EIP-3009 style, one signature). *(landed)*
- Adding EIP-2612/EIP-3009 later requires **no change to Sphere's state
  machine** — only a new adapter `prepareDeposit`.

Concrete changes landed: **discriminated-union manifest with string/hex chain
refs** — `BridgeManifest` is now `BridgeManifestBase` (chain-neutral, carries a
CAIP-2 `chainRef` like `tron:0xcd8690dc`) `&` a `family: 'tron'` variant holding
the Tron-only `chainId`/`rpcUrl`/`apiKey`; `loadOne` narrows on `family` and
cross-checks `chainRef` against the native `chainId` as an integrity pin (a new
mismatch test guards it). A second family adds a variant + loader branch; the
union stays additive.

Also landed: Tron-only fields now live inside the Tron variant; Unicity
`networkId` is derived from the trust base (not hardcoded `4`); a **manifest
registry keyed by family+chain+asset** (`buildBridgeRegistry`/`bridgeAssetKey`) —
the key is `chainRef:assetHex`, **stable across vault redeploys**, and duplicate
family+chain+asset registration throws; `AppBridges` now exposes `registry`
(`byKey`/`byCoinId`/`byTokenType`) and the in/out/resume flows resolve through it
instead of an ad-hoc `loaded.find`. **Explorer + address validation now live
behind the bridge**: a wallet-free `BridgePresentation` (`explorerTxUrl` /
`validateAddress`), dispatched on `manifest.family` (`bridgePresentation(bridge)`)
and resolved through the registry (`bridgePresentationFor(coinId)`); the modal no
longer keys on a numeric `chainId` or hardcodes a chain's URL / address shape.
(Presentation is a sibling of `BridgeSourceAdapter`, not a method on it — the
diagram lumps them, but presentation needs no wallet/rpc, so binding it to the
wallet-bound adapter would force every flow to construct a signer just to render a
link.)

Also landed: the neutral contracts now live in a **chain-neutral package
`@unicitylabs/bridge-core`** — `BridgeSourceAdapter` (+ its DTOs), the
`ChainWallet`/`ReceiptReader`/`TxReceipt` boundaries, `BridgePresentation`, and
`BridgeManifestBase`. The Tron plugin *implements* them (and re-exports for
back-compat); the EVM plugin will be a peer. Sphere's orchestrator (`bridgeIn.ts`)
now imports **only** `@unicitylabs/sphere-sdk` + `@unicitylabs/bridge-core` — zero
chain packages. Only the composition root (`loadBridges.ts`) imports the Tron
plugin, which is correct: that is where chain-specific wiring belongs.

Concrete changes still open:
- drop hardcoded `"tron"` in `deriveTokenType`/`deriveCoinId` (rides the
  coordinated cut — see legacy-identifier note below).

> **Build note:** `@unicitylabs/bridge-core` is a `file:` workspace package like
> the plugin (its `lib/` is git-ignored) — it must be built (`npm run build` in
> `bridge-core/`) before the plugin/Sphere typecheck against it.

## Phase 5 — EVM enablement (future, unblocked by Phase 4)

An `evm` adapter (`eth_call` allowance, EIP-1559 approve/lock, receipts,
WalletConnect + injected providers) and an EVM vault deploy. Trust Wallet becomes
first-class here. The 1.6 balance-delta hardening is exactly the portability the
EVM path needs.

---

## Protocol versioning & the coordinated cut

Two changes alter the **byte-level interop contract** and must not ship
piecemeal:

- **Salt in `lock` calldata + a new `Lock` event** (1.5).
- **One generic justification tag** replacing the per-asset tag (Phase 4).

Both require a single coordinated cut:

- **Bump `BRIDGE_PROTO_VERSION`.**
- Update [`00-interop-contract.md`](./00-interop-contract.md),
  [`MINT_REASON.md`](../MINT_REASON.md), the `bridge-vectors`, and every pin
  (configHash, vkey, ELF) **together**.
- Land **contract + TS + Rust guest/host + return service** in one cut; regenerate
  vectors; redeploy the vault (new `configHash`).
- The generic justification payload carries the full **`configHash`**, not merely
  chain/asset identifiers — this identifies vault rotations *and* the complete
  trusted configuration in one field.
- If the current Tron `tokenType`/`coinId` must stay stable across this, define a
  **versioned legacy-identifier scheme** (e.g. keep v0 derivation for the live
  Tron asset, use the family-parameterized derivation for v1+). "Drop hardcoded
  `tron` while keeping the derivation stable" is otherwise self-contradictory.

## Redeploy / migration discipline (not "stale")

A redeploy changes the vault address and therefore `configHash`, but **assets
remain locked in the old vault and existing bridged tokens retain claims against
it** — old vaults/proofs do **not** simply become "stale". Discipline:

- **Nile-only testing:** a destructive reset is acceptable **only** as an
  explicitly authorized testnet action that also accounts for any assets left in
  the old vault (drain/ignore by decision, recorded).
- **Any production-like deployment:** keep the **old manifest, verifier, and
  return service operational** until old-vault liabilities reach zero, or execute
  a proven migration. New deposits use the new vault; **old bridge-backs continue
  to settle against the old vault**.

## Sequencing / risk

- 1.1–1.4 (+ fail-closed persist) ship today with no redeploy — the biggest UX
  win (3→1 prompts) and the network/terminal-failure holes close first.
- The salt-on-chain half of 1.5 and all of 1.6 ride the single coordinated cut
  above (proto bump + vectors + redeploy), under the migration discipline — never
  a silent "old stuff is stale".
- The neutral interface pre-dates the vendors, so Phases 3/5 touch only the
  adapter, never Sphere; Phase 4 isn't "done" until a fake second chain proves it.
- Don't over-promise Trust/Tron — gate behind an acceptance test.
