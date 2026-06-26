# 00 — Interop contract (normative)

This is the single source of truth for every byte that crosses a component
boundary. The contracts, the TS SDK, and the prover each implement a *subset* of
it, but no component may define an encoding, domain separator, or hash that is
not fixed here. It refines [`../ZK_BACK3.md`](../ZK_BACK3.md) §2, §5, §14 and the
yellowpaper `appendix-bridging.tex` "Shared Parameters" table into a frozen,
testable form.

**Version:** `BRIDGE_PROTO_VERSION = 1` — decisions below are fixed; vectors are
published at M0. Every repo pins this constant; the conformance vectors are tagged
with it.

---

## 1. Hash policy (FIXED)

Two hash functions are in play; the split below is chosen for **on-chain
efficiency** and is normative. Each value is assigned one function for its whole
life.

| Function | Used for | Why this is the efficient choice |
|---|---|---|
| **keccak256 over `abi.encode`** (`K`) | Everything the **vault recomputes on-chain**: `configHash`, `lockDigest`, `returnRoot`, `lockRefRoot`, `domainTag`, and the `PublicValues` ABI layout. | `KECCAK256` is a native opcode (≈30+6/word gas) — strictly cheaper on-chain than the SHA-256 precompile `0x02` (≈60+12/word), and it is native on TVM too. SP1 has an accelerated keccak syscall, so the in-circuit cost is comparable to SHA-256; the on-chain saving is the deciding factor. |
| **SHA-256 over deterministic CBOR** (`H`) | Everything that must match the **Unicity SDK / aggregator**: `StateId`, transaction hashes, `trustBaseHash`, the nullifier and its `burnTransitionId`, and the replay accumulator (SMT). | Forced: these already exist as SHA-256/CBOR in the protocol and both SDKs and must match the Service bit-for-bit. The vault never recomputes them — it stores the accumulator *root* opaquely and emits leaves — so they cost nothing on-chain. Reusing the SDK's existing SHA-256 radix SMT for the accumulator also avoids a second tree implementation. |

This is exactly ZK_BACK3's split (`CONFIG_HASH`, `lockDigest`, `return_root`,
`lock_ref_root` are `keccak256(abi.encode(...))`; `nullifier`/accumulator are
SHA-256). The yellowpaper `appendix-bridging.tex` currently writes all hashes as
`H`; **it must be updated** to use keccak for the five vault-recomputed values
(its existing "EVM deployments MAY use keccak-256" note becomes normative).

Implementation consequence: the **circuit links keccak256** (e.g. `tiny-keccak`)
alongside RustCrypto `sha2`; only the five vault-recomputed values use `K`, and
`configHash` (keccak) is then fed into the nullifier (SHA-256), so the guest links
both. TS has both via `@noble/hashes`; Solidity/TVM have `keccak256` natively and
SHA-256 at precompile `0x02`.

---

## 2. Configuration and `configHash`

A deployment is the tuple (ZK_BACK3 §2.3):

```
config = (
  sourceChainId : uint64,
  vault         : address(20),
  asset         : address(20),
  tokenType     : bytes32,        // Unicity bridged-asset token type
  coinId        : bytes32,        // value id in token payment data
  reasonTag     : uint64,         // CBOR tag of BridgeBackReason
  lockDomain    : bytes32,        // domain separator for lockDigest
  nullifierDomain : bytes32       // domain separator for the nullifier
)

configHash = K(abi.encode(
  "unicity-bridge-return-config:v1",
  sourceChainId, vault, asset, tokenType, coinId,
  reasonTag, lockDomain, nullifierDomain))
```

- The **vault** derives `CONFIG_HASH` *and* its payout `ASSET` from one
  `BridgeConfig` struct in its constructor, so they cannot diverge (ZK_BACK3 §2.3,
  §9). `ASSET = asset`.
- The **circuit** receives `config` as a witness/constant, recomputes
  `configHash`, checks every reason/lock field against `config`, and commits
  `configHash` publicly.
- The **TS SDK** derives the same `configHash` to label tokens and to build the
  manifest. Note `tokenType`/`coinId` are themselves derived today by
  `bridge-plugin-tron-usdt/src/identifiers.ts`:
  `tokenType = SHA256("unicity-bridge:tron:<chainId>:<assetEvmHex>")`,
  `coinId = SHA256("unicity-bridge-coin:tron:<chainId>:<assetEvmHex>")`. Those
  derivations are frozen here too (they feed `config`).

