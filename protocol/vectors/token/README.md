# token/

End-to-end vectors: full burned-token CBOR blobs → the batch relation's public
outputs (nullifier, lock ref, return leaf, value), exercising the whole circuit
path (`00-interop-contract.md` §10, group `token`).

All three fixtures are `BRIDGE_PROTO_VERSION = 2`: state-transition SDK 3 wire
formats (mint and transfer transactions v2 with a deadline slot, inclusion
proofs carrying the round's reference time, the shard-aware inclusion
certificate) and the wallet's value payload (`tag(39050) [1, assets, null]`,
`00-interop-contract.md` §2.1) in every genesis. They are emitted by the prover
host (`emit-b1-token-vector`, `emit-split-token-vector`, `emit-b2-token-vector`)
from `crates/host/src/fixture.rs`; regenerate them there, never by hand.

`token-00.json` is the M2 direct bridge-lock B=1 fixture:

- one bridge-lock justified payment token using `config/config-00.json`;
- one terminal burn to `BurnPredicate(SHA256(BridgeBackReason))`;
- one shared anchored inclusion root for genesis + burn;
- one nullifier accumulator witness from the empty root; and
- the exact `PublicValues` ABI bytes the prover commits.

`token-01.json` is the M2 split-source B=1 fixture:

- one original bridge-lock justified source token;
- one SDK split burn of that source token;
- one split output token burned to `BurnPredicate(SHA256(BridgeBackReason))`;
- recursive extraction of the original source lock obligation from the split
  mint justification;
- one top-level anchored inclusion root for the returned split output's mint and
  burn; and
- embedded source-token certificates inside the split mint justification, matching
  the SDK's current certified-token wire format.

`token-02.json` is the B=2 multi-burn fixture: two independent direct
bridge-lock tokens burned to distinct reasons under one trust base, one ordered
accumulator transition over both nullifiers, two lock refs sorted by nonce.

The prover host's `check-vectors` command decodes all three fixtures into
`GuestInput` values and runs the guest relation in execute mode. Each fixture
also carries `in.guest_wire_input`, the exact byte payload consumed by the
feature-gated SP1 guest binary; `check-vectors` executes that wire payload and
checks the committed `PublicValues` ABI bytes and digest.
