# ZK bridge-out: returning bridged assets to the source chain with a SNARK

**Status:** architecture + data-structure design for the zk return path
(Unicity → source EVM chain). This is the trustless end-state named in
[`BRIDGE_BACK.md`](./BRIDGE_BACK.md) (row 3) and
[`TRUST_MODELS.md`](./TRUST_MODELS.md) (§6), now made concrete on **SP1 v6.2.4 +
Groth16**. It supersedes the committee/optimistic path of
[`OPTIMISTIC_UNLOCK.md`](./OPTIMISTIC_UNLOCK.md) for the assets that move to zk;
the two can coexist behind the same vault (a per-asset config knob, §11).

First asset: **USDT on Tron** (TVM ≈ EVM; a Groth16 verifier deploys there and is
cheaper than Ethereum L1). The design is written EVM-general; Tron-specific notes
are called out.

> **Scope of this doc.** Bridge-*in* (lock-on-Tron → mint-on-Unicity) is already
> specified and trustless ([`MINT_REASON.md`](./MINT_REASON.md)) and is unchanged
> here. This doc is only the *return* leg, and only its **zk** realization. We
> assume a **trusted operator** for v1 (§4 states exactly what it is trusted
> for); later iterations narrow that trust without changing the on-chain
> interface.

---

## 1. The shape of the problem (recap)

Returning a bridged token means: **burn** the Unicity token, then **release** the
matching source-chain asset from the lock vault. Per `TRUST_MODELS.md` §1c the
two legs are asymmetric, and that asymmetry is the whole game here:

- **No source-finality wait.** A Unicity burn carries a `UnicityCertificate`
  (BFT seal); it is final the instant it exists. There is nothing to "wait out".
- **The only cost is translation.** A weak EVM contract cannot cheaply check a
  Unicity burn (re-verifying BFT certs + SMT paths + the full token history on
  the TVM costs gas that *grows with token age*). zk collapses that
  arbitrarily-long, arbitrarily-complex check into **one succinct proof of
  constant on-chain cost**.

So the statement we must prove is exactly the check a wallet runs in
[`verify_token`](../../state-transition-sdk-rust/src/verify/mod.rs) — *plus* "and
the token ends in a burn whose reason commits to this withdrawal" — compressed
into a Groth16 proof a TVM contract can verify for ~constant gas.

---

## 2. Why the Rust SDK is the right substrate

The guest program is the existing verifier, recompiled for the zkVM. The Rust SDK
([`state-transition-sdk-rust`](../../state-transition-sdk-rust/)) is `no_std`-first
specifically for this:

> The default build gives the full SDK; building with `--no-default-features`
> yields the pure `no_std` verification + decoding core intended for zkVM guests
> (SP1 / RISC0). — `src/lib.rs`

Everything the statement needs already exists and is adversarially tested:

| Need | SDK surface |
|---|---|
| Decode a self-contained token, reconstruct chain linkage | `Token::from_cbor` (`transaction/token.rs`) |
| Full history verification vs. a pinned root of trust | `verify::verify_token_with` (`verify/mod.rs`) — I1–I6 invariants |
| Accept split-minted tokens (partial returns, §5) | `payment::SplitMintJustificationVerifier` (`payment/verifier.rs`) |
| Value of the burned token (conservation) | `payment::verify_payment_token` / `Asset::value` (`payment/asset.rs`) |
| Detect the burn + read its reason | `BurnPredicate::reason` (`predicate/builtin.rs`) |
| The root of trust itself | `api::bft::RootTrustBase` (`api/bft/root_trust_base.rs`) |

No new cryptography in the guest — we *recompile* a verifier that already passes
the per-rule adversarial suite in `verify/mod.rs`'s tests.

---

## 3. Architecture overview

