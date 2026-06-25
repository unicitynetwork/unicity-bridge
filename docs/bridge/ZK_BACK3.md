# Returning bridged assets to an EVM chain with a SNARK

**Status:** clean-room design for the trustless return path, Unicity → EVM source
chain. This document is self-contained.

This is the *return* leg of an asset bridge. Assets are locked on an EVM chain and
minted as native tokens on **Unicity**; this document specifies how a holder burns
a Unicity token and releases the matching locked asset on the EVM chain, with the
release gated purely on a succinct validity proof. There is no committee, no
operator trust, and no per-withdrawal state growth on the source chain.

The design rests on two ideas:

1. **A decomposed proof.** "This burn deserves a release" is not one monolithic
   check. It is a handful of independent claims (finality, value, backing,
   replay, destination). Each is proved against the *cheapest sufficient
   evidence*, and the most expensive part of a naïve check — re-verifying a BFT
   certificate at every step of the token's history — collapses to a single
   certificate verification by exploiting Unicity's structure as a write-once
   authenticated key–value store.

2. **A nullifier accumulator.** Replay protection is the one thing the source
   chain must store. We store it as a **single 32-byte accumulator root**, not as
   an ever-growing set and not as per-account nonces. The proof maintains the
   accumulator; the chain keeps one word and verifies one root transition.

---

## 1. The flow

### 1.1 Cast

- **Unicity.** A network whose ledger is a **write-once authenticated key–value
  store**. A key (a token *state*) can be written at most once, ever; once
  written it persists in every future root. The network periodically seals its
  store under a BFT **certificate** that any party can verify offline against a
  pinned **trust base** (the validator set + threshold). For any key it returns a
  **cryptographic inclusion proof** against a sealed root. This single-write
  property is what makes a token *state* behave as a globally unique object — and,
  as we will see, a natural nullifier.

- **A Unicity token.** A self-contained object: a genesis *mint*, a chain of
  *transfer* transitions, optional *splits*, and possibly a terminal *burn*. Each
  transition consumes one state and produces the next; each state is a key in the
  Unicity store. The token carries enough data to be re-verified by anyone holding
  only the trust base.

- **The EVM source chain.** Holds the locked assets in a **vault** contract and a
  Groth16 verifier. It is "weak": it cannot run a Unicity node, cannot make RPC
  calls, and pays per byte and per operation. Everything it learns about Unicity
  must arrive as a proof.

- **A prover.** A stateless, permissionless service that turns a burned token into
  a Groth16 proof and submits it. It cannot steal or redirect funds; it can only
  stall, and anyone can replace it.

### 1.2 Bridge-in, in one paragraph

To bring an asset in, a user locks it in the vault, which records a compact
**lock digest** committing to `{asset, amount, the Unicity token id, the intended
recipient}` and emits a lock event. A Unicity token is then minted whose genesis
carries a *mint reason* referencing that lock. The return path needs the vault to
have retained the lock digest (§4); it does not need the vault to remember
anything else about the deposit.

### 1.3 Bridge-out (the subject of this document)

```
  HOLDER (light)            PROVER (stateless)                 EVM VAULT
  ─────────────            ──────────────────                 ──────────
  1. burn token to
     BridgeBackReason ───►  2. fetch a recent certified
     {recipient, amount,       Unicity root R* and fresh
      fee, deadline, …}        inclusion proofs for the
                               token's whole lineage
                            3. build one SNARK proving
                               finality + value + backing
                               + replay-freshness
                            4. submit(publicValues, proof,
                               leaves, lockRefs) ───────────►  5. verify Groth16
                                                               6. check pinned config
                                                                  + lock digests
                                                               7. spentRoot := newRoot
                                                               8. transfer asset to
                                                                  recipient (− fee)
```

Narratively:

1. **Burn.** The holder moves the token to a burn state whose *reason* commits to
   the withdrawal: destination chain, vault, asset, recipient, gross amount, an
   optional relayer fee, and a deadline. The burn is an ordinary Unicity
   transition; the moment it is certified it is final. A partial return is a
   split first, then a burn of the child of the desired size.

2. **Prove.** A prover takes the burned token, fetches a recent certified root and
   the inclusion proofs the circuit needs, and produces one succinct proof. The
   prover holds no secrets and no authority; any prover can reproduce the proof
   from public data plus the burned token blob.

