# ZK bridge-back v2: return without per-burn nullifiers

**Status:** design proposal for the no-nullifier bridge-out path.

This document refines [`ZK_BACK.md`](./ZK_BACK.md). It keeps the same end goal:
return bridged Unicity tokens to the source chain by proving a Unicity burn in
SP1 v6.2.4 and verifying the Groth16 proof on the source EVM/TVM chain. The
change is the replay-prevention primitive.

`ZK_BACK.md` used:

```text
nullifier = burn result state hash
mapping(bytes32 => bool) nullifierUsed
```

This version removes that permanent per-burn nullifier set. A return instead
spends a source-chain **return nonce** owned by a source-chain account:

```text
ReturnAuthorization = { account, lane, nonce, recipient, amount, fee, deadline, asset, vault }
nextReturnNonce[account][lane] += 1
```

The burned Unicity token commits to that authorization. The zk proof proves the
burn and emits the authorization as public data. The vault checks the account
signature and the current nonce, increments the nonce, and transfers the locked
asset. Replaying the same proof fails because the account nonce has advanced.

There is still replay state on the source chain, because a source-chain contract
cannot safely release fungible pool assets from a reusable proof with no state at
all. The important difference is that this state is **one monotone counter per
source account/lane**, not an ever-growing set of burned-token ids.

---

## 1. Design goals

- Keep the bridge-out prover replaceable and trustless. A prover may stall, but
  cannot redirect or fabricate a withdrawal.
- Do not ask the source contract to call RPC or trust an operator for the
  original bridge-in lock. The return proof must be checkable against data the
  source vault already stores.
- Support token splits. A returned token may be a split descendant of the token
  originally minted from a source-chain lock.
- Avoid permanent per-withdrawal source-chain storage where possible. Source
  chain resource use is the primary optimization target.
- Keep private Unicity transfer history private. Source-chain operations, mint
  reasons, and bridge-out authorizations are public.

---

## 2. The key observation

The source contract must consume **some** scarce object when it releases USDT.
Otherwise the same valid Groth16 proof can be submitted repeatedly.

The scarce object should not be:

- a per-burn nullifier, because that creates permanent storage growth;
- a per-original-lock remaining balance, because partial split returns make
  replay possible until the whole lock is drained;
- a spent interval/range set, because it recreates a nullifier-like spent set in
  a more complex form;
- a global "processed Unicity block" cursor, because that would require proving
  completeness of all bridge-out burns in each Unicity interval.

The smallest practical source-chain object is an **account nonce**, the same
shape used by permits and meta-transactions. It has the replay property we need
and storage grows with active source accounts, not with withdrawals.

---

## 3. Bridge-in flow, with return-friendly source records

Bridge-in is still: lock on Tron, mint on Unicity, and carry a mint reason that
lets future recipients validate the lock.

The source vault should additionally store a compact commitment to the lock
record so the return circuit can prove backing without an RPC call.

```solidity
struct LockRecord {
    address assetContract;       // USDT for the first deployment
    bytes32 tokenType;           // Unicity bridged asset token type
    bytes32 coinId;              // payment asset id used in token data
    uint256 amount;              // smallest USDT unit
    bytes32 unicityTokenId;
    bytes32 recipientCommitment; // SHA256(minted recipient predicate CBOR)
}

mapping(uint256 => bytes32) public lockDigest; // lock nonce => digest
uint256 public nextLockNonce;
```

Canonical digest:

```text
lockDigest[nonce] =
  keccak256(abi.encode(
    "unicity-bridge-lock:v1",
    sourceChainId,
    address(this),
    nonce,
    assetContract,
    tokenType,
    coinId,
    amount,
    unicityTokenId,
    recipientCommitment
  ))
```

The `Lock` event still carries the full public fields for wallet validation over
RPC. The contract only needs to retain the digest and any accounting fields it
already needs for the pool.

Mint reason remains compatible with the existing Tron USDT plugin: it references
the lock transaction/log/nonce and canonical chain/contract/asset. For the return
proof, the guest uses the mint reason plus a private `LockRecord` witness and
the public `lockDigest[nonce]` checked by the source vault.

