# 01 — Source-chain contracts

**Stack:** Solidity `^0.8.24`, Hardhat (`@nomicfoundation/hardhat-toolbox`),
target TVM (Tron) first, EVM-compatible. Lives in `contracts/tron/` (extend) and
a parallel `contracts/evm/` if/when an EVM deployment is needed (same sources,
different deploy tooling).

**Owns:** custody of locked assets, the bridge-in lock record, on-chain proof
verification, the replay accumulator root, and settlement. Conforms to
[`00-interop-contract.md`](./00-interop-contract.md) §1–3, §7, §9.

**Reference:** ZK_BACK3 §3, §9; `appendix-bridging.tex` "Settlement".

---

## Scope note

The existing `UnicityLock.sol` return path (`unlock`/`withdrawn`, per-nonce single
withdrawal) is **superseded and not reused** — it is incompatible with ZK_BACK3:
releases are keyed by **nullifier**, not lock nonce, because a deposit split on
Unicity returns as many independent tokens; it also has no fee/deadline, no
accumulator (O(locks) replay storage instead of one root), and no proof
verification. We build the vault below fresh. The only things lifted from
`UnicityLock` are the **TRC20 safe-transfer helpers** (USDT returns no bool) and
the **reentrancy guard**.

---

## The vault (one contract, per ZK_BACK3)

A single fresh contract holds custody, the bridge-in `lockDigest` map, and the
return-side accumulator root + settlement. There is no separate return contract
and no callback into an old lock contract: ZK_BACK3 assumes one contract owns
custody + `lockDigest` + `spentRoot`, and a split would re-introduce a
custody/authority seam.

```solidity
contract UnicityBridgeVault {            // the bridge vault: lock-in + accumulator return
    // immutables, all derived from one BridgeConfig in the constructor
    IProofVerifier immutable verifier;   // SP1/Groth16 verifier (see §"Proof verification")
    bytes32 immutable VKEY;
    bytes32 immutable DOMAIN_TAG;        // K("unicity-bridge-return:v1")
    bytes32 immutable CONFIG_HASH;       // derived from cfg (00 §2)
    IERC20  immutable ASSET;             // = cfg.asset (cannot diverge from CONFIG_HASH)

    mapping(uint256 => bytes32) public lockDigest;        // bridge-in (00 §3)
    uint256 public nextNonce;
    mapping(bytes32 => bool)    public trustBaseAllowed;  // timelocked governance
    bytes32 public spentRoot;                             // init EMPTY_TREE_ROOT

    // bridge-in: keep lock(), but ALSO store lockDigest[nonce] and emit full fields
    function lock(uint256 amount, bytes32 unicityTokenId, bytes32 recipientCommitment)
        external returns (uint256 nonce);

    // bridge-back: the whole return path
    function fulfillBatch(
        bytes  calldata publicValues,
        bytes  calldata proof,
        ReturnLeaf[]    calldata leaves,
        SourceLockRef[] calldata lockRefs
    ) external nonReentrant;
}
```

`fulfillBatch` is ZK_BACK3 §9 verbatim in intent:

1. `verifier.verifyProof(VKEY, publicValues, proof)`.
2. decode `PublicValues`; check `domainTag`, `configHash`, `trustBaseAllowed`,
   `spentRootOld == spentRoot`, `batchSize`, `returnRoot`, `lockRefRoot`,
   lock-refs sorted-unique.
3. one `SLOAD` per unique lock ref: `lockDigest[nonce] == digest`.
4. sum + per-leaf `feeAmount ≤ amount`; `total == totalAmount`.
5. `spentRoot = spentRootNew`; emit `BatchFulfilled`.
6. per leaf: `fee = now ≤ deadline ? feeAmount : 0`; transfer `amount-fee` to
   recipient, `fee` to feeRecipient; emit `Released(nullifier, …)`.

All validation precedes transfers; reentrancy guard on; safe-transfer helper
retained for USDT.

### lockDigest provenance (fixed)

`lock()` computes and stores `lockDigest[nonce]` *at lock time* from
`amount/unicityTokenId/recipientCommitment` + the config immutables (`00` §3).
This keeps return-time work to a single `SLOAD` per unique lock and removes any
need to retain the full `LockRecord` struct. The `Lock` event still emits the full
fields for the TS verifier and explorers.

---

## Proof verification on-chain