---

## 3. Bridge-in: lock event, `LockRecord`, `lockDigest`

`UnicityLock` today stores a per-field `LockRecord{from,amount,unicityTokenId,
recipientCommitment,withdrawn}` and emits
`Lock(nonce, from, amount, unicityTokenId, recipientCommitment)`. ZK_BACK3 stores
only a digest. The reconciliation (see `01-source-chain-contracts.md`):

```
LockRecord = (
  asset               : address(20),   // = config.asset (deployment constant)
  tokenType           : bytes32,       // = config.tokenType
  coinId              : bytes32,       // = config.coinId
  amount              : uint256,
  unicityTokenId      : bytes32,
  recipientCommitment : bytes32        // = SHA256(recipient predicate CBOR)
)

lockDigest[nonce] = K(abi.encode(
  "unicity-bridge-lock:v1",
  sourceChainId, vault, nonce,
  asset, tokenType, coinId, amount, unicityTokenId, recipientCommitment))
```

- **Vault** stores `lockDigest[nonce]` at lock time and the public
  `Lock` event keeps emitting the full fields for the TS verifier and explorers.
- **Circuit** reconstructs `LockRecord` from the certified genesis + `config`,
  recomputes `lockDigest`, and exports `(nonce, digest)` as a public lock ref.
- **`recipientCommitment`** is `SHA256(recipientCbor)` — already implemented in
  `bridge-plugin-tron-usdt/src/identifiers.ts::recipientCommitment`. The circuit
  and vault use the same definition.

---

## 4. Bridge-back: `BridgeBackReason` (CBOR)

Built by the **TS wallet** at burn time, decoded by the **circuit**. CBOR array
under tag `reasonTag`, field order is normative (ZK_BACK3 §4, appendix Return
Reason):

| # | Field | Type | Note |
|---|---|---|---|
| 0 | `version` | uint = 1 | |
| 1 | `sourceChainId` | uint64 | must equal `config` |
| 2 | `vault` | bytes(20) | must equal `config` |
| 3 | `asset` | bytes(20) | must equal `config` |
| 4 | `tokenType` | bytes32 | must equal `config` |
| 5 | `coinId` | bytes32 | must equal `config` |
| 6 | `recipient` | bytes(20) | external release recipient |
| 7 | `amount` | uint256 | gross; equals burned token value for `coinId` |
| 8 | `feeRecipient` | bytes(20) | zero address ⇒ no fee |
| 9 | `feeAmount` | uint256 | `≤ amount` |
| 10 | `deadline` | uint64 | gates the **fee only**, never the principal |

The burn reason is **self-contained** (ZK_BACK3 §2.2, §4). The terminal burn
transfer carries the canonical `BridgeBackReason` bytes (`reasonBytes`) **in its
auxiliary data**, and its recipient predicate is `BurnPredicate(reasonHash)` with
`reasonHash = H(reasonBytes)` — *not* `BurnPredicate(reasonBytes)` directly. So
the reason is not an out-of-band witness: the circuit reads `reasonBytes` from the
certified aux data, recomputes `reasonHash`, requires the terminal recipient
predicate to equal `BurnPredicate(reasonHash)`, and only then decodes the fields.
A burned token blob therefore contains all release-authorizing data; if it is lost
before settlement the vault has nothing self-contained to release against.

`reasonHash = H(reasonBytes)` is SHA-256 over the raw canonical reason bytes
(PROVISIONAL — confirm at M0 against the SDK's `BurnPredicate` reason convention;
note `appendix-bridging.tex` writes `H(REASON_TAG, R)`, a two-field CBOR-array
hash, which must be reconciled to the same preimage). Deterministic-CBOR rules
(canonical, complete consumption, reject trailing/non-canonical) follow the rest
of the SDK.

---

## 5. Identifiers, the nullifier, and `burnTransitionId`

Per ZK_BACK3 §2.2. `StateId` is the SDK's existing
`StateId::derive(lock_script, source_state_hash) = H(lock_script, source_state_hash)`.

```
burnTransitionId = H("unicity-burn-transition:v1",
                     certifiedBurnStateId, certifiedBurnTransactionHash)

nullifier = H("unicity-bridge-return-nullifier:v1", configHash, burnTransitionId)
```

