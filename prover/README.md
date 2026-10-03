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
docker build -f prover/Dockerfile --target guest-check .                          # the source still builds the pinned ELF
docker build -f prover/Dockerfile --target guest-from-source -o prover/guest-elf .   # a new guest, for a new vault
docker compose up return-service                                           # port 8787
```

The image proves with the guest ELF committed in `prover/guest-elf/`, the exact
program whose verifying key the deployed vaults hold; the image does not compile
the guest. A build of the same guest source on another day produced a different
ELF and so a different key, which no deployed vault accepts, so the binary is
pinned and the source build is kept only for cutting the guest of a new vault.

Stages: `host` builds the host and service binaries with the SP1 host SDK
(needs `protoc` and Go 1.24 for the native Groth16 library), checks the pinned
ELF against its `sha256` file, derives the vkey from it (`vkey.json` /
`vkey.txt`, no proving involved), fails the build unless that key is the one
the `GUEST_DEPLOYMENT` record pins (default `sepolia/sepolia-usdc.json`), then
runs `check-vectors`; `contracts` compiles the vault artifact the Node relayer
reads; `runtime` carries the service, the relayer and the frozen deployment
files. The `vkey` stage is the ELF and key alone, for `--output`.
`guest-from-source` compiles the guest inside Succinct's image for the pinned
SP1 release and outputs the ELF with its `sha256` file; committing that output
to `prover/guest-elf/` changes the key and needs a new vault.

The guest-side crates (`crates/guest`, `crates/core`, `crates/sdk-ext`),
`Cargo.toml` and `Cargo.lock` are the source of the pinned ELF, and every line
of them is part of the program: panic locations carry line numbers, so removing
a comment above one changes the binary and its key. The `guest-check` target
rebuilds the guest from source and fails unless the result equals the pinned ELF
byte for byte; the `Guest ELF` workflow runs it on every change to those paths.
The SP1 image and `cargo-prove` are pinned by digest and `cargo prove build`
runs `--locked`, which is what makes the rebuild deterministic.

At start in `sp1_groth16` mode the service derives the key of the ELF at
`SP1_GUEST_ELF` and compares it with `deployment.vkey` in
`BRIDGE_DEPLOYMENT_CONFIG`. On a mismatch, a missing ELF or a build without
the `sp1` feature it exits with the reason instead of producing proofs the
vault would reject. Deriving the key is the SP1 program setup and takes one to
two minutes before the service listens.

The container starts in `precheck_only`. For real proofs set
`BRIDGE_RETURN_PROVE_MODE=sp1_groth16`; the first proof downloads the Groth16
circuit and proving key (about 6 GB) into the `sp1-artifacts` volume, and the
container needs about 16 GB of memory. Settlement runs through
`contracts/tron/scripts/relayer.js` until the all-Rust submitter lands; it
takes `TRON_SK`, `TRON_VAULT` and `TRON_RPC_URL` from the container
environment (the entrypoint writes them to the `.env` file the script reads).
