# bridge-vectors

Cross-stack **conformance vectors** for the Unicity asset bridge: the
machine-checkable form of
[`../docs/bridge/dev-plan/00-interop-contract.md`](../docs/bridge/dev-plan/00-interop-contract.md).
One reference generator produces them; the contracts (Solidity), the TS SDK, and
the prover (Rust) each consume them as test input. A component is "in sync" iff
its CI reproduces every vector for its subset.

**`BRIDGE_PROTO_VERSION`** is in [`VERSION`](./VERSION) (`= 1`). Every consumer
pins it; a version skew is a CI failure.

## Layout

```
bridge-vectors/
  VERSION                 # = BRIDGE_PROTO_VERSION
  gen/                    # the reference generator (zero-dependency Rust)
  config/      *.json     # config → tokenType/coinId, configHash        [implemented]
  lock/        *.json     # LockRecord → recipientCommitment, lockDigest  [implemented]
  reason/      *.json     # BridgeBackReason → reasonBytes + reasonHash    [implemented]
  nullifier/   *.json     # (stateId, txHash, configHash) → nullifier      [implemented]
  public/      *.json     # leaves/refs → roots; PublicValues → ABI        [implemented]
  accumulator/ *.json     # ordered nullifier stream → roots + witnesses   [implemented]
  token/       README.md  # burned-token blobs → relation outputs          [stub → M2]
```

Each JSON pairs explicit `in` fields with the expected `out` fields (hex). All
six implemented groups are threaded through one coherent example deployment, so
e.g. `nullifier-00`'s `config_hash` input equals `config-00`'s output.

## Generate

```sh
cd gen && cargo run        # writes ../<group>/*.json
```

The generator (`gen/src/`) is dependency-free: SHA-256 and Keccak-256 are inlined
and **self-tested against known vectors at startup** (`gen/src/hash.rs`), so it
builds offline and a transcription error fails loudly instead of emitting a wrong
reference. Each derivation has exactly one definition in `gen/src/derive.rs`,
cross-referenced to the `00` clause it implements.

## Consume (per component)

| Component | Must reproduce |
|---|---|
| Contracts (Solidity tests) | `config`, `lock`, `public` (keccak/ABI recompute), settlement |
| TS SDK | `config`, `reason`, `nullifier`, `lock` (recipientCommitment) |
| Prover (Rust) | all groups |

Recompute each `out` from the `in` with your implementation and assert equality.

## Hash policy (see `00` §1)

- **keccak256 / `abi.encode`** — `configHash`, `lockDigest`, `returnRoot`,
  `lockRefRoot`, `domainTag`, `PublicValues` (the vault recomputes these; native
  keccak is cheapest on-chain).
- **SHA-256 / deterministic CBOR** — `nullifier`, `burnTransitionId`,
  `tokenType`/`coinId`, `recipientCommitment`, the accumulator (these must match
  the Unicity SDK / Service; the vault never recomputes them).

## Provisional encodings to confirm at M0

The generator is the executable spec, so the few not-yet-pinned choices are
centralized here and flagged in code:

- **`reason` CBOR field types** (`gen/src/derive.rs::reason_cbor`): version /
  chainId / deadline as CBOR uints; addresses & ids as byte strings; amounts as
  minimal big-endian byte strings. Confirm against the SDK's deterministic-CBOR
  conventions when `BridgeBackReason` is added to the SDK.
- **`reasonHash = H(reasonBytes)`** (`reason` group `out.reason_hash`): SHA-256
  over the raw canonical reason bytes. This is the value the terminal burn's
  `BurnPredicate(H(reasonBytes))` binds, while `reasonBytes` itself rides in the
  burn transfer's auxiliary data (`00` §4). Confirm against the SDK's
  `BurnPredicate` convention; `appendix-bridging.tex` writes `H(REASON_TAG, R)`,
  to be reconciled to the same preimage.
- **ABI domain literal** (`gen/src/abi.rs`): the leading domain in `configHash` /
  `lockDigest` is a dynamic ABI `string` (matches ZK_BACK3). A `bytes32` domain
  constant would be all-static and cheaper to recompute; if chosen, change it in
  the generator and the contracts together and bump `BRIDGE_PROTO_VERSION`.

## Change control

This package and `00-interop-contract.md` move together. Any change to a
derivation bumps `BRIDGE_PROTO_VERSION`, regenerates the vectors, and forces every
consumer to re-pin.
