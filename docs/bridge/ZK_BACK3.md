# Returning bridged assets to an EVM chain with a SNARK

**Status:** layered design for the trustless return path, Unicity -> EVM source
chain.

This document specifies the return leg of a bridged asset: a holder burns a
Unicity token and releases the matching locked asset from the source-chain vault.
The source chain accepts a succinct validity proof, not a committee signature,
operator attestation, challenge window, RPC call, or source-chain light client.

The optimization target is source-chain cost, with security taking priority over
all cost reductions. The design therefore keeps the source-chain replay state to
one accumulator root and pushes the expensive verification work into the zkVM.

The document is intentionally layered. Each layer has a small contract:

| Layer | Responsibility | Public effect |
|---|---|---|
| Source vault | Holds locked assets, lock digests, trust-base allow-list, and one spent-root | Verifies Groth16, checks local source-chain state, transfers assets |
| Unicity anchor | Proves all referenced Unicity facts are under one certified root `R*` | One BFT certificate verification per batch |
| Token lineage | Proves the burned token's transition chain and owner authorizations | Prevents forged, stolen, or structurally broken tokens |
| Value and backing | Proves minted value came from a stored source-chain lock and splits conserved value | Prevents over-release |
| Replay accumulator | Proves each burn nullifier was absent and inserts it | Prevents double release with one on-chain root |
| Batch executor | Binds public leaves to the vault transfers | Keeps source-chain calldata and execution deterministic |

---

## 1. Design Summary

### 1.1 What changes from the account-nonce design

[`ZK_BACK2.md`](./ZK_BACK2.md) removed per-burn nullifier storage by using
source-chain account nonces. That is simple for the vault, but it adds
source-chain signatures, per-account/lane state, and nonce-ordering UX.

This design uses a global nullifier accumulator instead:

| Topic | Account nonces (`ZK_BACK2`) | Accumulator (`ZK_BACK3`) |
|---|---|---|
| Source replay state | `nextReturnNonce[account][lane]` | one `spentRoot` |
| Extra source authorization | EVM/Tron account signature | none; burn reason is authorization |
| User ordering | ordered per account/lane | no per-user ordering |
| Batch ordering | independent except account nonces | serialized by global `spentRoot` |
| Public per-burn replay data | account/lane/nonce | nullifier |
| Off-chain complexity | low | accumulator witness maintenance |
| Source-chain storage growth | one slot per active account/lane | one slot total for replay |

The accumulator is the cheaper source-chain end state. It is not simpler
operationally, so the rest of this document makes that complexity explicit and
modular.

### 1.2 Flow

```text
HOLDER                 WITNESS / PROVER PIPELINE                  EVM VAULT
------                 --------------------------                  ---------
1. burn token to       2. fetch one recent certified root R*
   BridgeBackReason       and anchored inclusion proofs
   {recipient, amount, 3. build accumulator non-membership
    fee, deadline,        witnesses against current spentRoot
    asset/vault}       4. prove anchor + lineage + value
                          + backing + replay transition
                       5. submit(publicValues, proof,
                          returnLeaves, lockRefs) -------------> 6. verify Groth16
                                                                  7. check config,
                                                                     lock refs,
                                                                     spentRoot
                                                                  8. spentRoot := new
                                                                  9. transfer asset
```

The holder only needs to burn the token and hand the burned token blob to any
prover. The prover has no authority. It can stall, but it cannot redirect funds
or fabricate a release.

### 1.3 On-chain cost model

The design gives:

- O(1) source-chain replay storage: one `bytes32 spentRoot`.
- O(1) source-chain proof verification per batch: one Groth16 verification.
- O(uniqueLocks) source-chain lock checks: one `SLOAD` per unique source lock
  digest referenced by the batch.
- O(B) source-chain settlement work: calldata hashing, deadline/fee checks, and
  ERC-20 transfers for `B` public leaves.

