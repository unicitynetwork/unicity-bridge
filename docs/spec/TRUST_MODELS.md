# Bridge trust models: latency, cost, and where Unicity's p2p tokens change the math

**Status:** research note / design exploration. Companion to
[`MINT_REASON.md`](./MINT_REASON.md) (the trustless bridge-in we have) and
[`BRIDGE_BACK.md`](./BRIDGE_BACK.md) (bridge-out options). This note steps back
and asks: given that fully-trustless operation has bad UX today, what is the
right set of *ideological compromises*, and what can we do that a generic bridge
(LayerZero, Wormhole, …) cannot — because Unicity tokens are self-contained,
BFT-certified, single-spend p2p objects?

---

## 1. The problem is actually two problems

People say "bridging is slow and expensive." Those are two different costs on
two different legs, and conflating them leads to the wrong fix.

### 1a. Bridge-IN latency (Tron/Ethereum → Unicity)

To accept a deposit *trustlessly*, the receiver must wait for the **source
chain** to be final, because a reorg could un-lock the asset after the Unicity
token is already minted and circulating.

- Tron: ~19 SR blocks ≈ **~57 s** to irreversibility.
- Ethereum L1: 2 epochs ≈ **~12.8 min**.
- An L2 deposit that needs L1 finality: tens of minutes.

We already removed *part* of this: the **minter trusts its own lock** and mints
once the lock tx is in a block (`MINT_CONFIRMATIONS=0`); only an independent
**receiver** enforces the K-confirmation threshold (the `verify` retry loop).
But a true third party who is *handed* a freshly-minted token still has to wait
out source finality before treating it as good. That residual wait is the UX
problem on the way in.

> **This latency is inherent to trustlessness.** You cannot safely accept a
> reorgeable deposit. Every "instant" bridge fixes it the same way: somebody
> *takes the reorg risk for a fee* (§5a). There is no cryptographic shortcut.

### 1b. Bridge-OUT cost (Unicity → Tron)

A Tron contract must be convinced a Unicity token was **burned** with a reason
committing to the withdrawal. The fully-trustless ways to do that on the TVM are
either non-scaling (re-verify the token's whole history + BFT certs on-chain;
cost grows with token age) or not-yet-available (zk-compress the history; no
Rust/zkVM SDK for Unicity yet). See `BRIDGE_BACK.md`. So today bridge-out is
"prohibitively expensive" exactly as you observed.

### 1c. The asymmetry that everyone misses

The two legs are **not symmetric**, and the difference is Unicity's biggest
lever:

| | Source finality | "Did it happen?" proof |
|---|---|---|
| **Tron → Unicity** | probabilistic, slow (must wait K) | a Tron event, re-checkable by anyone over RPC (cheap on the *verifier*, but the verifier is a full Unicity participant) |
| **Unicity → Tron** | **instant BFT finality** — the burn carries a `UnicityCertificate`; there is nothing to "wait out" | a burn proof that is **self-contained and offline-verifiable**, but the *consumer* (a TVM contract) is too weak to check it cheaply |

So bridge-out has **no source-finality wait at all** — the entire cost is
"translate a Unicity burn into something a dumb contract will accept." That is a
much more tractable problem than waiting for probabilistic finality, and it's
where committee/optimistic/zk designs live.

---

## 2. The trust spectrum

Every model is a point on a line from "verify everything yourself" to "trust a
named party." The honest framing is *what does each failure mode cost you*.

| Model | L1 cost (Tron) | Latency | Trust assumption | If the trusted part fails |
|---|---|---|---|---|
| **Self-verify (light client / re-check RPC)** | n/a for in; very high for out | source finality | none | — (trustless) |
| **zk validity proof** | low–med (Groth16 on TVM) | proving time | soundness of the proof system | — (trustless) |
| **Optimistic + fraud proof** | low | challenge window | ≥1 honest watcher online | safe unless *also* no honest watcher in-window |
| **Committee / m-of-n multisig** | low (1 sig set) | low | m-of-n don't collude | quorum collusion ⇒ theft |
| **MPC / threshold sig** | lowest (1 sig) | low | same quorum, cheaper on-chain | quorum collusion ⇒ theft |
| **LP-fronted "instant"** | (settles on slow path) | **instant** | none for the *recipient*; LP eats reorg risk | LP loses its own funds, user already paid/received |
| **Single custodian** | low | low | one party honest | rug |

Two observations:

- **Optimistic dominates committee** when fraud proofs are *cheap and anyone can
  file them* — you get committee-like cost/latency but the failure mode weakens
  from "quorum collusion" to "quorum collusion **and** nobody watching." Unicity
  makes fraud proofs cheap (§3), so optimistic is unusually attractive here.
- **LP-fronting is orthogonal** — it doesn't change the trust of the underlying
  bridge, it just hides the latency behind someone's balance sheet. It composes
  with any of the above and is the real UX win for bridge-in (§5a).

---

