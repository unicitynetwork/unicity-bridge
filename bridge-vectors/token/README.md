# token/

End-to-end vectors: full burned-token CBOR blobs → the batch relation's public
outputs (nullifier, lock ref, return leaf, value), exercising the whole circuit
path (`00-interop-contract.md` §10, group `token`).

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

The prover host's `check-vectors` command decodes both fixtures into
`GuestInput` values and runs the guest relation in execute mode. Each fixture
also carries `in.guest_wire_input`, the exact byte payload consumed by the
feature-gated SP1 guest binary; `check-vectors` executes that wire payload and
checks the committed `PublicValues` ABI bytes and digest.