The accurate summary is therefore: O(1) replay storage and proof verification,
plus O(B + uniqueLocks) settlement work. Transfers remain irreducible unless the
proof emits aggregated settlement leaves; see Section 11.3.

---

## 2. Core Concepts

### 2.1 Unicity as a write-once authenticated store

This design relies on a protocol property that must be true for the target
Unicity network:

1. A token transition is certified under a unique `StateId`.
2. A `StateId` can be written at most once.
3. Once written, it remains provable in later certified roots.
4. The network can return inclusion proofs for historical transition records
   against a chosen later root `R*`.

With those properties, one recent certified root `R*` can anchor every state
record in a token's history. The circuit verifies the BFT certificate for `R*`
once, then verifies many cheap anchored inclusion paths against that root.

If a future Unicity deployment does not provide historical inclusion proofs
against a later root, this optimization is invalid and the design must fall back
to per-transition certificates or another authenticated-history commitment.

### 2.2 State id, transition id, and nullifier

The accumulator must use an unambiguous replay key.

For implementation, avoid the ambiguous phrase "burn state key." In the Rust SDK,
a transfer is certified under the `StateId` of the state it spends:

```text
StateId = H(lock_script, source_state_hash)
```

The burn transfer is the terminal transfer whose recipient predicate is
`BurnPredicate(BridgeBackReason)`. Its certified transition is uniquely
identified by:

```text
burnTransitionId = H(
  "unicity-burn-transition:v1",
  certifiedBurnStateId,
  certifiedBurnTransactionHash
)
```

The return nullifier is:

```text
nullifier = H("unicity-bridge-return-nullifier:v1", configHash, burnTransitionId)
```

`configHash` binds the nullifier to one vault/asset configuration, so the same
Unicity burn cannot accidentally collide across independent bridge deployments.

The nullifier is public. It is emitted in the return leaf so anyone can rebuild
the off-chain accumulator and replace a stalled prover. It does not expose the
private token history unless an observer already has the burned token blob.

### 2.3 Config hash

The vault pins a single immutable configuration hash instead of rechecking many
asset fields independently:

```text
configHash = keccak256(abi.encode(
  "unicity-bridge-return-config:v1",
  sourceChainId,
  vault,
  asset,
  tokenType,
  coinId,
  bridgeBackReasonTag,
  lockDigestDomain,
  nullifierDomain
))
```

The circuit receives the full config as witness or constant, checks every burn
reason and lock digest against it, and commits only `configHash` publicly. The
vault compares `publicValues.configHash` with its immutable `CONFIG_HASH`.

---

## 3. Source-Chain Bridge-In Record

Bridge-in remains lock on source chain -> mint on Unicity. The source vault must
retain a compact lock digest so bridge-out can prove backing without an RPC call.

```solidity
struct LockRecord {
    address asset;               // ERC-20/TRC-20 locked
    bytes32 tokenType;           // Unicity bridged-asset token type
    bytes32 coinId;              // value id carried in token data
    uint256 amount;              // smallest source-asset unit
    bytes32 unicityTokenId;      // token this lock funds
    bytes32 recipientCommitment; // H(minted recipient predicate CBOR)
}

mapping(uint256 => bytes32) public lockDigest; // lock nonce => digest
uint256 public nextLockNonce;
```

Digest:

```text
lockDigest[nonce] = keccak256(abi.encode(
  "unicity-bridge-lock:v1",
  sourceChainId,
  vault,
  nonce,
  asset,
  tokenType,
  coinId,
  amount,
  unicityTokenId,
  recipientCommitment
))
```

The lock event should still emit the full fields for wallets and explorers. The
contract only needs the digest. During return, the circuit recomputes the digest
from a `LockRecord` witness, and the vault checks that the public lock ref equals
its stored `lockDigest[nonce]`.

This is per-deposit source-chain storage, but it is created at bridge-in and is
also useful for auditability. The return path adds only the replay accumulator
root.

Possible later optimization: replace `mapping(lockNonce => digest)` with an
append-only `lockRoot` maintained at lock time. That reduces return-time `SLOAD`s
for highly split assets, but it increases bridge-in complexity and should not be
the first implementation.