---

## 4. Bridge-out flow

### 4.1 Return authorization

Before burning, the wallet reads `nextReturnNonce[account][lane]` from the
source vault and builds:

```solidity
struct ReturnAuthorization {
    uint8   version;          // 1
    uint256 sourceChainId;
    address vault;
    address assetContract;
    bytes32 tokenType;
    bytes32 coinId;
    address account;          // nonce authority, usually the recipient wallet
    uint32  lane;             // 0 for normal wallets; more lanes for LPs/bots
    uint64  nonce;            // must equal nextReturnNonce[account][lane]
    address recipient;        // receives amount - fee
    uint256 amount;           // gross amount burned on Unicity
    address feeRecipient;     // zero address if no fee
    uint256 feeAmount;
    uint64  deadline;
}
```

The source account signs the EVM/Tron typed-data hash of this structure. The
signature is checked on-chain, not in the zkVM.

The signature domain includes `sourceChainId` and `vault`. The signed struct
includes the global asset fields (`assetContract`, `tokenType`, `coinId`) and
the per-return fields (`account`, `lane`, `nonce`, `recipient`, `amount`,
`feeRecipient`, `feeAmount`, `deadline`). A signature for one asset, vault, lane,
or nonce is therefore not reusable for another.

`lane` is optional but useful. A normal wallet can use lane `0` and serialize its
returns. A high-throughput relayer or liquidity provider can use several lanes
without creating a per-withdrawal replay map.

### 4.2 Burn reason

The wallet burns the Unicity token to a `BurnPredicate` whose reason is the CBOR
encoding of the return authorization without the signature:

```text
#tag(BRIDGE_BACK_REASON_TAG) [
  version,
  sourceChainId,
  vault,
  assetContract,
  tokenType,
  coinId,
  account,
  lane,
  nonce,
  recipient,
  amount,
  feeRecipient,
  feeAmount,
  deadline
]
```

The burn reason is public by nature and the return itself is a public source-chain
event, so there is no reason to hide these fields. The private part is the
intermediate Unicity transfer history, which stays inside the zk witness.

For a partial return, the wallet first splits the token on Unicity and burns the
split child whose value equals `amount`.

### 4.3 Proving and fulfilment

1. Wallet sends the burned token CBOR, the source lock record witness, and the
   account signature to any prover.
2. The prover builds an SP1 proof that the token is valid, backed by the source
   lock record, and currently burned to the return authorization.
3. The prover submits `fulfillBatch(publicValues, proof, leaves, lockRefs,
   signatures)` to the source vault.
4. The vault verifies the Groth16 proof, checks each source lock digest against
   local storage, checks each account signature and nonce, increments the nonces,
   and transfers USDT.

The prover is stateless from the security point of view. It can cache jobs and
proving keys for performance, but all correctness checks are in the proof and in
source-chain state.

---

## 5. ZK statement

### 5.1 Public values

The SP1 public values commit to:

```rust
pub struct PublicValues {
    pub domain_tag: [u8; 32],        // keccak256("unicity-zk-back:v2")
    pub trust_base_hash: [u8; 32],   // hash of Unicity RootTrustBase
    pub source_chain_id: u64,
    pub vault: [u8; 20],
    pub asset_contract: [u8; 20],
    pub token_type: [u8; 32],
    pub coin_id: [u8; 32],
    pub return_root: [u8; 32],       // Merkle/hash root of ReturnLeaf list
    pub lock_ref_root: [u8; 32],     // root of unique SourceLockRef list
    pub batch_size: u32,
    pub total_amount: U256,
}
```

`return_root` binds the public leaves the contract will execute. `lock_ref_root`
binds the source lock records the proof used for bridge-in backing.

### 5.2 Public calldata checked against public values

The contract receives the decoded public leaves because it must execute transfers
and nonce updates:

```rust
pub struct ReturnLeaf {
    pub account: [u8; 20],
    pub lane: u32,
    pub nonce: u64,
    pub recipient: [u8; 20],
    pub amount: U256,
    pub fee_recipient: [u8; 20],
    pub fee_amount: U256,
    pub deadline: u64,
}

pub struct SourceLockRef {
    pub nonce: U256,
    pub digest: [u8; 32],
}
```