3. **Release.** The vault verifies the proof, checks that its pinned parameters
   match, checks that the lock digests the proof relied on are ones it actually
   stored, advances its single accumulator root, and transfers the asset to the
   recipient. Re-submitting the same proof fails, because the accumulator root has
   moved and the burn is now a member of it.

What the vault **never** does: track Unicity roots, run a light client, verify a
Merkle branch, recover a signature, store a nullifier set, or store a per-account
nonce. Its entire return-path footprint is the lock digests it already had plus
**one** 32-byte accumulator root.

---

## 2. Security decomposition

A wallet that accepts a returned withdrawal is really asserting five independent
things. Naming them separately is what lets us prove each one cheaply.

| Claim | Statement | Cheapest evidence |
|---|---|---|
| **A — Anchor** | "These facts are sealed by the real Unicity." | One BFT certificate for a recent root `R*`, verified against the pinned trust base. |
| **F — Finality** | "The token reached *this* burn state, committing to *this* withdrawal, and it is final." | The burn state is a key present in `R*` — one inclusion proof. |
| **V — Value** | "The burned amount is real, conserved value — it descends from a genuine mint and every split along the way conserves value." | The *value lineage* only: genesis + the split nodes on the path to this leaf, each present in `R*` and sum-checked. |
| **B — Backing** | "The genesis mint is backed by a real source-chain lock." | Recompute the lock digest from a private witness; require it among the public lock refs the vault checks against its own storage. |
| **R — Replay** | "This burn has not been released before." | Non-membership of the burn's nullifier in the accumulator, then insertion. |
| **D — Destination** | "Recipient, amount, asset, and vault cannot be altered." | All committed inside the certified burn reason; copied to the public leaf; bound to the vault's own config on-chain. |

The decomposition matters most for the **Anchor**. The expensive part of verifying
a Unicity token the naïve way is re-verifying a BFT certificate — a quorum of
validator signatures — at *every* historical step. But Unicity is write-once and
append-only: **every state the token ever occupied is a key that still exists in
the current store.** So a *single* recent certified root `R*` already contains the
genesis, every intermediate state, and the burn. The circuit verifies the BFT
certificate for `R*` **once**, then discharges Finality and Value by cheap
inclusion proofs of individual states against `R*`. This is the witness shrink:
*N* certificate verifications become *1 certificate + N inclusion proofs*, and the
inclusion proofs are plain hash paths with a precompile behind them.

The per-transition **owner authorization** (the spend signature that makes a
transfer legitimate, not merely unique) is still checked, but each is a single
elliptic-curve verification with a precompile — cheap. So the cost profile of the
whole statement is:

```
  cost  ≈  1 × (BFT quorum verification)        // the Anchor, amortizable over a batch
        +  L × (hash-path inclusion proof)      // L = lineage length
        +  L × (single-sig authorization)       // cheap, precompiled
        +  (split sum checks)                   // arithmetic
        +  1 × (accumulator insert)             // per burn
```

with the one heavy term (the quorum verification) reduced to a single instance and,
in the batched roadmap (§10), shared across every burn in the batch.

> **A note on what "Value" needs and does not need.** The bridge's solvency
> invariant is `Σ(released) ≤ Σ(locked)`. That follows from Value + Backing alone:
> backing ties genesis value to a real lock, and split sum-checks guarantee
> children never exceed their parent. Authorization (who was allowed to transfer)
> is verified to match wallet-grade safety — so the bridge will not honor a burn
> reachable only through an unauthorized transfer — but it is the cheap part of
> the statement, not the expensive one.

---

## 3. The replay accumulator

Replay is the one fact the source chain itself must record: a Groth16 proof is
reusable bytes, so *something the vault stores* must move from "unspent" to "spent"
when it pays. The only design freedom is the **shape** of that state.

We store it as a **nullifier accumulator**: an indexed Merkle tree of every
nullifier ever released, of which the vault keeps **only the root**, a single
`bytes32 spentRoot`.

- The **nullifier** of a burn is derived from the burn state's key in the Unicity
  store: `nullifier = H(domain, vault, burnStateKey)`. Because a Unicity state is
  write-once, `burnStateKey` is globally unique and a token can reach a burn state
  exactly once — the nullifier is unique by construction, with no extra machinery.

- To release a burn, the proof supplies a **non-membership** witness showing the
  nullifier is absent from the tree under the current `spentRoot`, then performs
  the standard indexed-tree insertion and outputs the **new root**. The vault
  checks `spentRoot_old == spentRoot`, then sets `spentRoot := spentRoot_new`.

