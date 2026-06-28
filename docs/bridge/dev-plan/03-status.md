# 03 — Prover Service Status

Last updated: 2026-06-27

This status is for parallel-agent coordination. The workspace is multi-repo:
`prover/`, `bridge-vectors/`, and this file belong to `br`.
`state-transition-sdk-rust` is treated as a read-only reference; bridge-return
extensions live in `prover/crates/sdk-ext`.

## Implemented

### `br` repo

- Added `prover/` Cargo workspace with:
  - `crates/core`: byte-level bridge return derivations and public-value checks.
  - `crates/guest`: no_std guest relation shell, currently executable under
    normal Rust tests.
  - `crates/host`: vector checker CLI.
  - `crates/sdk-ext`: prover-owned extensions over the read-only Rust SDK.
- Guest relation now validates through `bridge-return-sdk-ext`:
  - `PublicValues` roots and totals;
  - ordered nullifier accumulator transition;
  - direct bridge-lock burned tokens for B=1;
  - anchored token verification through the SDK;
  - `trustBaseHash`;
  - terminal `BurnPredicate(SHA256(reasonBytes))`;
  - `BridgeBackReason` fields, fee bound, burn transition id, and nullifier;
  - bridge-lock obligations `(nonce, lockDigest)`;
  - payment amount for configured `coinId`.
- Guest public-output boundary now exposes:
  - `execute_public_output`;
  - `execute_wire`;
  - `wire::{encode_guest_input, decode_guest_input}`;
  - `public_values_abi`;
  - `public_values_digest`, the single digest intended for the SP1/Groth16
    public input boundary.
- Added feature-gated SP1 guest binary:
  - `bridge-return-sp1-guest`;
  - enabled by `bridge-return-guest/sp1`;
  - reads the byte-oriented `GuestInput` wire payload;
  - commits `public_values_abi` followed by `public_values_digest`.
- Added feature-gated SP1 host SDK plumbing:
  - `bridge-return-host/sp1`;
  - `sp1-execute <guest.elf> <wire_hex>`;
  - `sp1-mock-groth16 <guest.elf> <wire_hex> <proof.bin>`;
  - `sp1-proof-info <proof.bin>`;
  - host prechecks wire input through `execute_wire` and rejects if the SP1
    public-values stream differs from the expected ABI bytes plus digest.
- Added project-owned on-chain proof-metadata plumbing:
  - `Sp1ProofInfo.vkey_hash` now carries the program verifying-key hash
    (`bytes32`) that the source-chain vault's `verifyProof(programVKey, …)`
    binds to; the groth16 prove paths populate it from the proving key.
  - `sp1-vkey <guest.elf>` derives the program vkey hash from the ELF alone
    (cheap setup only, no proving) and prints it with the SP1 circuit version.
  - `sp1-export <guest.elf> <proof.bin> <bundle.json>` re-derives the vkey,
    **re-verifies the saved proof against it**, and writes the publishable
    `(programVKey, publicValues, proofBytes)` bundle the vault consumes.
- Published the first real B=1 Groth16 on-chain bundle as
  `bridge-vectors/proof/b1-groth16.json` (`BRIDGE_PROTO_VERSION=1`):
  - `vkey = 0x004d100af488ce9a36e6e44a71b8dced18aa6a55cf3634151ac7b5609302133f`
    (SP1 circuit `v6.1.0`);
  - `proof_mode = groth16`, `proof_bytes_len = 356` (4-byte selector `0x4388a21c`);
  - `public_values` ends in the 32-byte digest
    `0xe95026138c4b607eaaee2146438fd85f19c740e93833d5bbbf28683e09776dae`.
- Added `prover/crates/host/tests/b1_guest.rs`, an executable B=1 scenario that
  constructs a direct bridge-lock token, burns it to `BridgeBackReason`, and runs
  `bridge_return_guest::execute`.
- Added `prover/crates/host/src/fixture.rs`, a reusable B=1 fixture builder used
  by the integration test and vector emitter.
- Added a **B=2 batch** fixture (`build_b2_direct_bridge_fixture`): two independent
  direct bridge-lock tokens burned to distinct `BridgeBackReason`s, sharing one
  trust base, with one ordered accumulator transition over both nullifiers and two
  source lock refs sorted by nonce. This variant uses **per-burn anchors** (one
  2-leaf tree + one `UC*` per token).
