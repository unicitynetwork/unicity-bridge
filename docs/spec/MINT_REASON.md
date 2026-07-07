# Bridge mint-reason: Tron USDT lock proof

**Status:** normative spec for bridge-in of USDT (TRC20) on Tron → Unicity.
**Audience:** anyone implementing a minter or an independent verifier.

## Background

A Unicity token carries an optional **mint-reason** in the genesis mint
transaction. In the state-transition SDK this is the
`MintTransaction.justification` field — opaque, CBOR-tagged bytes that travel
with the token forever and are re-verified by every recipient. Verification is
dispatched by the justification's **CBOR tag** through
`MintJustificationVerifierService` →
[`IMintJustificationVerifier`](../../state-transition-sdk-js/src/transaction/verification/IMintJustificationVerifier.ts).

For a bridged asset the mint-reason is a **self-contained proof that the source
asset was locked on the source chain**, bound to the exact Unicity token it
funds. There is no trusted bridge operator: every verifier re-checks the proof
against a Tron RPC node at verify time.

## Binding the locked source token with minted token

Minting in the SDK is **permissionless**:
[`MintSigningService`](../../state-transition-sdk-js/src/crypto/MintSigningService.ts)
derives a *universal* minter key from the `tokenId`, so anyone can produce a
validly-signed genesis for any `tokenId`. Security therefore comes entirely from
the justification check, which is made non-replayable and theft-proof by having
the **Tron lock contract commit to the specific `tokenId` and recipient
predicate**:

- `tokenId` is globally unique, and the Unicity aggregator enforces exactly one
  genesis per `tokenId`. A lock that commits to `tokenId = X` can therefore fund
  **exactly one** token. → no replay, no double-mint.
- The lock also commits to `recipientCommitment = H(recipient predicate)`. The
  minted token is only accepted if it is locked to that exact recipient. → even
  though anyone may *build* the mint, only the recipient the locker designated
  on-chain can own it. → no theft / front-running.

"Authorized party" = the recipient the locker named on Tron. No privileged
minter key is needed.

## Canonical identifiers (deterministic)

Given the canonical config `{ chainId, assetContract }`:

```
TRON_USDT_TYPE  = SHA256(utf8("unicity-bridge:tron:" + chainId + ":" + assetContractHex))   // 32 bytes → TokenType
bridgedCoinId   = SHA256(utf8("unicity-bridge-coin:tron:" + chainId + ":" + assetContractHex)) // 32 bytes → coinId
decimals        = 6                                                                          // USDT TRC20
recipientCommitment = SHA256(recipient.toCBOR())   // recipient is the EncodedPredicate the token is minted to
```

`assetContractHex` is the lowercase hex of the **20-byte EVM-form** Tron address
(the form Tron event logs use — the `41` prefix and base58 checksum stripped;
`toEvmAddressHex` accepts `T…`, `41…`, or 20-byte hex and normalizes to it).
`chainId` distinguishes mainnet (`0x2b6653dc` = 728126428) from Nile testnet
(`0xcd8690dc` = 3448148188).

## Justification structure

CBOR, wrapped in the per-asset tag `TRON_USDT_LOCK_JUSTIFICATION_TAG = 1330002`
(bridge tags are allocated one-per-asset so the SDK's tag→verifier registry
stays 1:1):

```
#tag(1330002) [
  version:        uint,    // = 1
  chainId:        uint,    // Tron network id (see above)
  lockContract:   bstr,    // 20-byte EVM-form address of the canonical UnicityLock
  assetContract:  bstr,    // 20-byte EVM-form address of the USDT TRC20 token
  txid:           bstr,    // 32-byte Tron tx hash of the lock() call
  logIndex:       uint,    // index of the Lock event within that tx's logs
  amount:         uint,    // locked USDT amount (6 decimals)
  nonce:          uint     // lock nonce assigned by the contract (from the event)
]
```

The justification carries only a **reference** to the on-chain lock plus the
trust anchors. The values that matter for security — `tokenId`, `recipient`,
`tokenType`, and the token's declared value — are read from the
`CertifiedMintTransaction` itself and checked against the on-chain event; they
are never trusted from the justification body.

## Verification rule

Executed by every recipient (in `IMintJustificationVerifier.verify`). All checks
must pass; any failure ⇒ `VerificationStatus.FAIL` with a specific message.

1. **Decode & trust anchors.** Decode the justification. Reject unless
   `chainId`, `lockContract`, `assetContract` equal the verifier's configured
   canonical values. (These pin *which* Tron contract/asset/network is
   authoritative — the root of trust, shipped in plugin config.)
2. **Token type.** Assert `transaction.tokenType.bytes == TRON_USDT_TYPE`.
3. **Tx success.** Tron RPC `wallet/gettransactioninfobyid(txid)`; require the
   receipt result to be `SUCCESS`.
4. **Finality.** `wallet/getnowblock` tip; require
   `tip.blockNumber − tx.blockNumber ≥ confirmations` (default `K = 20`, ~Tron SR
   irreversibility). Below that ⇒ FAIL ("awaiting source finality").
5. **Locate event.** Take `log[logIndex]`; require it was emitted by
   `lockContract` and is the `Lock` event (topic0 == keccak256 of the event
   signature). Decode `{ nonce, from, amount, unicityTokenId, recipientCommitment }`.
6. **Amount.** Assert `event.amount == justification.amount`. If a value
   extractor is configured, also assert the token's declared bridged-coin value
   (`bridgedCoinId`) equals `event.amount`.
7. **Binding (core).** Assert
   `event.unicityTokenId == transaction.tokenId.bytes` **and**
   `event.recipientCommitment == SHA256(transaction.recipient.toCBOR())` **and**
   `event.nonce == justification.nonce`.
8. OK.

## Minter flow (and latency)

1. Recipient/locker picks a `salt`; computes
   `tokenId = TokenId.fromSalt(networkId, salt)` and
   `recipientCommitment = SHA256(recipient.toCBOR())` for the intended
   `SignaturePredicate(recipientPubkey)`.
2. On Tron: `usdt.approve(lock, amount)` then
   `lock.lock(amount, tokenId, recipientCommitment)`. Wait for `K`
   confirmations (~60 s) so step 4 will pass for verifiers.
3. Build `MintTransaction.create(networkId, recipient, valueData,
   TRON_USDT_TYPE, salt, justification)` and certify the genesis on Unicity.
4. Hand the token to the recipient. Because the minter waited for finality, the
   token verifies immediately for everyone. A wallet *may* accept a token before
   finality and display "awaiting source finality" until step 4 passes.

## Security summary

| Attack | Prevented by |
|---|---|
| Mint without locking | RPC check (steps 3–6): no matching on-chain Lock |
| Replay a lock for a 2nd token | Binding (step 7) + aggregator one-genesis-per-tokenId |
| Inflate value vs. locked amount | Amount check (step 6) |
| Steal someone's lock / front-run | recipientCommitment binding (step 7) |
| Point at a rogue lock contract | Trust-anchor check (step 1) |
| Use a reorged/unconfirmed lock | Finality check (step 4) |