```
  WALLET (light)                OPERATOR / PROVER (server, trusted v1)            SOURCE EVM CHAIN (Tron)
  ───────────────               ─────────────────────────────────────            ───────────────────────
  1. burn token to                                                               ┌───────────────────────┐
     BurnPredicate(reason) ───►  2. intake: receive burned Token CBOR + recipient│   UnicityLock /        │
     reason = {chainId,             3. PRE-CHECK off-chain (the trusted step):    │   ReleaseVault         │
       recipient, amount}              Token.verify(..) WITH RPC mint-justif.     │                        │
                                       check (is it a real, backed, final burn?)  │  - SP1 Groth16 verifier│
                                    4. dedupe by nullifier; enqueue               │  - pinned: vkey,       │
                                    5. batch (iter 2): build Merkle tree of leaves│    trustBaseHash,      │
                                    6. SP1 prove guest → STARK → wrap Groth16     │    tokenType, chainId  │
                                    7. submit(publicValues, proof) ──────────────►│  - nullifier set       │
                                                                                  │  - USDT pool           │
  8. (batch) claim(leaf, branch) ◄──────── recipient or operator relays ─────────│  → transfer USDT to    │
                                                                                  │    recipient           │
                                                                                  └───────────────────────┘
```

**Components**

- **Guest program** (`zk-bridge-guest`, SP1 RISC-V ELF): links the Rust SDK
  `no_std` core; implements the statement of §6. One ELF per *asset family*
  (Tron-TRC20); its **vkey** is pinned on-chain. Per-asset parameters
  (`tokenType`, `chainId`, `lockContract`) are *public inputs*, not baked in, so
  one ELF serves every TRC20 pool (USDT, USDC, …).
- **Prover/operator service** (`zk-bridge-operator`, Rust + `sp1-sdk`):
  intake → off-chain pre-check (the trusted RPC step) → dedupe → batch → prove →
  wrap → submit. Stateless except a durable queue + nullifier cache (§9).
- **`ReleaseVault`** (Solidity/TVM): the on-chain endpoint. Verifies the Groth16
  proof via the SP1 verifier, checks the proof's pinned public inputs against its
  config, records the batch `merkleRoot`, and releases USDT against unspent
  nullifiers (§10).
- **Wallet plugin** (TS, extends [`PLUGIN_ARCHITECTURE.md`](./PLUGIN_ARCHITECTURE.md)):
  builds the burn transition, computes the expected nullifier, calls the operator
  intake API, and shows return status. Light — it never proves.

---

## 4. The trust boundary (read this before §6)

A wallet's `Token.verify` for a bridged token runs the bridge **mint-justification
verifier**, which calls a **Tron RPC node** to confirm the lock event is real and
final (`MINT_REASON.md` steps 3–6). **A zkVM guest cannot make an RPC call.** This
is the one thing zk cannot internalize cheaply, and pretending otherwise would be
dishonest. So we split the check in two:

| Claim | v1: who proves it | How it could become trustless (§12) |
|---|---|---|
| **U — Unicity side is valid & final.** The blob is a real token: unbroken certified history from the pinned `RootTrustBase`, ending in a `BurnPredicate` whose reason commits to `{chainId, recipient, amount}`, and `amount` equals the token's declared bridged-coin value. | **The zk proof** (in-circuit `verify_token_with`). | unchanged |
| **B — the token is *backed*.** Its genesis mint-justification points at a real, final, sufficiently-confirmed source lock (the RPC check). | **The operator, off-chain** (step 3): it runs the *full* `Token.verify` including the RPC mint-justification check before admitting a burn to a batch. | replace the RPC check with an in-circuit **source-chain light-client / receipt-inclusion** proof (verify a Tron block header + the `Lock` log's receipt-trie path), pinning the source chain's trust anchor as another public input. Heavy but mechanical; it turns B into part of the SNARK and retires the operator. |

So **v1 is "trusted operator for backing (B), trustless for the Unicity-side burn
(U)"**. The operator can *stall* or *refuse*, and (because it picks the batch) it
could in principle admit an *unbacked* token it minted itself — but it gains
nothing by stalling, and an unbacked admission is publicly auditable (anyone can
re-run the RPC mint-justification check on any leaf in a posted batch and slash a
bonded operator). It can **never forge U**: the SNARK makes a fake or non-final
Unicity burn unrepresentable. Compare `OPTIMISTIC_UNLOCK.md`: there a quorum
attests U and a fraud proof falsifies it; here the SNARK *is* U, and only B is
left to an operator that later iterations remove.

This boundary is also why pool accounting is safe under a trusted operator (§5).

---

## 5. Accounting model: pool + nullifier (not per-nonce)

The existing `UnicityLock` does **per-deposit** accounting: `unlock(nonce, to,
amount)` releases exactly `locks[nonce].amount` and flips `withdrawn[nonce]`
(`contracts/tron/contracts/UnicityLock.sol`). That model **cannot express partial
returns**, and partial returns are the common case:

- A user bridges 1000 USDT → one token, `tokenId = X`, bound on-chain to
  `locks[N].unicityTokenId = X`.
- They want to return **400**. On Unicity they **split** first (already
  supported): the parent `X` is consumed; two *new* tokens are minted with *new*
  tokenIds `Y` (400) and `Z` (600), each carrying a `SplitMintJustification`.
- They burn `Y`. But `Y` is bound to **no** lock nonce — `locks[N].unicityTokenId
  == X ≠ Y`. Per-nonce release can never settle `Y`. Dead end.

So the zk path uses a **pool + nullifier** model, which is both simpler and
partial-native:

- The vault holds a **pooled** USDT balance (the sum of all locks for that asset).
- A validly-burned bridged token of value `v` authorizes releasing **`v`** from
  the pool to the committed `recipient`, regardless of split ancestry.
- Double-withdraw is prevented by a **nullifier set**, not by per-nonce flags.

**Why this is safe.** Value is conserved end-to-end: every bridged token in
circulation is backed by an equal locked amount (bridge-in guarantees it; the
in-circuit split verifier guarantees split children sum to the parent — see
`verify_payment_token`'s `root_sum` check, `payment/verifier.rs:276`). Therefore
`Σ(valid burns) ≤ Σ(locks) = pool balance`; valid burns can never drain the pool
below zero. The only way to over-release is to burn an **unbacked** token — which
is exactly claim **B**, the operator's trusted off-chain job (§4). Under a trusted
operator, pool accounting is sound; when B becomes a SNARK (§12), it is
trustless.

> **Nullifier.** `nullifier = result_state_hash of the burn transition` — the
> hash of the final (burn) state. It is globally unique: the Unicity aggregator
> certifies **at most one** spend per state (`TRUST_MODELS.md` P2), and the state
> hash binds `tokenId` + the burn predicate. A token can reach a burn state once.
> This is the `burnStateHash` of `OPTIMISTIC_UNLOCK.md`, reused as the on-chain
> replay key.

The per-nonce `UnicityLock` remains valid for **whole-token, no-split** returns
and for the optimistic path; the `ReleaseVault` below is the pool sibling. A
deployment can run both (governance routes an asset to one or the other).

---

## 6. The statement (guest program)

### 6.1 Public inputs (committed by the guest; checked on-chain)

Committed via SP1 `commit` into the public-values digest the Groth16 wrapper
exposes. The vault checks the pinned ones against its config and consumes
`merkleRoot`.

| Field | Bytes | Meaning / why public |
|---|---|---|
| `domainTag` | 32 | `keccak256("unicity-zk-back:v1")` — separates this circuit's commitments. |
| `trustBaseHash` | 32 | `H(canonical RootTrustBase)` (validator set + threshold + `networkId` + epoch). Pins **which Unicity** is authoritative. On-chain allowlist, admin-rotatable (§12). |
| `assetTokenType` | 32 | The bridged `TokenType` (= `SHA256("unicity-bridge:tron:<chainId>:<assetHex>")`, `MINT_REASON.md`). Pins **which asset/pool**; a proof for USDT can't drain USDC. |
| `sourceChainId` | 4/8 | Tron network id (`0x2b6653dc` mainnet). Pins the destination chain. |
| `lockContract` | 20 | The canonical vault address bound into the burn reason. A proof can't be replayed against a different vault. |
| `merkleRoot` | 32 | Root of the batch tree over withdrawal leaves (§6.4). Iteration 1: a single-leaf tree (root = leaf). |
| `batchSize` | 4 | Number of leaves; bounds the on-chain claim space and lets the vault sanity-check. |

`assetTokenType`, `sourceChainId`, `lockContract` are **inputs, not constants**,
so one guest ELF (one vkey) serves all TRC20 pools; the vault pins the specific
values per deployment.

### 6.2 Private inputs (witness; never leave the guest)

| Field | Meaning |
|---|---|
| `trustBase: RootTrustBase` | The full trust base; the guest hashes it and asserts `H(trustBase) == trustBaseHash`. (Alternative: bake it into the ELF so the vkey commits to it — rejected, because epochs rotate; see §12.) |
| `burns: Vec<BurnInput>` | One per returned asset (batch). Each holds the **entire** self-contained token. |
| └ `token_cbor: Vec<u8>` | The burned `Token` (genesis + full transfer history + the burn transition + every `InclusionProof`/`UnicityCertificate`). For split children this transitively embeds parent tokens via the split justification. This is the bulk of the witness. |

The burn `reason` and the token value are **inside** `token_cbor`; nothing about
the withdrawal is trusted from outside the certified bytes.

### 6.3 Per-burn algorithm (the heart)

For each `BurnInput` the guest does **exactly** what a full-node wallet does,
minus the RPC step (claim B, §4):

```
1.  token = Token::from_cbor(token_cbor)              // reconstructs chain linkage
2.  let mut registry = MintJustificationRegistry::new();
    registry.register(SplitMintJustificationVerifier::new());        // split-minted children
    registry.register(BridgeMintJustificationVerifier::structural());// see note below
3.  // verify_payment_token runs full history verification (I1–I6: certs, quorum, SMT,
    // sigs, history) AND returns the certified value collection in one call.
    assets = verify_payment_token(&token, &trustBase, &registry, decode_payment_data)?
4.  assert token.token_type() == assetTokenType
5.  (state_hash, lock_script) = token.latest_state()
    burn = BurnPredicate::from_encoded(lock_script)?  // FAIL if latest state isn't a burn
    {ver, chainId, lockContract', recipient, amount} = decode_reason(burn.reason())
6.  assert ver == 1 && chainId == sourceChainId && lockContract' == lockContract
           && recipient is 20-byte && amount > 0
7.  // value conservation: the certified bridged-coin value == amount in the reason
    v = assets.value_of(assetCoinId)                  // sum for our AssetId
    assert v == amount
8.  nullifier = state_hash                            // unique per single-spend
9.  leaf = keccak256(abi.encode(recipient, amount, nullifier))
```

> **`BridgeMintJustificationVerifier::structural()`** is the in-circuit stand-in
> for the RPC verifier: it accepts the genesis bridge justification by checking
> only what is internally checkable (tag, version, and that the justification's
> own committed `tokenId`/`recipientCommitment`/`amount` are self-consistent with
> the certified mint) and **does not** call Tron. It exists so `verify_token_with`
> doesn't fail-closed on a real bridged genesis. The skipped RPC check is claim
> **B**, discharged off-chain by the operator (§4). When B is internalized (§12)
> this is replaced by the source-chain light-client verifier and the structural
> stand-in is removed.

### 6.4 Aggregation + commit

```
10. merkleRoot = MerkleTree_keccak(sorted_or_indexed leaves)   // SP1 keccak precompile
11. commit(domainTag, trustBaseHash, assetTokenType, sourceChainId,
           lockContract, merkleRoot, batchSize)
```

Leaves use **keccak** (not SHA-256) so on-chain Merkle-branch verification in the
claim path is cheap; the SP1 keccak precompile keeps it cheap in-circuit too.
Iteration 1 sets `batchSize = 1` and `merkleRoot = leaf` (no tree). The host then
wraps STARK → Groth16 (`SP1_PROVER=groth16`), producing the ~constant-cost proof.

### 6.5 What each rule defends (statement ↔ attack)

| Attack | Caught by |
|---|---|
| Forged/edited token, broken history, double-spent state | step 3 (`verify_payment_token` I1–I6; nullifier in step 8) |
| Burn of the wrong asset to drain this pool | step 4 (`token_type == assetTokenType`) |
| "Withdraw" a token that wasn't actually burned | step 5 (latest state must be `BurnPredicate`) |
| Inflate `amount` above the token's real value | step 7 (`v == amount`, certified value) |
| Replay the same burn (in another batch / claim) | step 8 nullifier + on-chain nullifier set (§10) |
| Replay a proof against a different vault/chain/asset | public inputs §6.1, checked on-chain |
| Mint an unbacked bridged token and burn it | **not** caught in-circuit → claim **B**, operator off-chain (§4); SNARK in §12 |

---

## 7. Data structures

### 7.1 Guest I/O (Rust, `no_std`)

```rust
// committed public values; ABI-encoded so the EVM vault can decode the digest preimage
pub struct PublicValues {
    pub domain_tag:      [u8; 32],
    pub trust_base_hash: [u8; 32],
    pub asset_token_type:[u8; 32],
    pub source_chain_id: u64,
    pub lock_contract:   [u8; 20],
    pub merkle_root:     [u8; 32],
    pub batch_size:      u32,
}

pub struct BurnInput { pub token_cbor: Vec<u8> }   // self-contained burned Token

pub struct GuestInput {                            // SP1 stdin
    pub trust_base: RootTrustBase,                 // hashed to trust_base_hash
    pub asset_token_type: [u8; 32],
    pub source_chain_id: u64,
    pub lock_contract: [u8; 20],
    pub asset_coin_id: [u8; 32],                   // AssetId for the value check (step 6)
    pub burns: Vec<BurnInput>,
}

// decoded burn reason (CBOR inside BurnPredicate.reason), the bridge-out commitment
// reason = #[ ver:uint, chainId:uint, recipient:bstr(20), amount:bstr(uint) ]
pub struct BurnReason { pub ver: u64, pub chain_id: u64, pub recipient: [u8;20], pub amount: U256 }

pub struct WithdrawalLeaf { pub recipient: [u8;20], pub amount: U256, pub nullifier: [u8;32] }
// leaf hash = keccak256(abi.encode(recipient, amount, nullifier))
```

### 7.2 Burn reason (what the wallet writes, §10 reads)

The burn `reason` is the **bridge-out commitment**, symmetric to the bridge-in
`Lock` event. It binds the destination so a burn can't be redirected:

```
reason = CBOR #[ version:1, chainId, lockContract(20B), recipient(20B), amount ]
```

(`lockContract` in the reason lets the vault assert the holder intended *this*
vault; the guest copies it to the `lockContract` public input and the vault
checks equality.) This refines the `burnReason` sketch in `BRIDGE_BACK.md`
(dropping `lockNonce`, which the pool model doesn't use — §5).

### 7.3 On-chain (Solidity / TVM)

```solidity
struct Batch {            // one posted proof
    bytes32 merkleRoot;
    uint32  batchSize;
    uint32  claimed;      // monotone; informational
}
mapping(bytes32 => Batch) public batches;       // merkleRoot => batch
mapping(bytes32 => bool)  public nullifierUsed; // burn state hash => spent
```

### 7.4 Operator API (wallet ↔ operator, TS)

```ts
// POST /return  — wallet submits a burned token for return
interface ReturnRequest { tokenCbor: string /*hex*/; }   // recipient/amount are inside the burn reason
interface ReturnResponse {
  nullifier: string;                                       // recipient can track this
  status: "queued" | "rejected";
  reason?: string;                                         // e.g. "mint-justification not final"
}
// GET /return/:nullifier  — status
interface ReturnStatus {
  status: "queued" | "proving" | "submitted" | "released" | "rejected";
  batchRoot?: string; proofTxId?: string; releaseTxId?: string;
  merkleBranch?: string[];                                 // for self-service claim (batch)
}
```

---

## 8. Prover / operator component design

A pipeline of small, restartable stages around a durable queue. Wallets are
light; **all proving is server-side**, local CPU, no GPU initially.

1. **Intake** (`POST /return`). Decode `tokenCbor`; reject malformed early.
2. **Pre-check (the trusted step, claim B).** Run the **full** host
   `Token::verify` *with* the real RPC `BridgeMintJustificationVerifier`
   (`bridge-plugin-tron-usdt` logic) **and** `verify_payment_token`. This catches
   unbacked/non-final/under-confirmed burns *before* spending proving cycles, and
   is the operator's auditable responsibility. Compute `nullifier`; reject if
   already in the nullifier cache or on-chain.
3. **Enqueue.** Persist `{tokenCbor, nullifier, recipient, amount}` to the durable
   queue (survives restarts; idempotent by nullifier).
4. **Batch** (iteration 2). Close a batch on `size ≥ B_max` **or** `age ≥ T_max`
   (e.g. 64 burns or 10 min — tune to proving time vs. latency). Build the
   keccak Merkle tree; remember each leaf's branch.
5. **Prove.** `sp1_sdk`: execute for a cycle estimate, then
   `prove().compressed()` → `.groth16()`. The witness is the concatenated token
   blobs; cycle count ≈ Σ per-token (cert/quorum/SMT/sig + history length). Cache
   the proving key; pin `SP1_VERIFIER_VERSION`.
6. **Submit.** Relayer posts `submitBatch(publicValues, proofBytes)` to the vault
   (pays Tron energy). On success, persist `batchRoot` + `proofTxId`.
7. **Settle.** Either the operator relays each `claim` (push; operator pays gas)
   or recipients pull via `GET /return/:nullifier` → `merkleBranch` and call
   `claim` themselves. Mark `released`.

**Sizing & cost.** Per-token proving is dominated by secp256k1 ECDSA
verifications (one per transfer + per seal signature) and SHA-256 over the SMT
paths/CBOR — both have SP1 precompiles, so budget realistically but expect
**minutes per proof** on CPU for a young token, scaling with history length and
`quorum_threshold`. This is the lever for **batching** (iteration 2): one Groth16
verification on-chain amortized over `B` returns, and one proving run amortizing
the fixed STARK→Groth16 wrap (~the dominant fixed cost). Groth16 verification on
Tron is the ~constant on-chain cost the whole design buys (cheaper than the
~$10 Ethereum L1 figure). GPU proving is a later throughput knob, not a
correctness change.

**Failure modes.** Operator can stall (liveness only; funds never move wrongly —
the vault gates on the SNARK). A crash mid-pipeline is safe: stages are
idempotent on `nullifier`, and nothing is released until a proof verifies
on-chain. A bad batch (operator admitted an unbacked token) is **publicly
auditable** — anyone re-runs the RPC mint-justification check on a posted leaf —
which is what makes operator bonding/slashing meaningful before §12 lands.

---

## 9. Two iterations

| | **Iteration 1 — one proof per return** | **Iteration 2 — one proof per batch** |
|---|---|---|
| `batchSize` | 1 | up to `B_max` |
| `merkleRoot` | = single leaf (no tree) | keccak Merkle root over leaves |
| Guest | per-burn algorithm once | loop §6.3, then build tree §6.4 |
| On-chain | `submitBatch` releases immediately (or one `claim`) | `submitBatch` stores root; `claim(leaf, branch)` per recipient |
| Cost | one Groth16 verify **per return** | one Groth16 verify **per batch** + cheap Merkle claims |
| Latency | proving time only | + batch window `T_max` |
| Use when | low volume / launch | volume justifies amortization |

Crucially the **on-chain interface and the public-values layout are identical**
(iteration 1 is the degenerate `batchSize = 1` case), so the vault and wallet
plugin don't change between iterations — only the operator's batching and the
guest's loop bound do.

---

## 10. The release vault (on-chain)

Reuses the SP1 Groth16 verifier (`ISP1Verifier.verifyProof(vkey, publicValues,
proofBytes)`), pins the asset/chain/trust-base, and enforces single-spend.

```solidity
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

interface ISP1Verifier {
    function verifyProof(bytes32 vkey, bytes calldata publicValues, bytes calldata proof) external view;
}

contract ReleaseVault {
    ISP1Verifier public immutable sp1;
    bytes32 public immutable VKEY;           // pins the guest ELF (the circuit)
    bytes32 public immutable ASSET_TYPE;     // pins the bridged TokenType
    uint64  public immutable SOURCE_CHAINID;
    bytes32 public immutable DOMAIN_TAG;
    address public immutable POOL_ASSET;     // USDT TRC20
    mapping(bytes32 => bool) public trustBaseAllowed;   // admin-rotatable (epochs)
    mapping(bytes32 => bool) public nullifierUsed;
    mapping(bytes32 => uint32) public batchSize;        // merkleRoot => size (0 = unknown)

    event BatchPosted(bytes32 indexed merkleRoot, uint32 batchSize);
    event Released(bytes32 indexed nullifier, address to, uint256 amount);

    // ---- post a verified batch root ----
    function submitBatch(bytes calldata publicValues, bytes calldata proof) external {
        sp1.verifyProof(VKEY, publicValues, proof);     // reverts unless the SNARK is valid
        ( bytes32 domain, bytes32 trustBaseHash, bytes32 assetType, uint64 chainId,
          address lockContract, bytes32 merkleRoot, uint32 size ) = _decode(publicValues);
        require(domain == DOMAIN_TAG, "domain");
        require(trustBaseAllowed[trustBaseHash], "trust base");
        require(assetType == ASSET_TYPE && chainId == SOURCE_CHAINID, "asset/chain");
        require(lockContract == address(this), "wrong vault");
        require(batchSize[merkleRoot] == 0 && size > 0, "dup/empty batch");
        batchSize[merkleRoot] = size;
        emit BatchPosted(merkleRoot, size);
    }

    // ---- claim one withdrawal against a posted root ----
    function claim(
        bytes32 merkleRoot, address recipient, uint256 amount, bytes32 nullifier,
        bytes32[] calldata branch
    ) external {
        require(batchSize[merkleRoot] != 0, "no batch");
        require(!nullifierUsed[nullifier], "spent");
        bytes32 leaf = keccak256(abi.encode(recipient, amount, nullifier));
        require(_verifyBranch(merkleRoot, leaf, branch), "bad branch");
        nullifierUsed[nullifier] = true;                // single redemption
        emit Released(nullifier, recipient, amount);
        _safeTransfer(POOL_ASSET, recipient, amount);   // from the pool
    }
    // _decode / _verifyBranch / _safeTransfer / admin(setTrustBaseAllowed, timelocked) elided
}
```

Notes:
- **`require(lockContract == address(this))`** binds the burn reason to this exact
  vault — a proof minted for one deployment can't drain another.
- **`trustBaseAllowed`** is a small admin-rotatable set (not a single immutable)
  so the Unicity validator-set epoch can roll without redeploying; rotation is
  timelocked, like the verifier-rotation hatch in `OPTIMISTIC_UNLOCK.md` §4.
- **Iteration 1** can fold `submitBatch`+`claim` into one call (`size == 1`,
  `merkleRoot == leaf`).
- This is a **distinct contract** from `UnicityLock`; or the same vault gains a
  `setZkVerifier(address)` and routes pool releases through it, leaving
  bridge-*in* (`lock`) untouched.

---

## 11. Security analysis

| Property | Mechanism | Residual trust |
|---|---|---|
| Burn really happened & is final | in-circuit `verify_token_with` → certs + quorum + SMT + sigs (`verify/mod.rs`) | none (soundness of SP1+Groth16) |
| Right asset, right chain, right vault | public inputs pinned on-chain (§6.1, §10) | none |
| No value inflation | in-circuit `v == amount` certified value (step 6) | none |
| No double-withdraw | nullifier = burn state hash; on-chain `nullifierUsed` + Unicity single-spend (P2) | none |
| Partial returns | split in-circuit verifier + pool accounting (§5) | none |
| **Token is backed by a real lock (B)** | operator off-chain RPC pre-check (§4) | **trusted operator (v1)** → SNARK light-client (§12) |
| Liveness | multiple relayers, operator rotation/bonding | operator can stall, not steal |
| Privacy | only `{recipient, amount, nullifier}` go on-chain; the token history stays in the witness | history is revealed to the *operator* (it verifies it); never on-chain |

The honest one-liner: **everything about the Unicity side is trustless by
construction; the single residual trust is "the operator only admits backed
tokens," which is auditable now and removable later.** That is strictly stronger
than the optimistic committee (which trusts a quorum for the *whole* claim U+B
inside a challenge window).

---

## 12. Toward fully trustless (removing the operator)

The migration is **invisible to the vault and wallets** — same public-values
layout, same `claim` — it only changes the guest and a new public input:

1. **Internalize claim B.** Add a per-asset **source-chain inclusion verifier** to
   the guest: given a Tron block header (or a recent header the vault trusts) and
   a receipt-trie Merkle path to the genesis's `Lock` event, verify in-circuit
   that the lock exists, has ≥ K confirmations, and commits to the token's
   `tokenId`/`recipientCommitment`/`amount`. Pin the **source chain's** trust
   anchor as a new public input. This turns the structural stand-in (§6.3) into a
   real proof of B; the operator becomes a pure, untrusted prover/relayer.
2. **Trust-base rotation.** Keep `trustBaseHash` a public input with an on-chain
   allowlist (already in §10) rather than baking the trust base into the vkey, so
   Unicity epochs roll without recompiling the circuit.
3. **vkey governance.** Circuit upgrades rotate `VKEY` behind the same timelock as
   `trustBaseAllowed`; old roots remain claimable. Publish the ELF + vkey so
   anyone can reproduce the build (the trustlessness of a SNARK is only as good as
   a reproducible circuit).

End state: the vault gates purely on `verifyProof`, and the proof attests **U and
B** with no trusted party — the `TRUST_MODELS.md` §6 endgame, reached without ever
changing the on-chain interface.

---

## 13. Open questions

- **Header source for §12.1** — does the guest take a Tron header as witness with
  the vault pinning a recent header/committee, or a full SR-set light client? Sets
  the cost of going trustless.
- **Reason encoding freeze** — lock `reason` CBOR (§7.2) is consumed by both the
  guest and the vault; version it now (`version:1`) and add a registry tag like
  the mint justification.
- **`asset_coin_id` derivation** — confirm the bridged `coinId`
  (`SHA256("unicity-bridge-coin:tron:…")`, `MINT_REASON.md`) is the `AssetId`
  `verify_payment_token` expects, so step 6's value check is exact.
- **Batch close policy** — `B_max`/`T_max` tuning vs. CPU proving time; whether to
  prove incrementally (recursion) instead of one big batch circuit.
- **Pool solvency monitoring** — off-chain invariant `Σ released ≤ Σ locked`;
  alarm if an operator ever admits an unbacked burn (pre-§12 safety net).
- **Coexistence** — per-asset routing between this `ReleaseVault` (pool/zk) and
  the per-nonce `OptimisticUnlock`, exposed as plugin-manifest config.
```