---

## 4. Burn Reason

The holder returns value by burning a Unicity token to a `BurnPredicate` whose
reason commits to the source-chain release.

```text
BridgeBackReason = #tag(BRIDGE_BACK_REASON_TAG) [
  version,            // 1
  sourceChainId,
  vault,              // 20-byte EVM/TVM address
  asset,              // 20-byte token contract address
  tokenType,          // 32 bytes
  coinId,             // 32 bytes
  recipient,          // receives amount - fee
  amount,             // gross amount; equals burned token value for coinId
  feeRecipient,       // zero address if no fee
  feeAmount,          // <= amount
  deadline            // optional; see below
]
```

There is no separate source-chain signature. The act of burning is the
authorization, and the burn reason fixes the destination.

For a partial return, the wallet splits first and burns the child whose value is
the desired return amount.

### Deadline guidance

`deadline` is useful for relayer fee quotes, but it is dangerous because a burn
is irreversible. If the deadline expires before a proof lands, the source vault
must reject the release and the burned value remains locked on the source chain.

For v1, wallets should default to "effectively no deadline" or a very long
deadline. Short deadlines should be treated as an advanced relayer-order feature,
not a default return flow.

---

## 5. Public Data Structures

### 5.1 Public values

These are committed by the circuit and decoded by the vault.

```rust
pub struct PublicValues {
    pub domain_tag:      [u8; 32], // keccak256("unicity-bridge-return:v1")
    pub config_hash:     [u8; 32], // immutable vault config
    pub trust_base_hash: [u8; 32], // allow-listed on-chain
    pub spent_root_old:  [u8; 32], // must equal vault.spentRoot
    pub spent_root_new:  [u8; 32], // vault.spentRoot becomes this
    pub return_root:     [u8; 32], // commits to ReturnLeaf[]
    pub lock_ref_root:   [u8; 32], // commits to SourceLockRef[]
    pub batch_size:      u32,
    pub total_amount:    U256,
}
```

### 5.2 Return leaves

The vault receives decoded leaves because it must execute transfers and emit
public events.

```rust
pub struct ReturnLeaf {
    pub nullifier:     [u8; 32],
    pub recipient:     [u8; 20],
    pub amount:        U256,
    pub fee_recipient: [u8; 20],
    pub fee_amount:    U256,
    pub deadline:      u64,
}
```

The nullifier is intentionally public. Without it, the off-chain accumulator is
not reconstructable by replacement provers.

`return_root` is the keccak hash of fixed-width ABI encodings of `ReturnLeaf[]`
in submission order. The source-chain contract does not need to sort return
leaves. The circuit binds each leaf to the corresponding burn reason and
nullifier.

### 5.3 Source lock references

```rust
pub struct SourceLockRef {
    pub nonce:  U256,
    pub digest: [u8; 32],
}
```

`lock_ref_root` is the keccak hash of fixed-width ABI encodings of
`SourceLockRef[]`, sorted by `nonce` and with duplicate nonces rejected. The
vault enforces the same sorted-unique rule before checking storage.

---

## 6. Private Witnesses

```rust
pub struct GuestInput {
    pub config:           BridgeConfig,
    pub trust_base:       RootTrustBase,
    pub anchor_root:      [u8; 32],
    pub anchor_cert:      UnicityCertificate,
    pub spent_root_old:   [u8; 32],
    pub burns:            Vec<BurnInput>,
}

pub struct BurnInput {
    pub token_cbor:       Vec<u8>,
    pub inclusions:       Vec<AnchoredInclusionProof>,
    pub source_locks:     Vec<LockRecord>, // v1 expects exactly one
    pub nullifier_aux:    NonMembershipWitness,
}
```

`AnchoredInclusionProof` is a compact inclusion proof against `anchor_root`. It
must not repeat the BFT certificate for every transition.