## 3. Why Unicity changes the math

Generic bridges (LayerZero, Wormhole, Axelar) move **opaque messages** between
chains whose state neither side can cheaply verify, so they lean on a verifier
set vouching "we saw block X." Unicity tokens are not opaque messages — they are
**self-contained, certified, single-spend objects**. Four concrete properties:

### P1 — A burn is a self-contained, BFT-certified, *offline* object

A burned token is a `Token` whose latest state is a `BurnPredicate(reason)`,
carrying its full history and an `InclusionProof` that wraps a
`UnicityCertificate` (the L1 BFT seal). Verifying "this exact token was burned
with this reason, and that is final" needs **only the Unicity root trust base**
(a small pinned key set) — *no node, no RPC, no chain sync*. Contrast with a
Wormhole guardian, which must run a full source node and attest block inclusion.

Consequences:
- A Unicity "DVN"/trustee is **stateless and trivial to run** — hand it the
  token blob, it runs `Token.verify(...)`. This collapses the operational cost
  and centralization pressure of a verifier set.
- Anyone can re-run the identical check ⇒ **attestations are publicly
  falsifiable** ⇒ fraud proofs are cheap ⇒ optimistic mode is viable (§5b).

### P2 — The L1 aggregator already enforces single-spend

The committee's scary job in a normal bridge is being a **global-state oracle**:
"is this the *latest* state, or was it also spent elsewhere?" On Unicity the
aggregator certifies **at most one** transition per token state; the burn's
inclusion proof *is* the single-spend guarantee. The committee therefore never
has to track global state or worry about equivocation — it attests a fact that
the L1 already made unique. That removes the hardest and most failure-prone
responsibility a bridge committee normally carries.

### P3 — Instant finality on the Unicity side

A `UnicityCertificate` is BFT-final the moment it exists. Bridge-OUT has **zero
source-finality latency** (cf. §1c). The only latency is committee/relayer
round-trip + Tron inclusion — seconds, not the minutes a probabilistic chain
forces.

### P4 — Permissionless mint keyed to `tokenId`, value is freely p2p

Minting is keyed to `tokenId` and bound to a `recipientCommitment`; there is no
privileged minter key to steal, and an LP/relayer **cannot mint a bridged token
to itself**. And once minted, value moves by ordinary off-chain transfer/split —
instant and final. This is what makes the LP inventory model (§5a) clean: an LP
hands the recipient a *real, fully-backed* token from inventory, not an IOU.

---

## 4. The LayerZero model, mapped onto Unicity

**What LayerZero v2 actually is** (stripped of branding): the application picks a
set of **DVNs** (Decentralized Verifier Networks) plus a threshold, and an
**Executor** that delivers the message. A DVN attests "this packet corresponds
to a finalized source event." The receiving endpoint accepts the message if the
configured DVN quorum signed it. The older "Ultra Light Node" was the same idea
with two roles: an **Oracle** delivers the source block header, a **Relayer**
delivers the Merkle proof, and the destination verifies the proof against the
header — security = oracle and relayer are independent and don't collude. Net:
**LayerZero is a configurable committee** (with a light-client flavor), not a
trustless light client.

Mapping each leg:

- **Bridge-IN (Tron → Unicity).** Unicity already has something *better* than a
  DVN set here: each recipient is *its own verifier* — it re-checks the Tron lock
  event over RPC (`MINT_REASON.md`). A DVN set would only add the ability to
  **skip the finality wait by trusting attestors** — i.e., trade trust for
  latency. The cleaner answer to the latency problem is LP-fronting (§5a), which
  buys instant UX *without* the recipient trusting anyone.
- **Bridge-OUT (Unicity → Tron).** This maps onto the DVN model *exactly*: the
  Tron `unlock` contract is the receiving endpoint; the DVN set attests the burn;
  `unlock` checks the quorum signature. This is precisely the
  "committee/multisig receipt" row in `BRIDGE_BACK.md`. LayerZero's contribution
  is the **engineering pattern**: per-asset configurable required/optional
  verifiers + threshold, so trust is explicit, published, and tunable per asset.