This single structure delivers every property the design wants:

| Property | How |
|---|---|
| **O(1) on-chain storage** | One root word, regardless of how many burns are ever released. |
| **No ordering** | Burns are independent; non-membership + insert works in any order. No per-account nonces, no head-of-line blocking, no "skip" escape hatch. |
| **No signatures** | The burn itself is the authorization (only the holder can burn; the reason commits the destination). Nothing extra to sign. |
| **Automatic dedup** | Releasing the same burn twice — even within one batch — fails non-membership the second time. |
| **No on-chain Merkle work** | The vault never verifies a branch; the proof asserts the root transition and the vault stores one word. |
| **Replaceable provers** | The tree is reconstructable by anyone from public release events plus the on-chain root, so a stalled prover is replaced without trust. |

The accumulator does grow — off-chain — with the number of returns; that growth is
the spent-set, and it is unavoidable in any replay-safe design. The point is that
the growth lives off-chain, is public, and is reconstructable, while the source
chain's footprint stays constant.

> **Why a single global root and no sharding.** A batch reads the current root and
> outputs the next, so batches serialize on `spentRoot`. That serialization is at
> the *batch* layer, not the user layer: one batch carries many users' returns in
> any order, and assembling batches is a liveness role that is replaceable and
> cannot steal. A single sequencing prover is sufficient; if it stalls, anyone
> rebuilds the tree from the public root and the burned-token blobs and continues.
> No per-user contention exists to shard away.

---

## 4. The bridge-in record the vault keeps

So the return circuit can prove **Backing** without an RPC call, the vault retains
a compact commitment to each lock at lock time.

```solidity
struct LockRecord {
    address asset;               // ERC-20 locked
    bytes32 tokenType;           // Unicity bridged-asset token type
    bytes32 coinId;              // value/coin id carried in the token
    uint256 amount;              // smallest unit
    bytes32 unicityTokenId;      // the token this lock funds
    bytes32 recipientCommitment; // H(minted recipient predicate)
}

mapping(uint256 => bytes32) public lockDigest; // lock nonce => digest
uint256 public nextLockNonce;
```

with

```text
lockDigest[nonce] = keccak256(abi.encode(
    "unicity-bridge-lock", sourceChainId, vault, nonce,
    asset, tokenType, coinId, amount, unicityTokenId, recipientCommitment))
```

The lock event still carries the full fields for off-chain display; the contract
only needs the digest. The return circuit takes a private `LockRecord` witness,
recomputes the digest, and proves it is one of the digests the vault stored.
Because `unicityTokenId` and `recipientCommitment` are committed at lock time and
the Unicity genesis is bound to the same `tokenId`, a lock funds exactly one token
and cannot be repurposed.

---

## 5. The burn (return authorization)

The holder burns the token to a burn state whose reason is the withdrawal
commitment:

```text
BridgeBackReason = [
  version,            // 1
  sourceChainId,
  vault,              // 20-byte EVM address
  asset,              // 20-byte ERC-20 address
  tokenType,          // 32 bytes
  coinId,             // 32 bytes
  recipient,          // 20-byte EVM address, receives amount − fee
  amount,             // gross, equals the burned token's value for coinId
  feeRecipient,       // 20-byte EVM address, zero if no fee
  feeAmount,          // ≤ amount
  deadline            // unix seconds; vault rejects after this
]
```

Every field is public by nature — the release is a public source-chain event — and
each is bound into a certified token state, so a prover cannot alter any of them.
There is **no separate signed authorization and no account nonce**: the act of
burning is the authorization, and replay is handled by the accumulator. For a
partial return, the holder splits on Unicity first and burns the child whose value
equals `amount`.

---

## 6. The proof statement

### 6.1 Public values (committed by the circuit, checked by the vault)

```rust
pub struct PublicValues {
    pub domain_tag:      [u8; 32], // keccak256("unicity-bridge-return")
    pub trust_base_hash: [u8; 32], // pins which Unicity; allow-listed on-chain
    pub source_chain_id: u64,
    pub vault:           [u8; 20],
    pub asset:           [u8; 20],
    pub token_type:      [u8; 32],
    pub coin_id:         [u8; 32],
    pub spent_root_old:  [u8; 32], // must equal vault.spentRoot
    pub spent_root_new:  [u8; 32], // vault.spentRoot := this
    pub return_root:     [u8; 32], // commits to the executed ReturnLeaf[]
    pub lock_ref_root:   [u8; 32], // commits to the SourceLockRef[] used for backing
    pub batch_size:      u32,
    pub total_amount:    U256,
}
```

