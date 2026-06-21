# Optimistic `unlock`: committee receipt + challenge / fraud-proof flow

**Status:** design sketch. Implements the "optimistic committee" row of
[`BRIDGE_BACK.md`](./BRIDGE_BACK.md) and §5b of [`TRUST_MODELS.md`](./TRUST_MODELS.md).
Bridge-out (Unicity → Tron): release locked USDT against a **burned** Unicity
token, with low/constant L1 cost, low latency, and a failure mode weaker than
plain multisig — theft requires *quorum collusion **and** no honest watcher in
the window*.

It plugs into the **existing** `UnicityLock` with no contract change: set
`UnicityLock.setBurnProofVerifier(address(optimisticModule))`. The module is the
sole caller of `UnicityLock.unlock(nonce, to, amount)`, which already marks the
nonce `withdrawn` (single redemption) and checks `amount == locks[nonce].amount`.

---

## 1. What the committee is trusted for (and what it is not)

The withdrawal authorization decomposes into three linked claims. The contract
discharges two of them itself; the committee is trusted for **only one**.

| Claim | Who enforces it | How |
|---|---|---|
| **C1 — binding to a real deposit:** the withdrawal targets `lockNonce`, and `amount == locks[nonce].amount`, nonce not already `withdrawn` | **Contract** | reads `UnicityLock.locks(nonce)` at submit |
| **C2 — the burn happened:** Unicity burn state `burnStateHash` is final and its `BurnPredicate.reason` decodes to `{chainId, recipient, lockNonce, amount}` | **Committee (optimistically), falsifiable on-chain** | m-of-n signature on the happy path; on challenge, resolved by `IUnicityBurnVerifier` over the revealed proof |
| **C3 — no double-withdraw:** this burn funds at most one release | **Contract** | `consumedBurn[burnStateHash]` + `UnicityLock.withdrawn[nonce]` |

So the committee vouches for exactly one thing — **the existence, finality, and
reason-binding of the burn (C2)** — and even that is *publicly re-checkable*,
because a Unicity burn is a self-contained, BFT-certified, offline-verifiable
object (see `TRUST_MODELS.md` P1–P2). That is what makes the fraud proof cheap
and the optimistic model sound.

---

## 2. The committee receipt (what gets signed)

Each committee member signs, over a domain-separated digest:

```
Claim = { lockNonce, recipient, amount, burnStateHash, proofBlobHash }
digest = keccak256(DOMAIN, lockNonce, recipient, amount, burnStateHash, proofBlobHash)
DOMAIN = keccak256("UnicityOptimisticUnlock", tronChainId, address(this), address(lock))
```

- `burnStateHash` — the burned token's **final state hash** on Unicity. Unique
  by construction: the aggregator certifies at most one spend per state, so this
  is the single-spend anchor (no global-state oracle needed).
- `proofBlobHash = keccak256(serialized burn proof)` — a **commitment** to the
  exact self-contained burn object (token CBOR ‖ inclusion proof ‖
  `UnicityCertificate`) the committee verified. It is *not* revealed on the happy
  path; it only has to be produced if the claim is challenged (§4). This is what
  lets us handle "no such burn" without ever proving a negative on-chain.

A relayer collects ≥ `threshold` signatures and posts them. The committee never
touches Tron directly.

---

## 3. State machine (per `lockNonce`)

```
                         submitClaim (m-of-n sig, bond)
   None ───────────────────────────────────────────────► Claimed(deadline = now+W)
                                                              │
                 ┌────────────── challenge (bond) ────────────┤   finalize (after W)
                 ▼                                            │        │
        Challenged(deadline = now+R)                          │        ▼
          │                 │                                 │   Finalized → lock.unlock()
   reveal(blob) ✔     reveal ✘ / no reveal by R               │
   committee wins      committee fraud                        │
          │                 │                                 │
          ▼                 ▼                                 │
   back to Claimed     Cancelled (slash committee,            │
   (finalizable now)    reward challenger; lock stays         │
                        locked for an honest retry)           │
```

- **W** = `challengeWindow` (e.g. 30 min): minimal added latency on the happy
  path. Set per asset; shrink it (or drop to 0 = pure m-of-n) for faster, more
  trusting assets.
