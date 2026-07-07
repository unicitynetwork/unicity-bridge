# 03 — Prover service

**Stack:** Rust. The verification logic is the `no_std` core of
`state-transition-sdk-rust`; the zk relation is an **SP1 guest program** (RISC-V
zkVM) wrapping that core; the proof is STARK→Groth16 for cheap on-chain
verification. The off-chain pipeline (host) is std Rust services.

**Owns:** the batch return relation `R(x,w)` (ZK_BACK3 §7–8, `appendix-bridging.tex`
"Batch Proving Relation"), proof generation, and the witness/accumulator/sequencer
services (ZK_BACK3 §10). Conforms to [`00-interop-contract.md`](./00-interop-contract.md)
§1, §4–9.

**Reference:** ZK_BACK3 §6–8, §10–11; `appendix-bridging.tex` relation + batching.

---

## Why Rust/SP1 fits

`state-transition-sdk-rust` is explicitly `no_std`-first and "runs inside a zkVM
guest (SP1 / RISC0)" (its README). It already provides, in the verification core
(no features needed):

- `Token::from_cbor` + `verify_token_with(token, trust_base, registry)` — full
  chain-linkage + owner-authorization + certificate checks (`src/verify/mod.rs`).
- `MintJustificationRegistry` — the in-circuit extension point for the bridge
  backing reason (same shape the split verifier uses).
- RSMST split verification — value conservation on the path to the burned leaf
  (`src/payment/`, `src/rsmst/`).
- `BurnPredicate` (`0x02`) with a reason payload (`src/predicate/builtin.rs`).
- The radix SMT + inclusion certificate (`src/api/inclusion_certificate.rs`,
  `src/radix.rs`) — the basis for both anchored inclusion and the nullifier
  accumulator.

So the circuit is mostly *composition* of existing verified code, plus three new
capabilities the SDK lacks today.

---

## SDK extensions (new, in `state-transition-sdk-rust`)

These are `no_std` additions so the guest can link them. Each ships with host
tests **and** generates `bridge-vectors`.

### E1 — Anchored-mode inclusion (ZK_BACK3 §2.1, §7.1)
Today `verify_inclusion_proof` verifies each transition's *own*
`UnicityCertificate` (one BFT-quorum check per transition,
`src/verify/mod.rs::verify_unicity_certificate`). The batch needs: verify **one**
anchor certificate `UC*` → `h*`, then verify each transition by an SMT inclusion
path against the shared `h*`.
- Add `verify_inclusion_against_root(state_id, tx_hash, h*, cert) -> bool` (the
  existing `InclusionCertificate::verify` already takes an expected root — expose
  an anchored entry point that skips the per-proof `verify_unicity_certificate`).
- Add an "anchored" verification mode to the token walk that takes `UC*`/`h*` once
  and an `AnchoredInclusionProof` per transition (ZK_BACK3 §6).
- **Soundness precondition** (ZK_BACK3 §2.1): the Service must serve historical
  inclusion proofs against a later root. If a deployment cannot, anchored mode is
  invalid — document the fallback to per-transition certs.
- **v1 predicate-time scope** (ZK_BACK3 §2.1, §7.2; `00` §8): anchored mode drops
  the original `UC.IR.t` from the predicate call, so the walk MUST accept only
  time-independent predicate families (signature/burn/split) and **reject any
  predicate whose validity depends on validation time**. Timelocks/HTLCs are out
  of scope until a later circuit carries authenticated original time per
  transition. Encode this as an explicit allow-list of predicate types in the
  anchored walk, not an implicit assumption.

### E2 — Nullifier accumulator with non-membership (`00` §6)
The SDK's SMT exposes inclusion; add the presence-set operations:
- `non_membership_witness(tree, key) -> w`, `verify_non_member(root, key, w) -> bool`,
  `insert(root, key, w) -> root'` (presence value `0x01`, prefixes `0x00/0x01`,
  SHA-256).