The contract recomputes `return_root` from `ReturnLeaf[]` and `lock_ref_root`
from `SourceLockRef[]`, then checks that each `SourceLockRef.digest` equals
`lockDigest[SourceLockRef.nonce]`.

`ReturnLeaf` omits the batch-wide asset fields to keep calldata compact. The
contract reconstructs the signed `ReturnAuthorization` from `PublicValues` plus
the leaf before calling `ecrecover`.

### 5.3 Private witness

```rust
pub struct GuestInput {
    pub trust_base: RootTrustBase,
    pub burns: Vec<BurnInput>,
}

pub struct BurnInput {
    pub token_cbor: Vec<u8>,
    pub source_locks: Vec<LockRecord>, // usually one; vector keeps merge support possible
}
```

Current split-only tokens normally trace to one original source lock. The vector
keeps the statement ready for a future merge/combine operation where one returned
token may be backed by several source locks.

### 5.4 Per-token verification algorithm

For each burned token:

```text
1. Decode Token::from_cbor(token_cbor).
2. Verify the full token history against trust_base using:
   - SplitMintJustificationVerifier for split descendants.
   - BridgeMintJustificationVerifier::source_record_backed for bridge genesis.
3. In the bridge mint verifier:
   a. Decode the Tron/USDT lock mint reason.
   b. Find the matching private LockRecord by lock nonce.
   c. Recompute its lockDigest and require it is in the public SourceLockRef set.
   d. Check chain, vault, asset, tokenType, coinId.
   e. Check LockRecord.unicityTokenId == genesis.tokenId.
   f. Check LockRecord.recipientCommitment == SHA256(genesis.recipient.toCBOR()).
   g. Check LockRecord.amount equals the genesis payment amount.
4. Check token.token_type == public token_type.
5. Decode the latest state as BurnPredicate.
6. Decode BridgeBackReason from burn.reason.
7. Check reason fields match public chain/vault/asset/tokenType/coinId.
8. Check the certified payment amount for coinId equals reason.amount.
9. Check reason.feeAmount <= reason.amount.
10. Emit ReturnLeaf from the reason.
```

After all burns:

```text
11. Sort leaves by (account, lane, nonce) and reject duplicates.
12. Compute return_root over the sorted leaves.
13. Deduplicate source lock refs, sort by lock nonce, compute lock_ref_root.
14. Commit PublicValues.
```

No burn state hash or nullifier appears in the public values.

---

## 6. Source vault execution

Storage:

```solidity
mapping(uint256 => bytes32) public lockDigest;
mapping(address => mapping(uint32 => uint64)) public nextReturnNonce;
mapping(bytes32 => bool) public trustBaseAllowed;
```

There is no `nullifierUsed` mapping.

Fulfilment sketch:

```solidity
function fulfillBatch(
    bytes calldata publicValues,
    bytes calldata proof,
    ReturnLeaf[] calldata leaves,
    SourceLockRef[] calldata lockRefs,
    bytes[] calldata accountSignatures
) external {
    sp1.verifyProof(VKEY, publicValues, proof);

    DecodedPublic memory p = decode(publicValues);
    require(p.domainTag == DOMAIN_TAG, "domain");
    require(trustBaseAllowed[p.trustBaseHash], "trust base");
    require(p.sourceChainId == SOURCE_CHAIN_ID, "chain");
    require(p.vault == address(this), "vault");
    require(p.assetContract == address(ASSET), "asset");
    require(p.tokenType == TOKEN_TYPE && p.coinId == COIN_ID, "type");
    require(hashLeaves(leaves) == p.returnRoot, "return root");
    require(hashLockRefs(lockRefs) == p.lockRefRoot, "lock root");
    require(leaves.length == p.batchSize, "batch size");
    require(accountSignatures.length == leaves.length, "signatures");

    for (uint256 i = 0; i < lockRefs.length; i++) {
        require(lockDigest[lockRefs[i].nonce] == lockRefs[i].digest, "lock");
    }

    // Leaves are sorted by (account, lane, nonce). This lets the contract load
    // and store each account/lane counter once per contiguous run.
    uint256 total;
    for (uint256 i = 0; i < leaves.length; i++) {
        total += leaves[i].amount;
    }
    require(total == p.totalAmount, "total");

    for (uint256 i = 0; i < leaves.length; i++) {
        ReturnLeaf calldata leaf = leaves[i];
        require(block.timestamp <= leaf.deadline, "expired");
        require(verifyReturnSignature(p, leaf, accountSignatures[i]), "signature");
        require(leaf.nonce == nextReturnNonce[leaf.account][leaf.lane], "nonce");

        nextReturnNonce[leaf.account][leaf.lane] = leaf.nonce + 1;
        uint256 net = leaf.amount - leaf.feeAmount;
        safeTransfer(ASSET, leaf.recipient, net);
        if (leaf.feeAmount != 0) {
            safeTransfer(ASSET, leaf.feeRecipient, leaf.feeAmount);
        }
    }
}
```