```rust
pub struct AnchoredInclusionProof {
    pub state_id:           [u8; 32],
    pub transaction_hash:   [u8; 32],
    pub certification_data: CertificationData,
    pub smt_path:           InclusionCertificate,
}
```

The witness builder obtains `anchor_root`, `anchor_cert`, and the anchored
inclusion proofs by querying Unicity for one chosen root `R*`.

---

## 7. Layer Contracts

### 7.1 Unicity anchor layer

Inputs:

- `trust_base`
- `trust_base_hash`
- `anchor_root`
- `anchor_cert`
- all `AnchoredInclusionProof`s

Checks:

1. `H(trust_base) == trust_base_hash`.
2. `anchor_cert` seals `anchor_root` under `trust_base`.
3. Every anchored inclusion proof has a valid SMT path to `anchor_root`.
4. Every included record's `CertificationData` matches the transition it is used
   for.

Output:

- authenticated transition records under the single anchor root.

Security note: this is the only BFT quorum verification in the batch.

### 7.2 Token lineage layer

Inputs:

- decoded token
- authenticated transition records from the anchor layer

Checks:

1. Genesis and transfer chain linkage matches the SDK's reconstructed fields.
2. Each transfer spends the prior token state.
3. Each transition hash equals the authenticated transaction hash.
4. Each owner authorization is valid for the authenticated spend.
5. The terminal transfer's recipient is `BurnPredicate(BridgeBackReason)`.

Output:

- `burnTransitionId`
- decoded `BridgeBackReason`
- authenticated token value facts needed by the value layer.

This layer should track the Rust SDK verification invariants directly. The
implementation should avoid inventing a second verification model.

### 7.3 Value and backing layer

Inputs:

- bridge mint reason from genesis
- source lock witnesses
- token payment data
- split justifications along the path to the burned leaf

Checks:

1. Bridge genesis references a source lock nonce.
2. Recomputed `lockDigest` is present in the batch's source lock refs.
3. `LockRecord` fields match `config`.
4. `LockRecord.unicityTokenId == genesis.tokenId`.
5. `LockRecord.recipientCommitment == H(genesis.recipient.toCBOR())`.
6. Genesis payment amount for `coinId` equals `LockRecord.amount`.
7. Each split on the path conserves value and the burned child carries the
   claimed amount.
8. Burn reason amount equals the certified value for `coinId`.
9. `feeAmount <= amount`.

V1 should reject `source_locks.len() != 1` until Unicity has an explicit merge
or combine primitive whose verifier is included in this layer. The vector shape
is reserved for that future path; it should not silently broaden v1.

Output:

- gross amount
- recipient and fee fields
- source lock refs used by this burn.

### 7.4 Replay accumulator layer

Inputs:

- `spent_root_old`
- per-burn `nullifier`
- per-burn `NonMembershipWitness`

Checks:

1. Start with `running_spent = spent_root_old`.
2. For each burn in return-leaf order:
   - verify nullifier non-membership under `running_spent`;
   - insert the nullifier;
   - set `running_spent` to the insertion result.
3. Reject duplicate nullifiers in the same batch. The second insertion should
   fail non-membership, but an explicit duplicate check gives clearer errors.

Output:

- `spent_root_new`.

The accumulator is off-chain state, not trusted state. It is reconstructable from
the empty accumulator root and the ordered stream of previously emitted public
nullifiers. A rebuilt tree must reproduce the current on-chain `spentRoot` before
it is used for new witnesses.

### 7.5 Batch executor layer

Inputs:

- per-burn outputs from the prior layers
- source lock refs

Checks:

1. Construct one `ReturnLeaf` per burn in the exact order used for accumulator
   insertion.
2. Compute `return_root`.
3. Deduplicate and sort source lock refs by nonce, then compute `lock_ref_root`.
4. Compute `total_amount`.

Output:

- `PublicValues`
- `ReturnLeaf[]`
- `SourceLockRef[]`.

---

## 8. Circuit Algorithm

Once per batch:

```text
A0. require H(config) == config_hash
A1. require H(trust_base) == trust_base_hash
A2. verify anchor_cert seals anchor_root under trust_base
A3. running_spent := spent_root_old
```

For each burn, in return-leaf order:

```text
F1. token = Token::from_cbor(token_cbor)
F2. verify every required AnchoredInclusionProof against anchor_root
F3. verify SDK-equivalent chain linkage and owner authorization

V1. verify bridge genesis against source LockRecord digest
V2. verify split/value lineage to the burned token

D1. decode BridgeBackReason from the terminal BurnPredicate
D2. require reason config fields match config
D3. require reason.amount == certified value for coinId
D4. require reason.feeAmount <= reason.amount

R1. burnTransitionId = H(certifiedBurnStateId, certifiedBurnTransactionHash)
R2. nullifier = H(nullifierDomain, configHash, burnTransitionId)
R3. verify nullifier non-membership under running_spent
R4. running_spent := insert(running_spent, nullifier)

E1. append ReturnLeaf{nullifier, recipient, amount, feeRecipient, feeAmount, deadline}
E2. record SourceLockRef{nonce, digest}
```

After the batch:

```text
C1. spent_root_new := running_spent
C2. return_root := keccak(ReturnLeaf[] in append order)
C3. lock_ref_root := keccak(sorted unique SourceLockRef[])
C4. total_amount := sum(ReturnLeaf.amount)
C5. commit PublicValues
```

---

## 9. Vault Contract

Sketch:

```solidity
contract ReturnVault {
    ISP1Verifier public immutable sp1;
    bytes32 public immutable VKEY;
    bytes32 public immutable DOMAIN_TAG;
    bytes32 public immutable CONFIG_HASH;
    IERC20  public immutable ASSET;

    mapping(uint256 => bytes32) public lockDigest;       // set at bridge-in
    mapping(bytes32 => bool)    public trustBaseAllowed; // timelocked governance
    bytes32 public spentRoot;                            // init = EMPTY_TREE_ROOT

    function fulfillBatch(
        bytes calldata publicValues,
        bytes calldata proof,
        ReturnLeaf[] calldata leaves,
        SourceLockRef[] calldata lockRefs
    ) external nonReentrant {
        sp1.verifyProof(VKEY, publicValues, proof);
        P memory p = decode(publicValues);

        require(p.domainTag == DOMAIN_TAG,                 "domain");
        require(p.configHash == CONFIG_HASH,               "config");
        require(trustBaseAllowed[p.trustBaseHash],         "trust base");
        require(p.spentRootOld == spentRoot,               "stale root");
        require(leaves.length == p.batchSize,              "size");
        require(keccakLeaves(leaves) == p.returnRoot,      "leaves");
        require(keccakLockRefs(lockRefs) == p.lockRefRoot, "lock refs");
        require(_lockRefsSortedUnique(lockRefs),           "lock order");

        for (uint256 i = 0; i < lockRefs.length; ++i) {
            require(lockDigest[lockRefs[i].nonce] == lockRefs[i].digest, "lock");
        }

        uint256 total;
        for (uint256 i = 0; i < leaves.length; ++i) {
            ReturnLeaf calldata L = leaves[i];
            require(block.timestamp <= L.deadline, "expired");
            require(L.feeAmount <= L.amount,       "fee");
            total += L.amount;
        }
        require(total == p.totalAmount, "total");

        spentRoot = p.spentRootNew;
        emit BatchFulfilled(p.spentRootOld, p.spentRootNew, p.returnRoot, leaves.length);

        for (uint256 i = 0; i < leaves.length; ++i) {
            ReturnLeaf calldata L = leaves[i];
            ASSET.safeTransfer(L.recipient, L.amount - L.feeAmount);
            if (L.feeAmount != 0) {
                ASSET.safeTransfer(L.feeRecipient, L.feeAmount);
            }
            emit Released(L.nullifier, L.recipient, L.amount, L.feeRecipient, L.feeAmount);
        }
    }
}
```