- `certifiedBurnStateId` = `StateId` of the state the burn transfer spends.
- `certifiedBurnTransactionHash` = the certified transaction hash of the burn
  transfer (already produced by the SDK).
- **DIVERGENCE FROM TEX (FIXED):** `appendix-bridging.tex` flattens this to
  `η = H(NUL_DOMAIN, h_cfg, sid_b, txhash_b)` with no intermediate
  `burnTransitionId`. **The nested ZK_BACK3 form is normative**; the tex must
  adopt it.
- The nullifier is **public**: emitted in the release leaf and in the on-chain
  `Released` event so any party can rebuild the accumulator.
- **Shared by:** circuit (computes + inserts), off-chain accumulator builder
  (recomputes to build witnesses), vault (emits verbatim from the leaf).

---

## 6. Replay accumulator (nullifier SMT)

The accumulator is the radix sparse Merkle tree of yellowpaper Appendix RSMT,
i.e. the same family as the SDK's `InclusionCertificate` SMT, used in
**non-sum** mode (presence set, not the RSMST sum tree).

| Parameter | Value |
|---|---|
| Tree | radix sparse Merkle tree, 256-bit key space, LSB-first (matches `state-transition-sdk-rust::radix`) |
| Key | `nullifier` (`bytes32`) |
| Leaf value | fixed presence constant `0x01` |
| Hash | SHA-256, domain prefixes `0x00` (leaf) / `0x01` (node) — the plain inclusion-tree prefixes, **not** the RSMST `0x10`/`0x11` |
| Empty root `A∅` | empty-tree root (all-zero hash) — must equal the vault's `EMPTY_TREE_ROOT` constant |
| Operations | `VerifyNonMember(A,η,w)`, `Insert(A,η,w)→A'` |
| **Batch order** | witness `w_k` is valid against the root **after** inserts `0..k-1`, not against `A_old` (ZK_BACK3 §6, §7.4) |

- **Vault** stores only `A` (`bytes32 spentRoot`), initialized to `A∅`; never
  recomputes the tree.
- **Circuit** verifies non-membership and threads `Insert` in return-leaf order.
- **Accumulator builder** (off-chain) rebuilds the tree from the public
  `Released`/`BatchFulfilled` event stream and produces order-coupled witnesses.

> **SDK gap:** the Rust SDK's radix SMT today exposes inclusion proofs; it needs a
> **non-membership** witness + `Insert` returning the new root (see
> `03-prover-service.md`). The on-chain side is unaffected (root is opaque).

---

## 7. Public statement (`PublicValues`) — vault-decoded ABI

Committed by the circuit, decoded by the vault. Fixed-width `abi.encode`
(ZK_BACK3 §5.1):

```
PublicValues = (
  domainTag      : bytes32,   // K("unicity-bridge-return:v1")
  configHash     : bytes32,
  trustBaseHash  : bytes32,   // H(RootTrustBase) — see §8
  spentRootOld   : bytes32,
  spentRootNew   : bytes32,
  returnRoot     : bytes32,
  lockRefRoot    : bytes32,
  batchSize      : uint32,
  totalAmount    : uint256
)
```

Calldata committed by `returnRoot` / `lockRefRoot`:

```
ReturnLeaf    = (nullifier:bytes32, recipient:address, amount:uint256,
                 feeRecipient:address, feeAmount:uint256, deadline:uint64)
returnRoot    = K( fixed-width abi.encode of ReturnLeaf[] in submission order )

SourceLockRef = (nonce:uint256, digest:bytes32)
lockRefRoot   = K( fixed-width abi.encode of SourceLockRef[] sorted by nonce, dups rejected )
```

The vault recomputes `returnRoot`/`lockRefRoot` from calldata and checks
equality; it enforces the sorted-unique rule on lock refs independently.

> **Note on `domainTag` (FIXED):** ZK_BACK3 carries an explicit `domainTag`; the
> tex folds it into `configHash`. Normative: keep the explicit `domainTag` (matches
> the vault sketch); the tex must adopt it.

---

## 8. Trust base hash and anchor

- `trustBaseHash = H(RootTrustBase)` over the SDK's canonical encoding of the
  trust base. The exact preimage is **frozen at M0** by pinning a Rust helper
  `RootTrustBase::canonical_hash()` and mirroring it in TS; the vault only ever
  compares against an allow-listed set, so it does not compute it, but the
  circuit and any tooling must agree.