**The upgrade Unicity enables over vanilla LayerZero:** in LayerZero a DVN's
claim is expensive to independently check (you'd need a source light client), so
in practice you *trust* the DVN set. On Unicity a trustee's claim is checkable by
anyone from a 30 KB token blob + the trust base (P1) and the spend is already
unique (P2). So we can run the same DVN/committee pattern but in **optimistic
mode with slashing** — the committee provides liveness and a fast path, while
correctness is enforced by cheap, permissionless fraud proofs. You get
LayerZero's UX with a materially stronger failure mode.

---

## 5. Recommended designs (with the compromise stated plainly)

### 5a. Bridge-IN: LP-fronted instant delivery (recipient stays trustless)

The finality wait (§1a) is unavoidable for *whoever takes reorg risk*. So move
that risk to a party who is paid to take it and who has no claim on the user:

1. Recipient requests N units of the bridged asset.
2. A **liquidity provider** (permissionless, competitive) observes the user's
   Tron lock *as soon as it's in a block* (pre-finality) and immediately
   **transfers/splits N units from its existing inventory** of already-bridged,
   already-final tokens to the recipient. This is an ordinary Unicity p2p
   transfer: **instant, final, and fully trustless from the recipient's side** —
   they hold a real token, not an IOU (P4).
3. Once the user's lock reaches finality, the LP mints the replacement token from
   that lock to replenish inventory. **The LP alone carries the reorg risk** for
   the finality window, compensated by a spread/fee.

- **UX:** instant, and the recipient trusts *nobody* (this is strictly better
  than DVN-attested fast paths, where the recipient trusts the attestors).
- **Compromise:** requires LP capital and a fee; needs a discovery/quoting
  mechanism (an order book or AMM of LPs). Reorg risk is real but bounded and
  priced; LPs can require their *own* higher confirmation count before fronting
  large amounts. This is the Across/Stargate/Hop pattern, but cleaner because
  Unicity inventory is genuine fungible-by-value tokens, not wrapped IOUs.

### 5b. Bridge-OUT: optimistic committee receipt (DVN set + challenge window)

For Unicity → Tron, ship the committee model but make it optimistic:

1. Holder burns the token to a `BurnPredicate` whose reason commits to
   `{chainId, tronRecipient, lockNonce, amount}` (already in `BRIDGE_BACK.md`).
2. A **relayer** posts the withdrawal to `UnicityLock` along with an **m-of-n
   committee signature** over `{nonce, recipient, amount, burnStateId}`. The
   committee signs only after each member ran `Token.verify` on the burn — which,
   per P1–P3, is a stateless offline check with no finality wait.
3. `unlock` accepts after a short **challenge window** (e.g. minutes). Any
   watcher can **veto** by submitting the actual burned-token blob showing the
   committee's claim is false (wrong amount/recipient/nonce, or no such burn).
   Because the burn is self-contained and the spend is unique (P1, P2), the
   on-chain fraud check is a bounded, cheap verification — not a full history
   replay. A successful challenge slashes the committee's bond.

- **Latency:** seconds to sign + the challenge window. Drop the window for a
  pure m-of-n receipt if you want sub-minute finality and accept the stronger
  trust assumption — make it a **per-asset config knob** (LayerZero-style).
- **Cost:** one signature-set verification on the TVM, constant in token age.
- **Compromise / failure mode:** with the window, theft requires
  *quorum collusion* **and** *no honest watcher files in time* — much weaker than
  plain multisig. Liveness still depends on the committee (it can stall, not
  steal); mitigate with multiple relayers and committee rotation/governance.

### 5c. Compose and configure

- `confirmations` (already a param) = the *self-verify* finality bar; LPs (§5a)
  let users opt out of waiting for it without weakening their own trust.
- Bridge-out: expose `{committee set, threshold, challengeWindow}` as published
  per-asset config (the manifest in `PLUGIN_ARCHITECTURE.md`), so each asset
  picks its point on the spectrum and it's auditable.

---

## 6. End-states and the phased path

| Approach | In-latency | Out-latency | Out L1 cost | Recipient trust | Honest-failure cost |
|---|---|---|---|---|---|
| Pure trustless (today) | source finality | huge / N-A | grows w/ age | none | — |
| **+ LP fronting (in)** | **instant** | — | — | **none** | LP loses own funds |
| **+ optimistic committee (out)** | — | window (mins) | low, constant | none (re-checkable) | needs a watcher in-window |
| m-of-n receipt, no window (out) | — | **seconds** | low, constant | quorum honesty | quorum collusion ⇒ theft |
| zk validity (out, endgame) | — | proving time | low (Groth16) | none | — |

**Recommended sequencing:**

1. **Now — LP-fronted bridge-in.** Biggest UX win, *zero* added trust for users,
   no new cryptography. Needs an LP/quoting layer, not protocol changes.
2. **Now/next — optimistic committee bridge-out.** Implements `BRIDGE_BACK.md`'s
   v1, but the challenge window (cheap thanks to P1/P2) downgrades the failure
   mode from "trust the quorum" to "trust that *someone* is watching." Publish
   and rotate the committee; bond it.
3. **Later — zk bridge-out** once a Rust zkVM/STARK pipeline exists for Unicity:
   replace the committee's attestation with a succinct validity proof, retiring
   the trust assumption entirely. The committee can run the prover in the interim
   (proof first, trust as fallback), so the migration is invisible to users.

The throughline: **don't fight the finality wait with cryptography — front it
with liquidity (in), and don't pay to re-prove history on a weak chain —
attest the self-contained burn cheaply and make lying publicly punishable
(out).** Both lean directly on Unicity's distinctive properties: instant BFT
finality, offline-verifiable certified tokens, L1-enforced single-spend, and
free p2p value movement.