The vault calls an `IProofVerifier` (SP1's `ISP1Verifier`-style) that checks a
Groth16 proof over a bn254 curve. This is the cost-critical and
chain-risk-critical piece.

- **EVM:** use the canonical SP1 Groth16 verifier contract (bn254 precompiles
  `0x06/0x07/0x08` are standard).
- **Tron (TVM):** bn254 add/mul/pairing precompiles `0x06/0x07/0x08` operate on
  the same alt_bn128 parameters as Ethereum, so a standard snarkjs/SP1-generated
  Groth16 verifier executes correctly. (Tron's documented precompile deviations
  are at `0x03` and `0x09` — neither used by Groth16; SHA-256 `0x02` and native
  `keccak256` are available; there is no Poseidon precompile, but Groth16 needs
  none.) Two real constraints remain: **energy budgeting** — cost is roughly
  comparable to Ethereum's ~200k gas but denominated in energy, so measure it on
  Nile — and the **~80 ms CPU limit on `triggerconstantcontract` dry-runs** for
  verifiers with many public inputs (Tron issue #6288). Both are mitigated by
  SP1's Groth16 wrap exposing a **single public input** (the public-values
  digest), keeping the on-chain verifier minimal. Deploy the verifier as a TVM
  contract via tronbox/TronWeb.
- The verifier address + `VKEY` are immutable per circuit version; a circuit
  upgrade is a new vault deployment (or a governed verifier swap behind timelock,
  M5).

---

## Phased work

### M0 — design lock-in
- Scaffold the fresh ZK_BACK3 vault (lock-in + accumulator return in one
  contract); no reuse of the old `unlock`/`withdrawn` model.
- Hash policy is fixed (`00` §1): keccak/ABI for
  `configHash/lockDigest/returnRoot/lockRefRoot/domainTag`.
- Deploy a standard Groth16 verifier (snarkjs/SP1) to Nile and **measure energy**
  and dry-run latency for a single-public-input proof. The proof system is already
  chosen; this is budgeting, not a feasibility gate.

### M1 — bridge-in hardened
- Add `lockDigest[nonce]` storage to `lock()`; keep event fields.
- Solidity conformance tests against `bridge-vectors/config` + `/lock` (recompute
  `configHash`, `lockDigest` with keccak/ABI and assert equality with fixtures).
- Re-audit `lock()` invariants (`tokenIdUsed`, zero checks, CEI order).

### M2 — return path with a MOCK verifier
- `fulfillBatch` end-to-end against a `MockProofVerifier` that always accepts, so
  the **settlement logic** (root transition, lock-ref checks, fee/deadline,
  atomicity, events) is testable without proving.
- Solidity tests from `bridge-vectors/public` (recompute `returnRoot`/
  `lockRefRoot`, decode `PublicValues`) and settlement vectors (fee gating,
  sorted-unique, stale-root reject, duplicate-leaf reject).
- Emit `Released`/`BatchFulfilled` exactly per `00` §6/§9 (accumulator rebuild
  depends on these).

### M3 — real verifier
- Wire the real SP1 Groth16 verifier (EVM first, then TVM port).
- Deploy to Nile; settle one real proof produced by the prover track.
- Pin `VKEY` from the prover's published circuit version.

### M4 — batching
- No interface change (the vault is already batch-shaped). Add tests for N-leaf
  batches, gas/energy ceilings, `B_max` discovery under TVM block limits.

### M5 — production
- Timelocked governance for `trustBaseAllowed` add/rotate (validator-set epochs)
  and for any verifier swap; `Δ_gov ≥ 7d` (appendix params).
- Pull-payment settlement option for blockable recipients (ZK_BACK3 §9 batch
  atomicity note): credit `owed[recipient]` + `withdraw()` behind a deploy flag.
- External audit; publish mainnet `configHash`, `VKEY`, allow-listed trust-base
  hashes.

---

## Testing & CI

- Hardhat unit tests (EVM semantics) — existing `test/` pattern.
- Conformance tests consuming `bridge-vectors` (`00` §10): the vault is a
  consumer of `config`, `lock`, `public`, settlement groups. **Build fails on
  vector mismatch.**
- Property tests: stale-root rejection, duplicate nullifier/leaf, unsorted lock
  refs, fee > amount, reentrancy, no-return TRC20.
- Tron-specific: compile with tronbox/solc + deploy via TronWeb to Nile for the
  energy/precompile measurements (hardhat is EVM-only; see existing
  `hardhat.config.js` note).

## Risks

| Risk | Mitigation |
|---|---|
| Tron Groth16 energy cost / 80 ms dry-run limit (issue #6288) | precompiles `0x06/07/08` work; SP1's single public input keeps the verifier small; measure energy on Nile at M0 |
| Fresh vault address changes the TS `lockContract` trust anchor | coordinate the new address into `bridge-plugin-*` config + manifest at redeploy |
| Vault holds pooled custody (no per-lock earmark) | matches ZK_BACK3 solvency argument; covered by value-conservation in the circuit, not on-chain |
| `EMPTY_TREE_ROOT` mismatch with circuit/builder | pinned in `bridge-vectors/accumulator`; asserted in tests |
