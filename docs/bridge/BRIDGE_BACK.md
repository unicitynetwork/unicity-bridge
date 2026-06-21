# Bridging back: Unicity → Tron (design note, not yet implemented)

Returning a bridged token to Tron means: **burn** the Unicity token and
**release** the matching locked USDT from `UnicityLock`. The hard part is letting
a Tron contract be convinced that the burn really happened, cheaply and with
acceptable latency.

## What "burn" means here

The holder transfers the token to a `BurnPredicate`
([source](../../state-transition-sdk-js/src/predicate/builtin/BurnPredicate.ts))
whose `reason` commits to the Tron withdrawal:

```
burnReason = CBOR [ chainId, tronRecipient (21B), lockNonce, amount ]
```

`UnicityLock.unlock(...)` releases USDT to `tronRecipient` for `lockNonce` and
marks that nonce `withdrawn` (single use). The remaining question is *what proof*
`unlock` accepts that the burn is final on Unicity.

## Options

| Option | L1 cost (Tron) | Latency | Trust | Notes |
|---|---|---|---|---|
| **On-chain last-burn verification** | High & growing | High | Trustless | The contract re-verifies the token's whole history (each transfer adds a payer sig + a Unicity certificate). Cost grows with token age; verifying BFT certs + SMT paths on TVM is heavy. Doesn't scale. |
| **zk-compress history** (zkVM/STARK → Groth16) | ~low-$ on Tron | Medium (proving) | Trustless | One succinct proof of "this token burned with this reason". **Blocked:** no Rust/zkVM SDK yet for Unicity. Long-term target. |
| **Burn-proof aggregation** (semi-trusted batcher) | Low (amortized) | Medium (batch window) | Semi-trusted batcher (can stall, not steal) | A node verifies many burns off-chain and posts one compact proof/receipt per batch (e.g. a Merkle root of `{nonce, recipient, amount}` + a succinct validity argument). `unlock` checks membership. |
| **Committee / multisig receipt** | Low (1 sig set) | Low | Trust quorum honesty (m-of-n) | Trustees independently verify the burn (using the same `Token.verify` + spend check) and co-sign a receipt; `unlock` checks the multisig against the known trustee set. |
| **MPC / threshold sig** | Lowest (1 sig) | Low | Trust quorum | Same as committee but produces a single threshold signature — cheapest on-chain check, more setup complexity. |

## Recommendation (phased)

1. **Near term — committee/multisig receipt.** Simplest to ship, low and constant
   L1 cost, low latency. Trust is an explicit m-of-n trustee set that can be
   published and rotated. The trustees run exactly the same verification a wallet
   does (`Token.verify` over the full history + `isSpent`/burn check), so the
   security argument is auditable. Good enough for a v1 return path.
2. **Medium term — burn-proof aggregation.** Move trust from "quorum vouches" to
   "node posts a checkable proof", and batch to cut per-withdrawal cost. The
   committee can run the aggregator in the interim.
3. **Long term — zk.** Once a Rust zkVM/STARK pipeline exists, replace the
   trusted/semi-trusted step with a succinct trustless proof (Groth16 wrapper for
   cheap TVM verification). This is the end state; everything above is a bridge to
   it.

In all variants the `UnicityLock` accounting is the same: `unlock` is gated on a
proof of the burn, releases to the committed `tronRecipient`, and marks
`lockNonce` withdrawn so a burn can be redeemed at most once — symmetric to how
`lock` binds a deposit to exactly one `tokenId` on the way in.

## Open questions to resolve before implementing

- Exact `unlock` proof format per chosen option (multisig encoding / receipt
  schema / proof system).
- Trustee set governance & rotation (committee option).
- Fee/relayer model on Tron (who pays energy for `unlock`).
- Partial returns: a burn returns the token's full value; splitting first on
  Unicity (already supported) handles partial amounts.
