# token/ — STUB (filled at M2)

End-to-end vectors: full burned-token CBOR blobs → the batch relation's public
outputs (nullifier, lock ref, return leaf, value), exercising the whole circuit
path (`00-interop-contract.md` §10, group `token`).

Not yet generated because it requires the new SDK pieces in
`docs/bridge/dev-plan/03-prover-service.md`: anchored-mode inclusion (**E1**),
the structural backing verifier (**E3**), and a `BridgeBackReason`-bearing burn
built through the mint/transfer/split/burn flow. These reuse the existing
Rust↔JS cross-SDK fixture machinery (`state-transition-sdk-rust` →
"Regenerating cross-SDK fixtures").

Planned per file: a `*.cbor` burned-token blob plus a `*.json` of the expected
relation outputs for that token under a fixed `config` + anchor certificate.