- An **ordered batch insert** helper that threads the root and emits each `w_k`
  against the intermediate root after `0..k-1` (the order-coupling in `00` §6 /
  ZK_BACK3 §7.4). This is the single most bug-prone invariant; test it hard.

### E3 — Structural backing verifier (ZK_BACK3 §7.3, appendix relation)
A `MintJustificationVerifier` for the bridged-token genesis backing reason that is
**structural**: it recomputes `lockDigest` from the certified genesis + `config`,
checks `tokenId`/`recipientCommitment`/`coinId`/`amount` bindings, and **exports
`(nonce, digest)` as a public obligation** — it does **not** call an external
`LockFinal` oracle (external finality was settled at mint time; the vault confirms
the digest against `lockDigest[nonce]`). Contrast the TS bridge-in verifier (`02`),
which *does* query RPC; here we only re-bind structurally.

---

## The guest circuit (SP1 program)

New workspace `prover/` (Cargo workspace): `guest/` (the SP1 program) +
`host/` (proving + services) + `bridge-vectors-gen/` (fixture generator binary).

The guest implements `R(x,w)` (ZK_BACK3 §8, appendix Alg. "BridgeReturnRelation"):

```
once per batch:
  A0  require K(config) == x.configHash            // keccak/ABI (00 §1)
  A1  require H(trust_base) == x.trustBaseHash
  A2  verify UC* under trust_base; h* = UC*.IR.hash // the ONE BFT-quorum check
  A3  running = x.spentRootOld; V = 0

per burn i, in return-leaf order:
  F   verify_token (anchored mode vs h*)  [E1] — chain linkage + time-independent owner auth
      require token type == config.tokenType
  V   RSMST split/value lineage → certified value v_i for coinId  [existing]
  D   reasonBytes := terminal burn transfer aux data
      require terminal recipient == BurnPredicate(H(reasonBytes))
      decode BridgeBackReason from reasonBytes
      require reason fields == config; reason.amount == v_i; feeAmount ≤ v_i
  B   reconstruct LockRecord from genesis + config; d_i = lockDigest  [E3]
      require id/recipientCommitment/coinId/amount bindings
  R   burnTransitionId, nullifier (00 §5)
      require verify_non_member(running, η_i, w_i); running = insert(...)  [E2]
  E   append ReturnLeaf_i; record (nonce_i, d_i)

after batch:
  C   require running == x.spentRootNew; require V == x.totalAmount
      require K(ReturnLeaf[]) == x.returnRoot
      require K(dedupSort(lockRef[])) == x.lockRefRoot
      commit x  (PublicValues, 00 §7)
```

Notes:
- `K` (keccak/ABI) is linked into the guest only for `configHash`, `lockDigest`,
  `returnRoot`, `lockRefRoot`, `domainTag`; everything else is `H` (`00` §1).
- **v1 rejects `source_locks.len() != 1`** (ZK_BACK3 §7.3) — multi-lock merge is a
  reserved future primitive.
- The guest commits `PublicValues` as its public output; the host wraps to
  Groth16 so the vault verifies one bn254 proof.

---

## Off-chain pipeline (host, std Rust)

Four replaceable services (ZK_BACK3 §10). None hold authority; a stalled one is
replaceable from public data.

### S1 — Witness builder (§10.1)
Decode burned blobs; choose an anchor root `R*`; fetch compact anchored inclusion
proofs for every required transition; reconstruct `LockRecord`s from source-chain
`Lock` events; run a **host-side precheck** that mirrors the guest (fail fast
before proving). Uses the SDK's `http` feature (`HttpAggregatorClient`) to query
the aggregator/gateway.

### S2 — Accumulator builder (§10.2)
Rebuild the indexed Merkle tree from `EMPTY_TREE_ROOT` by replaying successful
`BatchFulfilled` + `Released(nullifier,…)` events in chain order; produce
order-coupled non-membership witnesses (E2); cache after successful batches;
rebase pending batches when another batch advances `spentRoot`. **Its tree is a
cache over public data, not a trusted DB** — it must reproduce the on-chain
`spentRoot` before use.

