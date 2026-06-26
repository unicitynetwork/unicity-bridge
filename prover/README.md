# Bridge return prover

Rust workspace for the trustless bridge-back prover track.

This is the M0 scaffold from `docs/bridge/dev-plan/03-prover-service.md`:

- `crates/core` is the `no_std` byte-level contract used by the future SP1 guest
  and the host precheck.
- `crates/guest` is the SP1 guest relation shell. It currently validates and
  returns supplied public values; E1-E3 in the Rust SDK will replace the witness
  placeholder with the real token/anchor/accumulator relation.
- `crates/host` is the std host entry point and vector-check harness.

Run the current checks:

```sh
cargo test
cargo run -p bridge-return-host -- check-vectors ../bridge-vectors
```

The core crate intentionally reproduces the `BRIDGE_PROTO_VERSION=1`
derivations from `docs/bridge/dev-plan/00-interop-contract.md` rather than
depending on the dependency-free reference generator. The generator remains the
source of conformance fixtures; this workspace is a consuming implementation.

## SP1 verifier boundary

As of the current Succinct docs, on-chain verification is routed through the SP1
Solidity verifier/gateway interface with a verification key, public values, and
proof bytes, and Groth16 is the recommended on-chain proof mode
([Solidity verifier](https://docs.succinct.xyz/docs/sp1/verification/solidity-sdk),
[proof types](https://docs.succinct.xyz/docs/sp1/generating-proofs/proof-types)).
This scaffold therefore keeps the bridge statement explicit as
`PublicValues` ABI bytes plus `public_values_digest(...)`; the exact SP1 release
and verifier contract shape should be pinned when M3 wires real proving.