- **R** = `revealWindow` (e.g. 1 h): time the defender has to reveal the proof.

---

## 4. Why the fraud proof is cheap — and how "no such burn" is handled

A challenger does **not** have to prove a negative. The committee committed to
`proofBlobHash`, so on challenge the **defender (relayer/committee) must reveal
the preimage**:

- **They reveal a blob that `IUnicityBurnVerifier` accepts and whose decoded
  fields match the claim** → the committee told the truth → the **challenger
  forfeits its bond** (anti-griefing) and the claim becomes finalizable.
- **They reveal a blob that fails verification or whose fields mismatch** → the
  committee lied → **slash the committee, reward the challenger**, cancel.
- **They reveal nothing before `R`** → a forged/non-existent burn has no passing
  preimage, so silence ⇒ fraud → **slash the committee, reward the challenger**.

The only expensive step — `IUnicityBurnVerifier.verifyBurn(blob)` — runs **only
in a revealed dispute**, never on the happy path. An honest watcher can check
off-chain (it's a Unicity participant; it queries `burnStateHash`) before posting
a bond, so it only challenges genuine fraud and never loses its bond. A griefer
who challenges a valid claim pays.

```solidity
/// Verifies a self-contained Unicity burn proof against the *pinned* Unicity
/// root trust base. Stateless / view. Reverts if the proof is malformed or its
/// UnicityCertificate (BFT seal) / SMT inclusion path / burn predicate fail to
/// verify. Returns the burned token's final state hash and decoded burn reason.
///
/// v1  = native verification (heavy, but dispute-only gas).
/// end = swap for a zk-proof verifier; `proofBlob` becomes a succinct proof and
///       the dispute no longer leaks the token's history on-chain.
interface IUnicityBurnVerifier {
    function verifyBurn(bytes calldata proofBlob)
        external view
        returns (
            bytes32 burnStateHash,
            uint256 chainId,
            address recipient,
            uint256 lockNonce,
            uint256 amount
        );
}
```

`IUnicityBurnVerifier` is the **trustless escape hatch**: the optimistic
committee is a fast/cheap path *bounded* by an on-chain verifier of the same
burn a wallet checks. Upgrading it from native → zk (admin-rotatable, behind
timelock) is the migration to the `TRUST_MODELS.md` end state, invisible to
users.

---

## 5. Contract sketch

```solidity
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

interface IUnicityLock {
    function unlock(uint256 nonce, address to, uint256 amount) external;
    function locks(uint256 nonce) external view
        returns (address from, uint256 amount, bytes32 tokenId, bytes32 rcptCommit, bool withdrawn);
}

contract OptimisticUnlock {
    enum Status { None, Claimed, Challenged, Finalized, Cancelled }

    struct Withdrawal {
        Status  status;
        uint64  deadline;        // Claimed: end of W; Challenged: end of R
        address relayer;         // posted the claim; must reveal if challenged
        address challenger;
        address recipient;
        uint256 amount;
        bytes32 burnStateHash;   // single-spend anchor on Unicity
        bytes32 proofBlobHash;   // commitment to the burn proof (revealed only on dispute)
    }

    IUnicityLock          public immutable lock;
    IUnicityBurnVerifier  public burnVerifier;   // upgradeable (native -> zk), via timelock
    address               public admin;
    uint256               public immutable TRON_CHAINID;
    bytes32               public immutable DOMAIN;

    // committee = signer set; security stake is global, slashed on proven fraud
    mapping(address => bool) public isSigner;
    uint256 public threshold;       // m of n
    uint256 public committeeStake;  // committee's bonded balance held here
    uint256 public slashAmount;     // >= max single withdrawal; paid to a successful challenger

    // per-claim, anti-spam / anti-grief bonds (returned to the honest side)
    uint256 public relayerBond;
    uint256 public challengerBond;
    uint64  public challengeWindow; // W
    uint64  public revealWindow;    // R

    mapping(uint256 => Withdrawal) public withdrawals;  // by lockNonce
    mapping(bytes32 => bool)       public consumedBurn; // burnStateHash -> reserved/used

    event Claimed(uint256 indexed nonce, address recipient, uint256 amount, bytes32 burnStateHash, uint64 deadline);
    event Challenged(uint256 indexed nonce, address challenger);
    event Finalized(uint256 indexed nonce);
    event CommitteeSlashed(uint256 indexed nonce, address challenger);
    event ChallengeFailed(uint256 indexed nonce);

    // ---------------------------------------------------------------- happy path
    function submitClaim(
        uint256 lockNonce,
        address recipient,
        uint256 amount,
        bytes32 burnStateHash,
        bytes32 proofBlobHash,
        bytes[] calldata sigs            // >= threshold committee signatures, signer-sorted
    ) external payable {
        require(msg.value == relayerBond, "relayer bond");
        Withdrawal storage w = withdrawals[lockNonce];
        require(w.status == Status.None, "claim exists");
        require(!consumedBurn[burnStateHash], "burn consumed");

        // C1 + C3: bind to a real, unspent deposit of exactly this amount.
        (, uint256 locked,,, bool withdrawn) = lock.locks(lockNonce);
        require(!withdrawn && amount == locked && recipient != address(0), "bad nonce/amount");

        // C2 (optimistic): the committee attests the burn binds these fields.
        bytes32 digest = keccak256(abi.encode(DOMAIN, lockNonce, recipient, amount, burnStateHash, proofBlobHash));
        _requireQuorum(digest, sigs);

        consumedBurn[burnStateHash] = true;             // reserve; freed if cancelled
        withdrawals[lockNonce] = Withdrawal({
            status: Status.Claimed,
            deadline: uint64(block.timestamp) + challengeWindow,
            relayer: msg.sender,
            challenger: address(0),
            recipient: recipient,
            amount: amount,
            burnStateHash: burnStateHash,
            proofBlobHash: proofBlobHash
        });
        emit Claimed(lockNonce, recipient, amount, burnStateHash, withdrawals[lockNonce].deadline);
    }

    function finalize(uint256 lockNonce) external {
        Withdrawal storage w = withdrawals[lockNonce];
        require(w.status == Status.Claimed && block.timestamp >= w.deadline, "not ready");
        w.status = Status.Finalized;
        _pay(w.relayer, relayerBond);                   // return relayer bond
        lock.unlock(lockNonce, w.recipient, w.amount);  // release USDT (lock re-checks amount, marks withdrawn)
        emit Finalized(lockNonce);
    }

    // ---------------------------------------------------------------- challenge
    function challenge(uint256 lockNonce) external payable {
        Withdrawal storage w = withdrawals[lockNonce];
        require(w.status == Status.Claimed && block.timestamp < w.deadline, "window closed");
        require(msg.value == challengerBond, "challenger bond");
        w.status = Status.Challenged;
        w.challenger = msg.sender;
        w.deadline = uint64(block.timestamp) + revealWindow;
        emit Challenged(lockNonce, msg.sender);
    }

    /// Defender reveals the committed burn proof. Verifies it on-chain (dispute-only gas).
    function reveal(uint256 lockNonce, bytes calldata proofBlob) external {
        Withdrawal storage w = withdrawals[lockNonce];
        require(w.status == Status.Challenged, "not challenged");
        require(keccak256(proofBlob) == w.proofBlobHash, "wrong blob");

        try burnVerifier.verifyBurn(proofBlob) returns (
            bytes32 sh, uint256 cid, address rcpt, uint256 n, uint256 amt
        ) {
            if (sh == w.burnStateHash && cid == TRON_CHAINID
                && rcpt == w.recipient && n == lockNonce && amt == w.amount) {
                // committee told the truth -> challenger was wrong
                _pay(w.relayer, challengerBond);        // griefer's bond to the relayer
                w.status = Status.Claimed;
                w.deadline = uint64(block.timestamp);   // finalizable immediately
                w.challenger = address(0);
                emit ChallengeFailed(lockNonce);
                return;
            }
        } catch { /* malformed / invalid proof => fraud, fall through */ }

        _slash(lockNonce, w);                            // fields mismatched or verify failed
    }

    /// No valid reveal before R => a forged/absent burn has no passing preimage => fraud.
    function resolveTimeout(uint256 lockNonce) external {
        Withdrawal storage w = withdrawals[lockNonce];
        require(w.status == Status.Challenged && block.timestamp >= w.deadline, "still revealing");
        _slash(lockNonce, w);
    }

    function _slash(uint256 lockNonce, Withdrawal storage w) internal {
        consumedBurn[w.burnStateHash] = false;           // free the anchor; lock stays locked for honest retry
        w.status = Status.Cancelled;
        committeeStake -= slashAmount;
        _pay(w.challenger, challengerBond + slashAmount);// refund + reward
        _pay(w.relayer == w.challenger ? w.challenger : address(0xdead), 0); // relayer bond burned (relayed fraud)
        emit CommitteeSlashed(lockNonce, w.challenger);
    }

    // -------------------------------------------------- equivocation (instant slash, no window)
    // Two valid quorums binding the same lockNonce (or same burnStateHash) to different content
    // are self-evident fraud: verify both signature sets, compare, slash immediately.
    function slashEquivocation(/* two full Claims + their sig sets */) external { /* ... */ }

    // ------------------------------------------------------------------- internals / admin
    function _requireQuorum(bytes32 digest, bytes[] calldata sigs) internal view { /* ecrecover loop,
        strictly increasing signer addresses (dedupe), each isSigner, count >= threshold. Use a
        BLS/threshold aggregate for O(1) verification — the MPC variant. */ }
    function _pay(address to, uint256 amt) internal { /* nonReentrant transfer */ }
    // admin (= UnicityLock.admin / governance, timelocked): setSigners, setThreshold, setParams,
    // setBurnVerifier (native -> zk), depositStake / withdrawStake (with unbonding delay).
}
```

> The sketch elides signature-loop and admin bodies and is not gas-tuned; it
> shows the trust decomposition and the challenge state machine. `_slash`'s
> relayer-bond handling is shown crudely — in practice burn the relayer bond to a
> sink or treasury so relaying a fraudulent claim is never profitable.

---

## 6. Economic + liveness analysis

- **Theft requires** a colluding `threshold` quorum **and** that no honest
  watcher challenges within `W`. With `W > 0` this is strictly weaker than plain
  multisig (which needs only quorum collusion). Watchers are paid (`slashAmount`)
  and their check is cheap and decisive (§4), so a live watcher set is realistic.
- **`slashAmount` must exceed the maximum at-risk withdrawal** (or cap
  per-withdrawal amount at `slashAmount`), so successful fraud is never
  net-profitable for the committee.
- **Griefing is bounded:** challenging a valid claim costs `challengerBond` (lost
  to the relayer on a successful reveal) and at most delays that one withdrawal
  by `R`. An honest watcher can confirm fraud off-chain before bonding, so it
  never loses.
- **Liveness (not safety) depends on the committee:** a quorum can *stall*
  (refuse to sign) but cannot *steal*. Mitigate with multiple independent
  relayers, committee rotation, and a governance fallback that can rotate a dead
  set. A stalled withdrawal never loses funds — the burned token's holder can
  retry once a quorum signs.
- **Privacy:** a dispute reveals the burned token's history on-chain (Unicity's
  off-chain privacy is lost for that one token in the rare challenge path). The
  zk verifier upgrade removes this — reveal a succinct proof, not the history.
- **Cost (happy path):** `submitClaim` (m-of-n `ecrecover`, or O(1) with an
  aggregate sig) + `finalize` (one `unlock`). **Constant in token age** — the
  whole point vs. on-chain history replay.

## 7. Config knobs (per asset, published in the plugin manifest)

`{ signerSet, threshold, challengeWindow W, revealWindow R, slashAmount, relayerBond, challengerBond, burnVerifier }`
— each asset picks its point on the trust/latency spectrum, auditable like the
other trust anchors in [`PLUGIN_ARCHITECTURE.md`](./PLUGIN_ARCHITECTURE.md).
`W = 0` collapses to a pure m-of-n receipt (fastest, most trusting); a native or
zk `burnVerifier` plus `W > 0` gives optimistic security; a zk `burnVerifier`
with a deposit-time mandatory proof is the trustless end state.
```