Implementation note: for gas/energy efficiency, group adjacent leaves by
`account,lane`, validate sequential nonces in memory, then write
`nextReturnNonce` once at the end of each group. The sketch writes on every leaf
only to show the invariant.

The preferred mode is **submit-and-execute**: the proof and all leaves are in one
source-chain transaction, so the vault does not store batch roots. If a batch is
too large for a Tron transaction, split it into smaller proof batches. A
two-step "post root, claim later" mode is possible, but it adds batch-root state
and is not the storage-optimal path.

---

## 7. Batching

Iteration 1 is a batch of one. Iteration 2 uses the same public values with
`batch_size > 1`.

The prover closes a batch by:

- source-chain execution limit, not only proof efficiency;
- max proving time;
- max user latency;
- nonce ordering constraints for accounts already in the batch.

Recommended batching rule:

```text
close when:
  leaf_count == B_max
  OR estimated_source_energy >= E_max
  OR oldest_leaf_age >= T_max
```

Leaves from the same `account,lane` must be submitted in nonce order. Different
accounts and lanes are independent.

---

## 8. Why this handles splits

A source lock cannot simply be marked "withdrawn" when a returned token is burned,
because a user may split a 1000 USDT bridged token into 400 and 600 USDT children
and return only one child.

This design does not consume original locks on return. Original locks only prove
that bridged supply was backed at mint time. Return replay prevention is handled
by the source account nonce. Value correctness is handled in the circuit:

- bridge genesis proves the original token was backed by a source lock record;
- split justifications recursively prove children sum to the burned parent;
- the returned token's certified payment amount must equal the return amount.

So a 400 USDT split child can be returned without touching a per-lock remaining
balance, and replaying that 400 USDT proof cannot drain the remaining 600 USDT
because the source account nonce has already advanced.

---

## 9. Prover component

The prover service is a performance component, not a trust anchor.

Responsibilities:

1. Intake burned token CBOR, return authorization signature, and optional source
   lock witnesses supplied by the wallet.
2. Fetch missing source lock records from the source chain if the wallet only
   supplied lock nonces/digests.
3. Run a host precheck identical to the guest statement to avoid wasting proving
   time.
4. Order requests by source account/lane/nonce and build bounded batches.
5. Produce SP1 Groth16 proofs.
6. Submit batches or return proof artifacts to another relayer.

Durable state is useful for job scheduling, but safety does not depend on it.
Any other prover can reproduce a proof from the burned token, source lock record,
trust base, and account signature.

---

## 10. Security properties

| Property | Mechanism | Residual risk |
|---|---|---|
| Burn is real and final | In-circuit Unicity token verification against `trustBaseHash` | Proof-system soundness and allowed trust-base governance |
| Token is backed by a source lock | In-circuit bridge mint verifier plus source-vault `lockDigest` check | None, assuming the source vault stored correct lock digests at lock time |
| Partial returns are valid | Recursive split verifier and payment amount equality | None for split-only tokens |
| Recipient/amount cannot be changed | Burn reason, public leaf, and account signature cover the same fields | None |
| Same proof cannot release twice | Source account/lane nonce increments on fulfilment | Account must manage nonce order |
| Prover cannot steal | Vault pays the recipient/fee recipient from the signed, burned authorization | Prover can stall |
| Private transfers stay private | Token history is private witness data | Prover sees the token history it proves |
| Source-chain storage does not grow per burn | No nullifier set; only account/lane nonce counters and lock digests | One counter slot per active account/lane |