### 6.2 Public calldata bound to the public values

The vault needs the decoded leaves to execute transfers, and the lock refs to
check backing:

```rust
pub struct ReturnLeaf {                 // one per released burn
    pub recipient:     [u8; 20],
    pub amount:        U256,
    pub fee_recipient: [u8; 20],
    pub fee_amount:    U256,
    pub deadline:      u64,
}

pub struct SourceLockRef { pub nonce: U256, pub digest: [u8; 32] }
```

`return_root = keccak(ReturnLeaf[] in submission order)` and
`lock_ref_root = keccak(deduplicated SourceLockRef[] sorted by nonce)`. The leaf
omits the batch-wide fields (`asset`, `token_type`, `coin_id`, `vault`,
`source_chain_id`); the vault reconstructs the full commitment from `PublicValues`
plus the leaf when it needs to. Leaf order is free — the accumulator imposes no
ordering — so the vault simply hashes the array as received.

### 6.3 Private witness

```rust
pub struct GuestInput {
    pub trust_base:       RootTrustBase,   // hashed to trust_base_hash
    pub anchor_root:      [u8; 32],        // R*: a recent certified Unicity root
    pub anchor_cert:      UnicityCertificate, // seals R* under trust_base
    pub burns:            Vec<BurnInput>,
    pub spent_root_old:   [u8; 32],
}

pub struct BurnInput {
    pub token_cbor:    Vec<u8>,            // the burned token: genesis + lineage + burn
    pub inclusions:    Vec<InclusionProof>,// one per lineage state, all against R*
    pub source_locks:  Vec<LockRecord>,   // usually one; a vector keeps merge support open
    pub nullifier_aux: NonMembershipWitness, // low-leaf witness for the accumulator
}
```

The relayer obtains `anchor_root`, `anchor_cert`, and a fresh `inclusions` set for
the whole lineage by querying Unicity for inclusion proofs against `R*` — exactly
the write-once-store primitive the network exposes.

### 6.4 The algorithm

Once per batch:

```text
A0. require H(trust_base) == trust_base_hash
A1. verify anchor_cert seals anchor_root under trust_base      // the one BFT quorum check
A2. running_spent := spent_root_old
```

For each burn (in any order):

```text
F1. token = decode(token_cbor)
F2. for every state s in the token's lineage (genesis, each transfer output,
       each split node, and the burn state):
        verify inclusion of s in anchor_root                   // hash-path proof
F3. verify chain linkage: each transition's input state == the prior output state,
       and the transition carries a valid owner authorization  // cheap single-sig

V1. genesis: decode the bridge mint reason; recompute lockDigest from the matching
       private LockRecord; require that digest ∈ lock_ref set; check
       source_chain_id / vault / asset / token_type / coin_id; check genesis value
       == LockRecord.amount; check LockRecord.unicityTokenId == genesis.tokenId and
       LockRecord.recipientCommitment == H(genesis.recipient)
V2. for each split node on the path to the burned leaf: children sum to parent
       (value conservation) and this leaf's branch carries the claimed value

D1. require token.token_type == token_type (public)
D2. burn state: decode the burn predicate; decode BridgeBackReason from its reason
D3. require reason.{source_chain_id, vault, asset, token_type, coin_id} == public
D4. require certified value for coin_id == reason.amount
D5. require reason.fee_amount <= reason.amount

R1. nullifier = H(domain_tag, vault, burnStateKey)
R2. verify non-membership of nullifier under running_spent (nullifier_aux)
R3. running_spent := insert(running_spent, nullifier)

E1. append ReturnLeaf{recipient, amount, fee_recipient, fee_amount, deadline}
E2. record the SourceLockRef(s) used by this burn
```

After the batch:

```text
C1. spent_root_new := running_spent
C2. return_root   := keccak(leaves in append order)
C3. lock_ref_root := keccak(dedup, sort(lock refs))
C4. total_amount  := Σ leaf.amount
C5. commit PublicValues
```

No nullifier, no burn-state hash, and no per-account counter ever appears in the
public values; replay is entirely captured by the `spent_root_old → spent_root_new`
transition.

---

## 7. The vault

