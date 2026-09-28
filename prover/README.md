# Bridge return prover

Rust workspace for the trustless bridge-back prover track.

This is the M0 scaffold from `docs/dev-plan/03-prover-service.md`:

- `crates/core` is the `no_std` byte-level contract used by the future SP1 guest
  and the host precheck.
- `crates/guest` is the SP1 guest relation shell. It validates the
  token/anchor/accumulator relation in normal Rust tests and exposes
  `execute_public_output` for the future SP1 IO wrapper.
- `crates/host` is the std host entry point and vector-check harness.

Run the current checks:

```sh
cargo test
cargo run -p bridge-return-host -- check-vectors ../protocol/vectors
cargo check -p bridge-return-guest --features sp1 --bin bridge-return-sp1-guest
```

The core crate intentionally reproduces the `BRIDGE_PROTO_VERSION=1`
derivations from `protocol/interop.md` rather than
depending on the dependency-free reference generator. The generator remains the
source of conformance fixtures; this workspace is a consuming implementation.

## SP1 verifier boundary

As of the current Succinct docs, on-chain verification is routed through the SP1
Solidity verifier/gateway interface with a verification key, public values, and
proof bytes, and Groth16 is the recommended on-chain proof mode
([Solidity verifier](https://docs.succinct.xyz/docs/sp1/verification/solidity-sdk),
[proof types](https://docs.succinct.xyz/docs/sp1/generating-proofs/proof-types)).
This scaffold keeps the bridge statement explicit as `PublicValues` ABI bytes
plus `public_values_digest(...)`. The guest crate now packages both through
`execute_public_output`; the exact SP1 release and verifier contract shape should
be pinned when M3 wires real proving.

The SP1 guest binary is feature-gated so normal workspace tests do not require
the SP1 toolchain:

```sh
cargo check -p bridge-return-guest --features sp1 --bin bridge-return-sp1-guest
cargo prove build -p bridge-return-guest --binaries bridge-return-sp1-guest --features sp1 --output-directory target/sp1 --elf-name bridge-return-sp1-guest
```

It reads the byte-oriented `GuestInput` wire format, calls `execute_wire`, then
commits `public_values_abi` followed by `public_values_digest`. The host can emit
fixture wire payloads for execute/prove plumbing:

```sh
cargo run -p bridge-return-host -- emit-b1-wire-input
cargo run -p bridge-return-host -- emit-split-wire-input
```

The host-side SP1 SDK plumbing is also feature-gated:

```sh
cargo check -p bridge-return-host --features sp1
cargo run -p bridge-return-host --features sp1 -- sp1-execute <guest.elf> <wire_hex>
cargo run -p bridge-return-host --features sp1 -- sp1-mock-groth16 <guest.elf> <wire_hex> <proof.bin>
cargo run -p bridge-return-host --features sp1 -- sp1-proof-info <proof.bin>
```

`sp1-execute` and `sp1-mock-groth16` precheck the same wire input through
`execute_wire` and reject if the SP1 public-values stream differs from the
expected ABI bytes plus digest.

The RISC-V ELF build path has been checked with the local SP1 toolchain and
emits `target/sp1/bridge-return-sp1-guest`. Full fixture `sp1-execute` is still
too expensive for routine local validation: the B=1 token fixture stayed
CPU-active for more than 15 minutes and was interrupted. Use the normal Rust
`check-vectors` path for fast conformance until there is a smaller SP1 smoke
fixture or a cheaper relation.

## Docker

`prover/Dockerfile` builds everything from the repository root (its build
context; `/.dockerignore` keeps the sibling checkouts and build output out):

```sh
# from the repository root
docker build -f prover/Dockerfile -t bridge-return-service .
docker build -f prover/Dockerfile --target vkey -o prover/target/docker .   # ELF + vkey only
docker compose up return-service                                           # port 8787
```

Stages: `guest` builds the SP1 guest ELF inside Succinct's own image for the
pinned SP1 release, which is the reproducible build a vault's verifying key
must match; `host` builds the host and service binaries with the SP1 host SDK
(needs `protoc` and Go 1.24 for the native Groth16 library) and derives the
vkey from the ELF (`vkey.json` / `vkey.txt`, no proving involved), then runs
`check-vectors`; `contracts` compiles the vault artifact the Node relayer
reads; `runtime` carries the service, the relayer and the frozen deployment
files. The `vkey` stage is the ELF and key alone, for `--output`.

The container starts in `precheck_only`. For real proofs set
`BRIDGE_RETURN_PROVE_MODE=sp1_groth16`; the first proof downloads the Groth16
circuit and proving key (about 6 GB) into the `sp1-artifacts` volume, and the
container needs about 16 GB of memory. Settlement runs through
`contracts/tron/scripts/relayer.js` until the all-Rust submitter lands; it
takes `TRON_SK`, `TRON_VAULT` and `TRON_RPC_URL` from the container
environment (the entrypoint writes them to the `.env` file the script reads).