- Added a **shared-anchor B=2** fixture (`build_b2_shared_anchor_fixture`) +
  multi-leaf inclusion-path builder (`multi_leaf_paths`): all four transitions
  (each token's mint + burn) are leaves of a **single** sparse Merkle tree, so
  one shared `UC*` anchors the whole batch (the §11 one-quorum-check shape). The
  builder matches the SDK `InclusionCertificate::verify` convention (LSB-first
  key bits; depth 255 nearest the leaf, depth 0 nearest the root). The committed
  public values are byte-identical to the per-anchor B=2 fixture — only the
  anchoring shape differs.
- **Guest-side one-quorum dedup (§11):** `validate_bridge_burns` now verifies
  each *distinct* anchor `UC*` exactly once — burns sharing a byte-identical
  anchor reuse the cached verified root — so a shared-anchor batch pays one
  BFT-quorum (secp256k1) check instead of one per burn. New `sdk-ext` entry
  points support this without re-running the quorum: `verify_token_against_root`
  and `bridge_lock_obligations_for_token_against_root` (verify a token in
  anchored mode against an already-verified root); `verify_token_anchored` is
  refactored to `verify_anchor_certificate` + `verify_token_against_root`.
- Added `prover/crates/host/tests/b2_guest.rs`: executes both B=2 variants and
  asserts the order-coupled invariants (swapped accumulator witnesses, unsorted
  lock refs, wrong batch size, swapped leaves, dropped burn witness all reject),
  plus that the shared-anchor batch carries one byte-identical `UC*` (vs two
  distinct anchors per-burn), yields the same public values, and that the dedup
  still enforces the quorum (unsigned shared anchor rejects; unsigned second
  anchor in a per-burn batch rejects). M4 relation validated in execute mode.
- Added `emit-b2-shared-wire-input` (the shared-anchor B=2 wire payload).
- Added `bridge-vectors/accumulator/accumulator-00.json`.
- Added `bridge-vectors/token/token-00.json`, the M2 B=1 direct bridge-lock
  execute fixture.
- Added `bridge-vectors/token/token-01.json`, the M2 B=1 split-source execute
  fixture that burns a split output and recursively extracts the original source
  lock obligation.
- Added `bridge-vectors/token/token-02.json`, the **B=2 multi-burn** execute
  fixture. It uses the multi-burn schema: one `(token_cbor, trust_base,
  anchor_certificate_cbor, lock_justification_tag)` entry per burn under
  `in.burns`, with `leaves`, `lock_refs`, `accumulator_witnesses`, and
  `guest_wire_input` shared at the batch level.
- `bridge-return-host check-vectors` now consumes `token/token-00.json`,
  `token/token-01.json`, and `token/token-02.json`, then runs the guest relation
  in execute mode. `check_token` accepts both schemas: the single-burn vector
  (`token_cbor`/… at the top of `in`) is treated as a one-element batch, and the
  multi-burn vector iterates `in.burns`.
- `bridge-return-host emit-b1-token-vector` prints the generated B=1 token vector;
  its output currently matches `bridge-vectors/token/token-00.json`.
- `bridge-return-host emit-split-token-vector` prints the generated split-source
  token vector; its output currently matches `bridge-vectors/token/token-01.json`.
- `bridge-return-host emit-b2-token-vector` prints the generated B=2 multi-burn
  token vector; its output currently matches `bridge-vectors/token/token-02.json`.
- **Froze the canonical Nile-USDT config** (`bridge-vectors/deployment/nile-usdt.json`):
  the full `BridgeConfig` for the live deployment with derived `token_type` /
  `coin_id` / `config_hash` / `domain_tag` + the deployed addresses + the
  testnet2 trust-base `canonical_hash` (`0x72a672…358c0`). The `config_hash`
  (`0xe06d52…204203`) equals the deployed vault's on-chain `CONFIG_HASH` — the
  cross-stack freeze (Rust prover == Solidity vault). New host commands
  `emit-config` (authoritative derivation from inputs) and `emit-trust-base-hash`;
  `tests/nile_config.rs` re-derives and guards it against drift.
- Added the **S1 witness package + host precheck** (`crates/host/src/s1.rs`,
  ZK_BACK3 §10.1):
  - `WitnessPackage` wraps the `GuestInput` S1 hands to the prover (S3);
  - `WitnessPackage::precheck` mirrors the guest by running the exact entry
    points (`execute_public_output` + `execute_wire`), confirms the committed
    public values equal the computed ones, and round-trips the wire encoding to
    catch encode/decode drift before the expensive prove — no SP1 dependency, so
    it runs under plain `cargo test`;
  - `PrecheckReport` returns the public values, ABI bytes, digest, batch size,
    total amount, and the exact wire payload for the prover;
  - `s1::precheck_wire(bytes)` is the standalone gate over a raw wire payload.
  - `bridge-return-host precheck-wire <wire_hex>` exposes it on the CLI.
  - `crates/host/tests/s1_precheck.rs` covers B=1/split/B=2 accept and tampered
    public-values / truncated-wire reject.
- **Guest certified-mode relation path.** The guest now carries a per-burn
  verification mode (`BurnVerification::Anchored(UC*) | Certified`); certified
  burns verify each transition against its own `UnicityCertificate` (live
  aggregator tokens), anchored burns keep the §11 shared-`UC*` dedup. Wire format
  bumped to **v2** (per-burn mode tag); token vectors regenerated.
  `s1::build_certified_guest_input` assembles a B=1 certified `GuestInput`, and
  `s1_live.rs::guest_relation_accepts_live_certified_token` runs a **real live
  token through the full guest relation** (execute + wire round-trip). The same
  live token runs in the **zkVM** (`sp1-execute`, via
  `examples/emit_certified_live_wire`) at **1,861,507 cycles** with
  `public_values == expected` — a real aggregator token validated in-circuit
  (certified mode does one quorum check per transition: genesis + burn).
- **S1 aggregator HTTP client** (`s1::aggregator`, host `http` feature): wraps the
  SDK's blocking `HttpAggregatorClient` — `client_from_env` builds it from
  `UNICITY_GATEWAY` (+ optional `UNICITY_API_KEY`); `fetch_inclusion_proof` /
  `fetch_terminal_inclusion_proof` pull a transition's proof from the live
  gateway. `tests/s1_aggregator.rs` checks construction (no network) and has an
  `#[ignore]` live fetch (run with the repo `.env` exported) — **verified working
  against the live testnet2 gateway** (pulled the sample token's terminal
  inclusion proof over the network). The TLS stack is behind the `http` feature
  so the default build stays lean.
- **S1 certified-mode verification of live tokens** (`s1::verify_certified_burn`,
  ZK_BACK3 §10.1): full cryptographic verification of a real aggregator-served
  token (each transition carries its own `UnicityCertificate`) against the
  testnet2 trust base — quorum + chain linkage + owner auth + the bridge-lock
  obligation — not just the structural byte derivations. New public
  `bridge_lock_obligations_for_token_certified` in `sdk-ext`; the host crate now
  enables `unicity-token/std` for `RootTrustBase::from_json` (the zkVM guest build
  is a separate `cargo prove` invocation and stays `no_std`).
  - `examples/cross_check_live.rs` now runs this full verification (quorum 3-of-4)
    on the live blob *before* the byte-derivation cross-checks.
  - `crates/host/tests/s1_live.rs` froze a real `npm run e2e:back` sample
    (`tests/data/bridge-back-live-sample.json` + `trustbase.testnet2.json`,
    public testnet artifacts, no secrets) and asserts it verifies (lockDigest +
    nonce match the TS wallet) and that an unsatisfiable quorum rejects it. This
    is the first CI test over **real aggregator data**, not synthetic fixtures.
- Token vectors now include `in.guest_wire_input`, the exact wire payload consumed
  by the SP1 guest binary. `check-vectors` executes both the decoded JSON relation
  input and the raw wire input.
- `bridge-return-host emit-b1-wire-input` and `emit-split-wire-input` print the
  fixture wire payloads as hex.
- `.gitignore` now unignores `prover/Cargo.lock` so the prover host build lockfile
  can be tracked.

### `prover/crates/sdk-ext`

- Added anchored direct-token verification APIs:
  - `verify_anchor_certificate`
  - `verify_inclusion_against_root`
  - `verify_token_anchored`
- Added certified-token verification for embedded split sources:
  - `verify_token_certified`
  - verifies each embedded source transition against its own stored
    `UnicityCertificate`, matching the SDK's current certified-token wire format.
- Added `trust::canonical_hash()` used by the public `trustBaseHash`.
- Added nullifier accumulator module:
  - `verify_non_member`
  - `insert`
  - host `NullifierTree`
  - `ordered_insert_witnesses`
- Added structural bridge-lock backing verifier:
  - `bridge_lock_obligation`
  - `bridge_lock_obligations_for_token_anchored`
  - `TRON_USDT_LOCK_JUSTIFICATION_TAG`
- Added recursive split-source bridge-token obligation collection:
  - parses `SplitMintJustification`;
  - verifies the top-level returned token under the batch anchor;
  - verifies embedded burned source tokens against their own certificates to avoid
    a self-referential token hash in the current SDK encoding;
  - checks manifest burn predicates, token type, payment data, and RSMST proofs;
  - collects the underlying bridge-lock obligations for the guest relation.
- This crate depends only on public `state-transition-sdk-rust` APIs.

### `state-transition-sdk-rust` repo

- No files are modified. It is used as a read-only reference dependency.

## Validation Commands Run

From `prover/`:

```bash
cargo fmt --check
cargo test
cargo run -p bridge-return-host -- check-vectors ../bridge-vectors
cargo run -p bridge-return-host -- emit-b1-token-vector
cargo run -p bridge-return-host -- emit-split-token-vector
cargo run -p bridge-return-host -- emit-b2-token-vector
cargo run -p bridge-return-host -- emit-b1-wire-input
cargo run -p bridge-return-host -- emit-split-wire-input
cargo run -p bridge-return-host -- emit-b2-wire-input
cargo run -p bridge-return-host -- emit-b2-shared-wire-input
cargo run -p bridge-return-host -- precheck-wire "$(cargo run -q -p bridge-return-host -- emit-b2-wire-input)"
cargo check -p bridge-return-host --example cross_check_live
cargo check -p bridge-return-guest --features sp1 --bin bridge-return-sp1-guest
cargo check -p bridge-return-host --features sp1
cargo prove build -p bridge-return-guest --binaries bridge-return-sp1-guest --features sp1 --output-directory target/sp1 --elf-name bridge-return-sp1-guest
cargo run --release -p bridge-return-host --features sp1 -- sp1-vkey target/sp1/bridge-return-sp1-guest
cargo run --release -p bridge-return-host --features sp1 -- \
  sp1-export target/sp1/bridge-return-sp1-guest \
  /tmp/bridge-b1-groth16-real.bin ../bridge-vectors/proof/b1-groth16.json
```

From `state-transition-sdk-rust/`:

```bash
cargo test --no-default-features --features alloc
cargo test
```

All passed on 2026-06-26. The SP1 build produced:

```text
prover/target/sp1/bridge-return-sp1-guest
```

SP1 CPU execution **now completes** once two things are fixed:

1. **Accelerated precompiles** are patched in (`prover/Cargo.toml` `[patch.crates-io]`,
   SP1 6.2.0 patch line): `sha2 0.10.9`, `k256 0.13.4` (+ `crypto-bigint 0.5.5`,
   `signatures` fork), `tiny-keccak 2.0.2`. These route the SHA-256 / secp256k1-verify /
   keccak calls to zkVM syscalls. Off-zkVM the forks fall back to stock impls, so host
   `cargo test` is unaffected.
2. **Always run the host with `--release`** — debug `sp1-sdk` is prohibitively slow.

```bash
cargo run --release -p bridge-return-host --features sp1 -- \
  sp1-execute target/sp1/bridge-return-sp1-guest "$(cat /tmp/bridge-b1-wire.hex)"
```

Result for the B=1 direct fixture: **921,640 cycles**, and `public_values ==
expected_public_values` (relation executes correctly in the zkVM). This was
non-terminating (>15 min) before the precompiles. The RISC-V ELF and the
public-values/digest boundary are validated.

`sp1-mock-groth16` then produced a `groth16`-mode proof artifact whose
`public_values` match and which round-trips through `sp1-proof-info`. A real
`sp1-groth16` (CPU) command was added (`crates/host/src/sp1.rs::real_groth16`).
Real local-CPU Groth16 proving findings (8-yo Intel Mac, 16 GB):

- SP1's Groth16 prover defaults to **Docker**; enable `sp1-sdk`'s **`native-gnark`**
  feature (host Cargo.toml) to build the gnark Go lib via local Go (`libsp1gnark.a`),
  no Docker.
- Use **release circuit mode** (unset `SP1_CIRCUIT_MODE` or set it to `release`).
  SP1 6.3.1 embeds circuit version `v6.1.0` and downloads the pre-generated
  circuit/key archive from
  `https://sp1-circuits.s3-us-east-2.amazonaws.com/v6.1.0-groth16.tar.gz`.
  `SP1_CIRCUIT_MODE=dev` instead targets a private S3 bucket and falls back to a
  local Groth16 setup; the host now rejects that setting before proving.
- The earlier native `len(points) != len(scalars)` failure was caused by a
  **truncated cached `groth16_pk.bin`**, not an incompatibility between native
  gnark and release artifacts. The complete key is 5,862,173,061 bytes with
  SHA-256 `c3760e0e3b58487f8704680d5b3ad32a9fbca9f3cb0749d69055c4f1271ca167`.
  Gnark ignored the `ReadDump` error and failed later during multi-exponentiation,
  which made the symptom misleading. Re-downloading the complete release archive
  fixes it; no local key generation is needed.
- Use single-worker (`SP1_WORKER_NUM_*=1`) to stay under the memory ceiling
  (about 85% peak). A complete B=1 proof succeeded locally in about 52 minutes;
  native gnark took 6m58s after the recursive wrap. The saved proof is 2,014 bytes
  (`SP1ProofWithPublicValues` container), contains a 356-byte Groth16 proof, and
  has SHA-256 `98c6c5d3d9b27aff85ead2bf78543087fe9907f519a66c9d70a731827b2bd0d7`.

The successful no-Docker/no-prover-network/no-GPU command was:

```bash
RUST_LOG=warn,sp1_prover=info,sp1_sdk=info \
SP1_PROVER=cpu SP1_CIRCUIT_MODE=release \
SP1_WORKER_NUM_CORE_WORKERS=1 \
SP1_WORKER_NUM_SETUP_WORKERS=1 \
SP1_WORKER_NUM_SPLICING_WORKERS=1 \
SP1_WORKER_NUM_PREPARE_REDUCE_WORKERS=1 \
SP1_WORKER_NUM_RECURSION_EXECUTOR_WORKERS=1 \
SP1_WORKER_NUM_RECURSION_PROVER_WORKERS=1 \
SP1_WORKER_NUM_DEFERRED_WORKERS=1 \
cargo run --release -p bridge-return-host --features sp1 -- \
  sp1-groth16 target/sp1/bridge-return-sp1-guest \
  "$(cat /tmp/bridge-b1-wire.hex)" /tmp/bridge-b1-groth16-real.bin
```

A `emit-b2-wire-input` command was added; the B=2 batch fixture runs under
`sp1-execute` at **1,796,330 cycles** (~2× B=1's 921,640, linear in batch size)
with `public_values == expected`, validating M4 in the zkVM, not just host
execute mode.

**§11 one-quorum dedup, measured in the zkVM** (`sp1-execute`, B=2, after the
dedup landed): the per-anchor batch (two distinct `UC*`) runs at **1,800,100
cycles**; the shared-anchor batch (`emit-b2-shared-wire-input`, one `UC*`) runs
at **1,678,330 cycles** — a **121,770-cycle (~6.8%) saving**, i.e. exactly one
secp256k1 BFT-quorum verification avoided. Both commit `public_values ==
expected`. The saving scales with batch size: a B-burn batch under one shared
anchor saves `(B-1)` quorum checks.

## Current Scope Limits

- The split-source path is covered by a deterministic synthetic fixture
  (`token/token-01.json`). It uses the SDK's current `SplitMintJustification`
  shape, where the embedded burned source token carries its own certificates.
  A future compact anchored witness type can remove the embedded-cert duplication
  once the SDK exposes anchored proof blobs separately from certified tokens.
- Real SP1 B=1 Groth16 generation and SDK verification are wired and validated.
  The SP1 release verifier key is present in the downloaded v6.1.0 artifacts.
  Project-owned proof/vkey metadata is now **published**
  (`bridge-vectors/proof/b1-groth16.json` via `sp1-export`) **and verified against
  the real SP1 v6.1.0 Groth16 verifier bytecode** (vendored under
  `contracts/tron/contracts/verifier/`; `test/verifier.test.js` confirms the
  published B=1 proof verifies and tampered proof/publicValues/vkey revert).
  **The same verifier is deployed to Nile (`TN4nQmnVz3H3zDnN77NQZTAfBpzkEdoeBR`)
  and verifies the published proof on-chain at ~218k energy within the
  `triggerconstantcontract` dry-run limit** — settling the open M3 risk that
  bn254 Groth16 verification works on Tron (`04-deployment.md` Stage C).
- B>1 batching is validated in **execute mode** for B=2 (`b2_guest.rs`), both
  per-anchor and **shared-anchor** (`build_b2_shared_anchor_fixture` +
  `multi_leaf_paths`), and as a published JSON conformance vector
  (`token/token-02.json`, multi-burn schema) consumed by `check-vectors` and the
  `vectors.rs` integration test. The guest-side **one-quorum dedup** (§11) is
  implemented: the relation verifies each distinct anchor `UC*` once, so a
  shared-anchor batch pays a single BFT-quorum check (measured below).
  **M4 done — a real B=2 Groth16 proof settled a 2-burn batch on Nile in one
  `fulfillBatch`** (vault `TN4n2jy…`, tx `e212bc9e…`, energy 306,473 vs B=1's
  287,126 — the shared anchor amortizes the ~218k proof verification, ~19k
  marginal per extra burn). Tooling: `emit-settlement-b2`, `build_settlement
  _fixture_b2`, `stage-c-settle-b2.js`. Still open: an optional witness-model
  slimming (carry the shared anchor once instead of per burn), and larger
  B (measure `B_max` / per-burn energy + proving cost).
- The B=1/B=2 settlement fixtures use synthetic local certificates and keys
  (deterministic, tailored to the deployed vault for the on-chain settles).
- **Real-data proof (live aggregator): done in CERTIFIED mode.** A real Groth16
  proof of a **real testnet2 token** (the `npm run e2e:back` token, certified by
  the live Unicity validators — `trust_base_hash 0x72a672…`) was generated
  (`sp1-groth16`) and **verifies on the deployed Tron `SP1Verifier`** (energy
  218,165; tamper cases reject). Published as
  `bridge-vectors/proof/live-testnet2-certified.json`; reproduce with
  `scripts/verify-onchain.js <bundle>`.
  - **Anchored mode for a live token is blocked by the aggregator API**
    (`s1::aggregator::fetch_anchored_token` + `examples/s1_anchored_live.rs`):
    `get_inclusion_proof.v2` takes only a `stateId` and serves the *current* root
    (per-transition rounds differ + advance between calls), with no target-root/
    snapshot param, so a multi-transition live token's proofs can't be pinned to
    one shared root. Anchored batching of *live* tokens needs an aggregator
    "inclusion-against-a-specified-root" endpoint; certified mode is the live path
    until then.
  - **Still open:** full on-chain *settlement* of a live token (lock+fulfillBatch)
    needs `e2e:back` reconfigured to a deployed vault (config.vault = A, a real
    Tron recipient, a standard TRC20) so the live token's config matches.
- **Multi-batch (a): replay rejection done; continuity blocked by an accumulator
  bug.**
  - **Replay/double-spend guard verified on-chain.** Re-submitting an
    already-settled batch (the B=2 vault `TN4n2jy…`, `spentRoot` now advanced)
    reverts with **`vault: stale root`** — `spentRootOld(0) != spentRoot`. A
    settled batch cannot be replayed.
  - **Continuity machinery + a real E2-accumulator bug — now FIXED.** Built the
    continued fixture (`build_settlement_fixture_continued` +
    `emit-settlement-continued`): its `spent_root_old` correctly reproduced the
    vault's live `spentRoot` from the prior batch's nullifiers, but the burn's
    non-membership witness **failed its own verifier**. Root cause: the nullifier
    accumulator was a **path-compressed radix tree whose root did not bind prefix
    bits above the top branch**, so a key diverging in that compressed prefix had
    no representable non-membership witness (~21–35% of random ≥2-element trees).
    This was sound only for ≤1-element trees (so M3 B=1 / M4 B=2 were correct) but
    broke **B≥3 and all multi-batch continuity** (effective `B_max = 2`).
  - **Fix:** reworked `sdk-ext/src/accumulator.rs` (and the independent
    `bridge-vectors/gen` reference) into a **depth-256 sparse Merkle tree framed
    from depth 0** (empty-subtree default hashes), so every prefix bit is bound
    and non-membership is provable at any tree size. Correctness suite
    `sdk-ext/tests/accumulator_nonmembership.rs` (now passing, not ignored:
    multi-size self-verify + insert-vs-rebuild agreement); the cross-stack vector
    check passes (gen ↔ sdk-ext agree byte-for-byte); regenerated
    `bridge-vectors/{token,public,accumulator}`. Rebuilt the guest ELF (new vkey
    `0x002b42fa…`) and confirmed **in-circuit**: B=1 and B=2 public values match,
    and the previously-broken **multi-batch / ≥2-element-tree witness now executes
    and matches**. Cost: ~2.6× more cycles (B=1 0.92M→2.40M, B=2 1.80M→4.76M) for
    the 256-deep folds — optimizable later (memoize empty-subtree table per batch,
    compress witnesses). Root hashes changed, so the M3/M4 test vaults' on-chain
    `spentRoot` no longer reproduces — re-deploy throwaway vaults to demo a live
    multi-batch settle.
- **Batch atomicity / blockable recipients (§9): pull-payment mode added.**
  Push-mode `fulfillBatch` transfers to each recipient inline, so one reverting or
  **blocklisted recipient (e.g. USDT) bricks the entire batch** — every other burn
  in it is stuck permanently (a griefing/liveness vector). Added a deploy flag
  `PULL_PAYMENTS` (constructor bool; CLI `TRON_PULL_PAYMENTS=1`): settlement
  credits `owed[recipient]` (no external calls in the batch) and recipients claim
  via `withdraw()` (CEI + `nonReentrant`). A hostile recipient can then revert at
  most its **own** withdrawal, never the batch. Hardhat suite (47 passing) proves
  both: in pull mode a blocklisted recipient settles + the good recipient
  withdraws; in push mode the same batch reverts (`vault: transfer failed`). New
  `MockBlocklistTRC20`. Push remains the default (single-tx UX for plain assets);
  pull is recommended for assets with transfer hooks/blocklists.
- **S2 accumulator-builder (rebuild from chain events).** New `host::s2`
  reconstructs the nullifier accumulator from the vault's `Released` /
  `BatchFulfilled` events and **verifies it against the chain**: it replays each
  batch's nullifiers through the (fixed SMT) accumulator and checks every
  `spentRootOld → spentRootNew` transition, so an out-of-order/incomplete/tampered
  log is rejected before anyone settles on top of it. `next_batch()` then produces
  the `spent_root_old` + per-leaf non-membership witnesses + `spent_root_new` a
  prover needs for the next batch (folded exactly as the guest verifies; rejects
  double-spends). It reuses the one Rust SMT (no third implementation) so a relayer
  can call it rather than re-deriving roots. Tests `tests/s2_rebuild.rs` (5):
  multi-batch rebuild matches the chain (incl. ≥2-element trees), next-batch
  witnesses verify and fold to `spent_root_new`, and tampered-root / out-of-order /
  double-spend logs are rejected. CLI `bridge-return-host s2-rebuild <events.json>`
  (smoke-verified the B=2 vector's root). This is the load-bearing primitive for
  multi-batch operation (S4 relayer / a live multi-batch settle build on it).

## Suggested Next Work

1. **(done)** SP1 v6.1.0 verifier/proof metadata persisted
   (`bridge-vectors/proof/b1-groth16.json`) **and** verified against the real
   `SP1Verifier`/`Groth16Verifier` bytecode the vault calls — vendored under
   `contracts/tron/contracts/verifier/`, `test/verifier.test.js` green (the B=1
   bundle verifies; tampered proof/publicValues/vkey revert). Stage A of
   `04-deployment.md`.
2. Complete the on-chain *settlement* smoke (`01` M3). **Stage B mock deploy is
   DONE on Nile** (`04-deployment.md`): the vault was made **Tron-compatible**
   (constructor now stamps `cfg.vault = address(this)` instead of asserting it —
   Tron's txID-based CREATE address makes the old self-reference circular; EVM
   behavior/Hardhat tests unchanged) and is live with `MockProofVerifier`:
   - `UnicityBridgeVault` = `TNXx9Pv6T8L983y3FM66xBYRip5G4MQH2a`
     (`CONFIG_HASH` verified to bind the deployed address),
   - `MockProofVerifier` = `TBwGYUY9BimAjnaPyFd6YwTit2o2zSRjn9`,
   - deployed via `contracts/tron/scripts/deploy-nile.js stage-b`.
   **Stage B settlement smoke also DONE on Nile** (`scripts/mock-smoke.js`):
   `setTrustBaseAllowed` → `lock` → `fulfillBatch` all SUCCESS, 1 unit released
   via `_safeTransfer` (tx `348e744a…`). It uses a standard `MockTRC20`
   (`TD14oa…`) on a mock-asset vault (`TW9JPc…`) because the user-provided Nile
   "USDT" (`TXYZ…`) is **non-standard** — its `transfer` moves funds but returns
   `false`, which the vault's safe-transfer correctly rejects (real Tether returns
   void, which the vault handles). **Stage C / M3 DONE — a real SP1 Groth16 proof
   settled one burn on Nile end-to-end:** the real verifier (`TN4nQ…`) verifies
   the proof inside the vault's `fulfillBatch`; a deployment-tailored proof
   (config_hash = vault CONFIG_HASH, `spentRootOld=0`, real recipient, lockRef =
   the seeded `lock()`) was generated with `sp1-groth16` and **settled on-chain**
   (vault `TTFpnc8…`, fulfillBatch tx `09d56543…`, ~287k energy, 1e6 units
   released). Tooling: `emit-settlement`, `deploy-nile.js real-vault`,
   `stage-c-settle.js prepare|fulfill` (`04-deployment.md` Stage C). The exit
   criterion "a real proof settles one burn on testnet" is met.
3. **(done)** S1 host witness-package structs + precheck mirroring `GuestInput`
   (`crates/host/src/s1.rs`, `precheck-wire`, `s1_precheck.rs`), **plus
   certified-mode verification of a real live token** (`verify_certified_burn`,
   `cross_check_live`, `s1_live.rs` over a frozen testnet2 sample). **Still open:**
   the live *fetch over the network* — the aggregator `http` client now exists
   (`s1::aggregator`, verified pulling a proof from the live gateway); what
   remains is wiring it into an end-to-end witness builder and reconstructing
   `LockRecord`s from real Tron `Lock` events (needs the deployed vault,
   blocker #2). Today the sample comes from `npm run e2e:back`
   (live aggregator mint/burn, Tron lock mocked) rather than an in-process fetch.
   The earlier **mode gap is closed**: the guest relation now has a *certified*
   mode (`BurnVerification::Certified`), so a real live token runs through the
   relation and the zkVM (see implemented list); anchored mode remains the §11
   batch optimization for when the aggregator serves historical inclusion (§2.1).
4. **(done)** B>1 JSON token vector (`token/token-02.json`, multi-burn schema)
   emitted via `emit-b2-token-vector`; `check_token` now consumes a `burns` array
   and the `vectors.rs` test covers it. (The B=2 execute path was already covered
   by `b2_guest.rs`.)
5. **(done)** Single shared-`UC*` anchor across burns: multi-leaf inclusion-path
   builder (`multi_leaf_paths`, `build_b2_shared_anchor_fixture`) **and** the
   guest-side one-quorum dedup (`validate_bridge_burns` verifies each distinct
   anchor once via `bridge_lock_obligations_for_token_against_root`). zkVM cycle
   saving measured below. **Still open (optional):** witness-model slimming to
   carry the shared anchor once instead of per burn (serialization-size win).

## Dirty Workspace Notes

As of this update, the top-level workspace contains unrelated changes in
`bridge-plugin-tron-usdt/` and a root `bft-trustbase.testnet2.json` file from
other work. The file `prover/crates/host/examples/cross_check_live.rs` was
produced by the substream 01/02 agents and was only adjusted to import the
prover-owned `sdk-ext` crate after SDK changes were moved out. Do not revert or
include unrelated files in prover-service commits unless explicitly coordinating
with the owning agent.