```solidity
contract ReturnVault {
    ISP1Verifier public immutable sp1;
    bytes32 public immutable VKEY;
    bytes32 public immutable DOMAIN_TAG;
    uint64  public immutable SOURCE_CHAIN_ID;
    address public immutable ASSET;
    bytes32 public immutable TOKEN_TYPE;
    bytes32 public immutable COIN_ID;

    mapping(uint256 => bytes32) public lockDigest;       // set at bridge-in
    mapping(bytes32 => bool)    public trustBaseAllowed; // admin-rotatable (epochs), timelocked
    bytes32 public spentRoot;                            // init = EMPTY_TREE_ROOT

    function fulfillBatch(
        bytes calldata publicValues,
        bytes calldata proof,
        ReturnLeaf[] calldata leaves,
        SourceLockRef[] calldata lockRefs
    ) external {
        sp1.verifyProof(VKEY, publicValues, proof);
        P memory p = decode(publicValues);

        require(p.domainTag == DOMAIN_TAG,                 "domain");
        require(trustBaseAllowed[p.trustBaseHash],         "trust base");
        require(p.sourceChainId == SOURCE_CHAIN_ID,        "chain");
        require(p.vault == address(this),                  "vault");
        require(p.asset == ASSET,                          "asset");
        require(p.tokenType == TOKEN_TYPE && p.coinId == COIN_ID, "type");

        require(p.spentRootOld == spentRoot,               "stale root"); // the replay guard
        require(keccakLeaves(leaves) == p.returnRoot,      "leaves");
        require(leaves.length == p.batchSize,              "size");
        require(keccakLockRefs(lockRefs) == p.lockRefRoot, "lock refs");

        uint256 total;
        for (uint256 i; i < lockRefs.length; ++i)
            require(lockDigest[lockRefs[i].nonce] == lockRefs[i].digest, "lock");
        for (uint256 i; i < leaves.length; ++i) total += leaves[i].amount;
        require(total == p.totalAmount,                    "total");

        spentRoot = p.spentRootNew;                        // one SSTORE, whole batch

        for (uint256 i; i < leaves.length; ++i) {
            ReturnLeaf calldata L = leaves[i];
            require(block.timestamp <= L.deadline,         "expired");
            safeTransfer(ASSET, L.recipient, L.amount - L.feeAmount);
            if (L.feeAmount != 0) safeTransfer(ASSET, L.feeRecipient, L.feeAmount);
        }
    }
}
```

Notes:

- The vault does **not** track Unicity roots. The proof self-certifies its anchor
  root `R*` against the pinned trust base, so "which Unicity" is governed entirely
  by the `trustBaseAllowed` allow-list. Epoch rotation adds a hash under timelock;
  no light client and no redeploy.
- `require(p.spentRootOld == spentRoot)` is the complete replay defense. There is
  no nullifier mapping and no nonce mapping to grow.
- Release is **submit-and-execute**: the proof and all leaves are in one
  transaction, so the vault stores no batch roots. If a batch is too large for one
  block, split it into smaller proofs; each advances `spentRoot` in turn.

---

## 8. Security analysis

| Property | Mechanism | Residual |
|---|---|---|
| Burn is real and final | Anchor cert for `R*` + inclusion of the burn state (claims A, F) | Proof-system soundness; trust-base governance |
| Value is genuine and conserved | Genesis value + recursive split sum-checks along the lineage (claim V) | None for split lineages |
| Token is backed by a real lock | In-circuit digest recompute + vault's stored `lockDigest` (claim B) | None, assuming correct digests were stored at lock time |
| Only authorized transfers honored | Per-transition owner-authorization check over the certified lineage | None (wallet-grade); cheap, precompiled |
| Recipient / amount / asset / vault fixed | Committed in the certified burn reason, copied to the public leaf, bound to vault config (claim D) | None |
| No double release | Nullifier non-membership + insertion; `spentRoot_old == spentRoot` on-chain (claim R) | None |
| Source-chain storage is constant | One accumulator root; no nullifier set, no nonce map | One word total |
| Prover cannot steal | Vault pays only the reason's recipient/fee from a certified burn | Prover can stall |
| Prover is replaceable | Tree and proofs reconstructable from public root + burned blobs | Liveness only |
| Privacy of transfer history | Lineage is private witness; only the public withdrawal fields go on-chain | The prover sees the history it proves |

The honest one-liner: **the entire Unicity side is trustless by construction
(soundness + a pinned, governable trust base), backing is checked against data the
vault itself stored, and replay costs one word on-chain.** No operator, no
committee, no challenge window.