Contract rules:

- The vault never verifies Unicity Merkle branches or BFT signatures.
- The vault never stores a nullifier set or nonce map.
- The vault must emit every public nullifier in `Released`, in the same order as
  the return leaves.
- The vault should emit `BatchFulfilled(oldRoot, newRoot, returnRoot, batchSize)`
  so accumulator rebuilders can checkpoint root transitions.
- `spentRoot` is updated once per batch.
- All validation happens before external transfers.
- The known asset should still be transferred through a safe-transfer helper and
  a reentrancy guard.

---

## 10. Off-Chain Components

The prover is trustless, but the engineering should not be a single monolith.

### 10.1 Witness builder

Responsibilities:

- decode burned token blobs;
- choose an anchor root `R*`;
- fetch compact anchored inclusion proofs for every required transition;
- fetch or reconstruct source lock records from source-chain events;
- run a host-side precheck matching the circuit.

### 10.2 Accumulator builder

Responsibilities:

- reconstruct the indexed Merkle tree from `EMPTY_TREE_ROOT` by replaying
  successful `BatchFulfilled` and `Released(nullifier, ...)` events in chain
  order;
- produce non-membership witnesses;
- update a local cache after successful batches;
- rebase pending batches when another batch advances `spentRoot`.

This service is replaceable. Its local tree is a cache over public data, not a
trusted database.

### 10.3 Prover

Responsibilities:

- receive witness packages;
- produce SP1 proofs and wrap to Groth16;
- expose proof artifacts to relayers.

### 10.4 Sequencer / relayer

Responsibilities:

- select burns for a batch;
- serialize batches on the current `spentRoot`;
- submit `fulfillBatch`;
- retry or rebase stale-root batches.

A single sequencer improves liveness, but it is not trusted. If it stalls,
anyone can rebuild the accumulator from public events and continue.

---

## 11. Efficiency Improvements

### 11.1 Immediate v1 choices

- Use `configHash` to reduce public-value decoding and config comparisons.
- Publish nullifiers in return leaves to preserve accumulator reconstructability.
- Use one anchor root per batch and compact anchored inclusion proofs.
- Deduplicate and sort lock refs so the vault performs one `SLOAD` per unique
  source lock.
- Keep submit-and-execute settlement so the vault stores no batch roots.

### 11.2 Batching

Stage 1: `batch_size = 1`.

Stage 2: one proof for `B` burns sharing one anchor root and one accumulator
transition. This amortizes:

- one on-chain Groth16 verification;
- one STARK-to-Groth16 wrap;
- one BFT quorum verification of `R*`;
- one `spentRoot` write.

On-chain transfer cost remains linear in the number of settlement leaves.

### 11.3 Optional settlement aggregation

The accumulator must insert one nullifier per burned token, but the vault does
not necessarily need one ERC-20 transfer per burn.

Later, the circuit can emit:

```rust
pub struct SettlementLeaf {
    pub recipient: [u8; 20],
    pub amount: U256,
}
```

and prove that `SettlementLeaf[]` is the aggregation of `ReturnLeaf[]` by
recipient after subtracting fees. The vault would transfer settlement leaves
instead of return leaves, while still emitting the underlying public nullifiers.

This reduces source-chain transfers when many burns settle to the same recipient
or relayer, but it adds another in-circuit aggregation and another public root.
Keep v1 one transfer per return unless measured source-chain transfer cost
dominates.

### 11.4 Recursion and parallelism

For high throughput, prove each token lineage independently and aggregate child
proofs in a batch circuit. The batch circuit verifies child proofs, inserts their
public nullifiers into the accumulator, binds all returns to one `configHash` and
trust-base policy, and emits the same `PublicValues`.

This changes proving architecture only. The vault interface stays stable.

---

## 12. Security Analysis