### S3 — Prover (§10.3)
Receive witness packages; run SP1 prove; wrap STARK→Groth16; expose artifacts.
GPU/proving-network capable; this is the heavy compute.

### S4 — Sequencer / relayer (§10.4)
Select burns; serialize batches on the current `spentRoot`; **pre-simulate
transfers and drop recipients that would revert** (batch-atomicity liveness, `01`
/ ZK_BACK3 §9); submit `fulfillBatch`; retry/rebase on `"stale root"`. A single
sequencer improves liveness but is untrusted.

---

## Phased work

| Milestone | Prover deliverables |
|---|---|
| **M0** | scaffold `prover/` workspace; design `GuestInput`/`PublicValues` Rust types matching `00` §6/§7; confirm the SP1 Groth16 export exposes a **single public input** (the public-values digest) so the on-chain verifier stays within Tron's ~80 ms dry-run / energy budget (`01`) |
| **M2** | E1+E2+E3 in the SDK with host tests + vector generators; guest relation for **B=1**; run in SP1 **execute** mode (no proof) to validate logic against `bridge-vectors/token`; S1 + S2 minimal |
| **M3** | SP1 **prove** + Groth16 wrap; publish `VKEY`; one real proof verified on-chain (`01` M3); S3 real prover |
| **M4** | ordered multi-burn batch (B>1) sharing one `UC*` + one accumulator transition; S2 order-coupled witnesses at batch scale; S4 sequencer/relayer; measure `B_max` |
| **M5** | recursion/aggregation (prove lineages in parallel, aggregate child proofs — vault interface unchanged, ZK_BACK3 §11.4); optional settlement aggregation (`SettlementLeaf[]`, §11.3) if measured transfer cost dominates |

---

## Testing & CI

- **SDK unit tests** (host) for E1/E2/E3, adversarial one-field-tamper style like
  the existing `src/verify/mod.rs` tests — especially the accumulator ordering
  (E2): a witness built against `spentRootOld` must fail for any burn after the
  first.
- **`bridge-vectors` generation + self-check**: the Rust generator is the
  reference for `config/lock/reason/nullifier/accumulator/public`; a host test
  re-reads its own output and the guest (execute mode) consumes `token` vectors.
- **Differential**: SP1 *execute* output of the relation vs the S1 host precheck
  must agree on every public value before any proof is generated.
- **Proving smoke** (M3+): prove B=1, verify the Groth16 proof with the same
  verifier contract bytecode the vault uses (`01`).

## Risks

| Risk | Mitigation |
|---|---|
| Anchored mode invalid if Service lacks historical inclusion proofs (ZK_BACK3 §2.1) | confirm at M0 against the target network; fallback to per-transition certs documented |
| Time-dependent predicate (timelock/HTLC) in a token's lineage | v1 anchored mode drops `UC.IR.t` and rejects such predicates via an explicit allow-list — out of scope until authenticated original time is carried (`00` §8) |
| Accumulator order-coupling bugs (E2) | dedicated adversarial tests; explicit duplicate-nullifier check in-circuit (ZK_BACK3 §7.4) |
| Proving time for long lineages | recursion/parallel lineage proofs (M5); measure per `00`/§14 |
| Circuit drifts from SDK semantics | the circuit *reuses* the SDK core rather than re-implementing it (E1–E3 are thin); ZK_BACK3 §12 req. 2 |
| keccak-in-guest cost | only 5 values use `K` (SP1 has an accelerated keccak syscall); rest is `H` (`00` §1) |
| Tron Groth16 verifier energy / 80 ms dry-run limit (issue #6288) | bn254 works on Tron (`0x06/07/08`); the Groth16 wrap exposes one public input, keeping the verifier small; measure energy on Nile (`01`) |