- **Anchor:** one recent `UnicityCertificate` `UC*` with root `h* = UC*.IR.hash`.
  The circuit verifies `UC*` once (the single BFT-quorum check) and every
  transition by an SMT inclusion path against `h*` (anchored mode). The vault is
  oblivious to `h*`; it trusts the allow-listed `trustBaseHash` the proof commits.
- **v1 predicate-time scope (FIXED).** Anchored mode drops each transition's
  original request-validation time (`UC.IR.t`) from the in-circuit predicate call,
  so **v1 accepts only predicate families whose validity is independent of Unicity
  system time** — signature ownership, burn, and split/value, i.e. the current
  bridge-token path. The relation MUST reject any predicate whose result depends on
  `UC.IR.t`. A later version needing timelocks/HTLCs must either carry
  authenticated original validation time per transition or fall back to
  per-transition certificates for those spends (ZK_BACK3 §2.1, §7.2).

---

## 9. Settlement semantics (must match in vault + circuit + tex)

- **Deadline gates the fee only.** `now ≤ deadline` ⇒ pay `recipient`
  `amount-feeAmount` and `feeRecipient` `feeAmount`; else pay `recipient` the full
  `amount`, no fee. The principal is **always** released (ZK_BACK3 §4, §9).
- **Replay guard** is exactly `spentRootOld == spentRoot` on-chain; success sets
  `spentRoot = spentRootNew` once per batch.
- **Batch atomicity** is a liveness property: a reverting recipient reverts the
  batch, leaving `spentRoot` unchanged and no nullifier consumed.

---

## 10. Conformance vectors (the sync mechanism)

A versioned fixture package `bridge-vectors/` (sibling to the SDKs) is the
machine-checkable form of this document. **One** designated reference
implementation generates it; **all three** components consume it as test input.
A component is "in sync" iff its CI passes the current vectors for its subset.

### Layout

```
bridge-vectors/
  VERSION                       # = BRIDGE_PROTO_VERSION
  config/      *.json           # config → configHash, derived tokenType/coinId
  lock/        *.json           # LockRecord → lockDigest; Lock event bytes
  reason/      *.json           # BridgeBackReason fields → canonical CBOR bytes
  nullifier/   *.json           # (stateId, txHash, configHash) → burnTransitionId, nullifier
  accumulator/ *.json           # ordered nullifier stream → roots; non-membership witnesses
  public/      *.json           # PublicValues struct → abi bytes; ReturnLeaf[]/lockRef[] → roots
  token/       *.cbor + *.json  # full burned-token blobs → relation outputs (M2+)
```

Each JSON pairs explicit inputs with expected outputs (hex). Generators are
deterministic and committed.

### Who generates what

| Vector group | Reference generator | Rationale |
|---|---|---|
| `config`, `lock`, `reason`, `nullifier`, `public` | **Rust** (`state-transition-sdk-rust` + a `bridge-vectors` bin) | the circuit is the strictest consumer; generate where the relation lives |
| `accumulator` | **Rust** (SMT builder) | same SMT code the circuit links |
| `token` (end-to-end burned blobs) | **TS/Rust mint+burn flow** | reuses live mint/burn; both SDKs already share fixtures |

### Who consumes what

| Component | Must pass |
|---|---|
| Contracts (Solidity tests) | `config`, `lock`, `public` (keccak/ABI recomputation), settlement vectors |
| TS SDK | `config`, `reason`, `nullifier`, `lock` (recipientCommitment), `public` |
| Prover (Rust) | all groups |

### CI gate

- Each repo vendors `bridge-vectors` at a pinned version and fails on mismatch.
- A repo bumping `BRIDGE_PROTO_VERSION` must regenerate and the others must
  re-pin — the version skew is itself a CI failure.
- The M2+ devnet end-to-end test is the integration backstop; vectors are the
  fast per-commit guard that needs no proving.

---

## 11. Change control

- This document + `bridge-vectors` move together; a change to either bumps
  `BRIDGE_PROTO_VERSION`.
- Anything in ZK_BACK3 §14 "Parameters To Freeze" must be reflected here before a
  component depends on it.
- Divergences from `appendix-bridging.tex` are listed inline (§1, §5, §7) and must
  be reconciled or explicitly accepted at M0.
