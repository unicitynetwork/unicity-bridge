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

The prover host's `check-vectors` command decodes this fixture into a
`GuestInput` and runs the guest relation in execute mode.