| Property | Mechanism | Residual risk |
|---|---|---|
| Burn is real and final | One certified anchor root plus inclusion of the burn transition | Proof-system soundness; trust-base governance |
| Token history is valid | SDK-equivalent chain linkage and owner-authorization checks | Circuit must track SDK semantics exactly |
| Value is genuine | Bridge genesis backed by vault-stored lock digest | Correct lock digest at bridge-in |
| Splits cannot inflate value | Recursive split sum checks on the path to the burned leaf | None for supported split semantics |
| Destination cannot be altered | Burn reason binds config, recipient, amount, fee, deadline | None |
| Same burn cannot release twice | Public nullifier non-membership and insertion, checked by `spentRootOld == spentRoot` | Accumulator implementation correctness |
| Source-chain replay storage is constant | One `spentRoot`, no nullifier map and no nonce map | Off-chain accumulator grows publicly |
| Prover cannot steal | Vault pays only public leaves proven from burn reasons | Prover can stall |
| Prover is replaceable | Public nullifiers and lock refs let anyone rebuild witnesses | Requires reliable access to burned token blobs and Unicity proofs |
| Transfer history privacy | Token lineage stays private witness data | Prover sees histories it proves |

Security-critical implementation requirements:

1. Public nullifiers must be emitted, or the accumulator is not independently
   reconstructable.
2. `burnTransitionId` must be derived from authenticated SDK fields, not from an
   informal "burn state" label.
3. The circuit must bind the nullifier in `ReturnLeaf` to the same nullifier
   inserted into the accumulator.
4. The vault must enforce `spentRootOld == spentRoot` before accepting the root
   transition.
5. The vault must compare `configHash` and `trustBaseHash` against governed
   source-chain state.

---

## 13. Liveness and Operations

- There is no per-user ordering. Users do not manage return nonces.
- There is global batch sequencing. Batches serialize on `spentRoot`.
- A stale batch is safe but fails on-chain with `"stale root"`; it must be
  rebuilt against the new accumulator root.
- No third party can insert arbitrary nullifiers. Only a valid proof can advance
  `spentRoot`.
- If a prover disappears, another prover rebuilds the accumulator from public
  `Released` events from the empty accumulator root, refetches Unicity anchored
  proofs, and proves again.
- If a burned token blob is lost before release, the source-chain vault cannot
  release it. Wallets and relayers must treat burned token blobs as critical
  recovery material until settlement.

---

## 14. Parameters To Freeze

Normative before implementation:

- `BridgeBackReason` CBOR tag and field order.
- `configHash` domain and full preimage.
- `lockDigest` domain and full preimage.
- `burnTransitionId` derivation from SDK fields.
- `nullifier` domain and full preimage.
- accumulator type, depth, empty root, leaf format, and hash.
- fixed-width ABI encoding for `PublicValues`, `ReturnLeaf`, and
  `SourceLockRef`.
- sorted-unique rule for lock refs.
- trust-base allow-list governance and timelock.

Measured before production:

- maximum batch size under Tron/EVM block limits;
- Groth16 verifier cost;
- cost of `B` ERC-20 transfers;
- cost of lock-ref storage reads;
- SP1 proving time for typical lineage lengths;
- anchor-proof fetch latency from Unicity.

---

## 15. Glossary

`StateId`
: SDK key for a certified state spend, derived from lock script and source state
  hash.

`transition hash`
: Hash of the certified mint or transfer transaction.

`burn transition`
: Terminal transfer whose recipient is `BurnPredicate(BridgeBackReason)`.

`burnTransitionId`
: Domain-separated hash of the certified burn `StateId` and transition hash.

`nullifier`
: Public replay key derived from `configHash` and `burnTransitionId`.

`spentRoot`
: Source-chain root of the off-chain indexed Merkle tree containing released
  nullifiers.

`anchor root`
: A recent Unicity authenticated-store root sealed by a BFT certificate.

`anchored inclusion proof`
: Compact proof that a transition record is present under the anchor root,
  without repeating the BFT certificate.

`lockDigest`
: Source-chain commitment stored at bridge-in proving that a specific Unicity
  token was backed by a specific source-chain lock.