---

## 11. Liveness and UX tradeoffs

The cost of removing per-burn nullifiers is ordered returns per source
`account,lane`.

- A wallet should reserve and burn nonces sequentially.
- A stuck proof for nonce `n` blocks nonce `n+1` on that lane.
- Anyone with the burned token blob can prove nonce `n`; this keeps prover
  liveness replaceable.
- If a burned token blob is permanently lost, the account may need an explicit
  `skipReturnNonce(account,lane,nonce,signature)` escape hatch. Skipping makes
  that burned token unreleasable if it is found later, so wallets should expose
  it as a recovery operation, not normal flow.
- High-volume actors can use multiple lanes to avoid head-of-line blocking.

This is the same operational shape as account nonces in ordinary source-chain
transactions, but the nonce is scoped to bridge returns.

---

## 12. Contract and plugin changes

Source vault:

- store `lockDigest[lockNonce]` at bridge-in;
- add `nextReturnNonce[account][lane]`;
- add `fulfillBatch` with SP1 Groth16 verification;
- verify return authorization signatures;
- remove `nullifierUsed` from the zk path.

Wallet plugin:

- expose source-chain `nextReturnNonce(account,lane)`;
- build and sign `ReturnAuthorization`;
- burn to `BridgeBackReason`;
- submit the burned token to one or more prover endpoints;
- track fulfilment by `{account,lane,nonce}`, not by burn nullifier.

Verifier plugin for normal token receives:

- unchanged for bridge-in validation. It may continue to validate the mint reason
  over Tron RPC.
- return proofs do not use RPC. The source vault checks its own `lockDigest`
  storage.

---

## 13. Rejected alternatives

### Per-lock remaining balances

This fails for splits. If a 1000 USDT source lock backs a 400 USDT split child,
replaying the 400 USDT burn twice would still fit under the original 1000 USDT
remaining balance. The contract would not know it saw the same burned child.

### Source claim ranges

Assigning each token a source-lock range can prevent the replay, but the source
contract must then store spent ranges or a spent interval tree. That is a
nullifier set with more complicated splitting rules and worse contract logic.

### Source-chain pre-requests

A request-first flow also removes burn nullifiers:

```text
openReturnRequest(...) -> requestId
burn reason includes requestId
fulfill deletes request
```

This is safe and useful as a fallback for wallets that cannot sign typed source
authorizations. It is not the optimal default because it adds a source-chain
transaction and temporary request storage before every burn.

### Processed Unicity checkpoint cursor

The source vault could store a single processed Unicity checkpoint and only
accept proofs for the next interval. That would avoid per-account nonces, but it
requires the proof to show it included every bridge-out burn in the interval.
The current token format gives self-contained token histories, not a cheap
bridge-specific global burn index. This would couple bridge liveness to a global
scanner and make private token activity harder to keep out of public inputs.

---

## 14. Open implementation questions

- Exact typed-data format on Tron. TVM supports EVM-style `ecrecover`, but the
  domain separator and address encoding should be frozen in tests.
- Whether to make `lane` 16, 32, or 64 bits. It is public and cheap; 32 bits is
  ample.
- Whether `skipReturnNonce` belongs in v1 or should wait until there is real
  recovery UX.
- Whether the source vault stores only `lockDigest` or also full lock fields for
  easier contract-side introspection. Digest-only is cheaper; event plus RPC is
  enough for off-chain display.
- Exact public-value ABI. Prefer fixed-width ABI encoding so the source contract
  can recompute roots cheaply.
- Batch size limits on Tron energy once Groth16 verification, signature checks,
  nonce writes, and TRC20 transfers are measured.