---

## 9. Liveness and operations

- **Ordering-free.** A stuck proof for one burn never blocks another. There are no
  per-account nonces and therefore no head-of-line blocking and no skip/recovery
  hatch.
- **Batch serialization.** Batches advance one shared `spentRoot`, so a second
  prover building on a stale root must rebase onto the new root and re-prove its
  burns. This is contention at the batch layer only; a single sequencing prover
  removes it, and that prover is permissionless and cannot steal.
- **No griefing surface.** The accumulator is written only by the proof, so no
  third party can insert junk that blocks releases. (Contrast a design that lets
  arbitrary parties register claims into a shared on-chain structure.)
- **Recovery.** If a prover disappears mid-flight, anyone reconstructs the
  off-chain tree from the on-chain `spentRoot` and public release events, refetches
  inclusion proofs from Unicity, and continues. No state is lost and nothing is
  unreleasable.
- **Deadlines.** A burn carries a `deadline`; if no proof lands in time the vault
  rejects it. Because the burn is irreversible on Unicity, wallets should expose a
  generous deadline and surface "awaiting release" until a proof lands.

---

## 10. Roadmap to batched proving

The on-chain interface and the public-value layout are **identical at every
stage** — `batch_size` is just a number — so the vault and wallet never change.
Only the prover's internals evolve.

**Stage 1 — one burn per proof (`batch_size = 1`).** The degenerate case: one
lineage, one nullifier insertion, one root transition. Establishes the interface
and the trust-base governance. On-chain cost: one Groth16 verification + one
`spentRoot` update + one transfer.

**Stage 2 — in-circuit batching (`batch_size = B`).** One proof proves `B` burns
against a **single shared anchor root `R*`**, performs `B` sequential accumulator
insertions, and emits one root transition with `B` leaves in calldata. This
amortizes the three fixed costs at once:
- the single on-chain Groth16 verification, over `B` returns;
- the STARK→Groth16 wrap (the dominant fixed proving cost), over `B` returns;
- **the single BFT quorum verification of `R*`, over `B` returns** — the largest
  in-circuit fixed cost, paid once for the whole batch because every burn is
  anchored to the same root.

On-chain cost stays O(1) in `B` apart from the `B` irreducible transfers (one root
update regardless of `B`).

**Stage 3 — recursion and parallel proving.** Prove each burn's lineage as an
independent succinct proof (embarrassingly parallel across machines/GPUs), each
emitting its nullifier and validated public facts. A lightweight **aggregation
circuit** then verifies the child proofs, performs the batch accumulator insertion
over their nullifiers, binds them to the shared `R*`, and commits the batch
`PublicValues`. This decouples per-token proving latency from batch size and turns
proving into a horizontally scalable pipeline. Per-token proving cost is dominated
by lineage length (inclusion proofs + single-sig authorizations); the heavy
quorum check is still shared once via the aggregator's anchor.

**Stage 4 — continuous accumulator.** A long-running aggregator maintains the
off-chain indexed tree, accepts burned tokens into a mempool, and posts periodic
root-advancing batch proofs on a cadence chosen by latency and source-chain cost
targets. Relayers and provers are interchangeable; the on-chain contract is
unchanged from Stage 1.

Throughout, the security argument is constant: every stage emits the same
`PublicValues`, the vault gates purely on `verifyProof` plus its stored lock
digests and single accumulator root, and the only thing that ever scales on-chain
is the unavoidable per-recipient transfer.

---

## 11. Parameters to freeze before implementation

- **`BridgeBackReason` encoding.** Version it (`version: 1`) and register its tag;
  it is consumed by both the circuit and, via the leaf, the vault.
- **Trust-base allow-list policy.** Timelock and quorum for adding/rotating
  `trustBaseHash` as Unicity validator epochs roll.
- **Anchor recency.** Any certified root containing the burn works; pick a
  prover-side policy for how recent `R*` should be (freshness of inclusion proofs
  vs. how often the network seals).
- **Accumulator instantiation.** Indexed Merkle tree depth and the hash used for
  the tree (match a precompile available in the zkVM and cheap to recompute
  off-chain).
- **Public-value ABI.** Fixed-width encoding so the vault decodes and re-hashes
  leaves and lock refs cheaply.
- **Batch limits.** Maximum `B` per source-chain transaction once Groth16
  verification, the root update, and `B` transfers are measured against the block
  gas limit.
