# 04 — Nile deployment procedure

How to deploy the source-chain contracts (`contracts/tron/`) to the **Tron Nile
testnet** and wire them to the prover's real proof. This is the runbook for the
on-chain half of M3 (`01` M3, `03-status.md` next-work #1/#2).

> **Not yet executed.** As of this doc the contracts are not deployed, on purpose:
> - The **real SP1 v6.1.0 Groth16 verifier** Solidity contract is not in the repo
>   yet (only `IProofVerifier.sol` + `test/MockProofVerifier.sol`). That is the
>   first prerequisite — see Stage A. *(blocker #1)*
> - The vault's `CONFIG_HASH` must bind a **real, non-synthetic** `BridgeConfig`;
>   the current proof bundle commits a synthetic config. *(blocker #3)*
>
> Run the stages below in order once each prerequisite lands. Deploying is an
> outward-facing, hard-to-reverse action — get explicit go-ahead before Stage B/C.

---

## Prerequisites

From `.env` (already populated except the two deploy outputs):

| Var | Use |
|---|---|
| `TRON_CHAIN_ID=3448148188` | Nile network id (`BridgeConfig.sourceChainId`) |
| `TRON_RPC_URL=https://nile.trongrid.io/` | TronWeb fullhost / TronGrid HTTP |
| `TRON_GRPC=grpc.nile.trongrid.io:50051` | gRPC endpoint (tronbox/`trident` if used) |
| `TRON_SK` | funded deployer private key |
| `TRON_ACCOUNT=TBAubr…` | deployer address (admin) |
| `TRON_USDT=TXYZopYRdj2D9XRtbG411XZZ3kM5VkAeBf` | bridged asset (`BridgeConfig.asset`) |
| `TRON_VERIFIER=` | **filled by Stage A/C** (deployed verifier address) |
| `TRON_VAULT=` | **filled by Stage C** (deployed vault address) |

Also needed:
- A Nile-funded `TRON_ACCOUNT` (TRX for energy/bandwidth — use the Nile faucet).
- The proof bundle `bridge-vectors/proof/b1-groth16.json` (`vkey`, `publicValues`,
  `proofBytes`) for the verifier smoke.
- Node (TronWeb) for deployment. **Hardhat here is EVM-only** (it compiles/tests
  the same Solidity but cannot deploy to Tron) — see the tooling note.

### Tooling note (Tron vs EVM)

`contracts/tron/hardhat.config.js` targets the EVM for compile + unit tests only.
TVM deployment uses **TronWeb** (or `tronbox`/`trident`). The Solidity is solc
`0.8.24`; bytecode/ABI from Hardhat's `artifacts/` are reused — only the deploy
transport differs. The procedure below uses a small TronWeb script
(`contracts/tron/scripts/deploy-nile.js`, to be added) that reads `.env`.

---

## Address/config derivation (read first)

The vault is **self-referential**: `BridgeConfig.vault` must equal the deployed
vault address, and `CONFIG_HASH = keccak(abi.encode(cfg))` is recomputed on-chain
(`BridgeEncoding.configHash`). Two consequences:

1. **Predict the vault address before constructing `cfg`.** Deploy via `CREATE2`
   with a fixed salt, compute the deterministic address, set `cfg.vault` to it,
   then deploy. (Or deploy the verifier first, predict the vault address, build
   `cfg`, deploy the vault to the predicted address.)
2. The `BridgeConfig` fields must match **byte-for-byte** what the circuit commits
   (`00-interop-contract.md` §2). Fields (`contracts/tron/contracts/BridgeEncoding.sol`):

   ```
   sourceChainId   = TRON_CHAIN_ID (3448148188)
   vault           = <predicted vault address>
   asset           = TRON_USDT (as 20-byte EVM-form hex)
   tokenType       = deriveTokenType(chainId, asset)   // plugin src/derivations
   coinId          = deriveCoinId(chainId, asset)
   reasonTag       = BridgeBackReason CBOR tag (00 §4)
   lockDomain      = K(domain string)                  // 00 §1
   nullifierDomain = K(domain string)
   ```

   The TS plugin already derives `tokenType`/`coinId` from `(chainId, asset)`
   (`bridge-plugin-tron-usdt` `deriveTokenType`/`deriveCoinId`); use the same
   values in the Rust `BridgeConfig` so `config_hash` matches across all three
   stacks. **This shared config is blocker #3** — freeze it before Stage C.

The vault constructor:

```solidity
constructor(BridgeConfig cfg, IProofVerifier verifier_, bytes32 vkey, address admin_)
```

- `verifier_` = `TRON_VERIFIER`
- `vkey` = the `vkey` from `b1-groth16.json` (`0x004d100af488ce9a36e6e44a71b8dced18aa6a55cf3634151ac7b5609302133f` for the current B=1 circuit; regenerate per circuit version with `sp1-vkey`)
- `admin_` = `TRON_ACCOUNT`

`fulfillBatch(publicValues, proof, leaves, lockRefs)` then requires, in order:
`verifyProof` passes → `domainTag == DOMAIN_TAG` → `configHash == CONFIG_HASH` →
`trustBaseAllowed[trustBaseHash]` → `spentRootOld == spentRoot` → return/lock-ref
roots match → each `lockDigest[nonce]` bound (set by a prior `lock()`).

---

## Stage A — verifier smoke (local, no Tron) — blocker #1

Goal: prove the published bundle verifies against the **same** SP1 verifier
bytecode the vault will call, before spending anything on Nile.

1. Generate SP1's `v6.1.0` Groth16 verifier Solidity (`SP1Verifier.sol` +
   `Groth16Verifier.sol`) and add it under `contracts/tron/contracts/verifier/`.
   Confirm its `ISP1Verifier`/`verifyProof` shape matches `IProofVerifier`.
2. Hardhat test (`contracts/tron/test/`): deploy the verifier, call
   `verifyProof(vkey, publicValues, proof)` with the three fields from
   `bridge-vectors/proof/b1-groth16.json`. **Expect: no revert.**
3. Negative: flip one byte of `proof` / `publicValues` → expect revert.

When green, the verifier is ready to deploy.

## Stage B — mock end-to-end on Nile (M2)

Goal: exercise the vault's settlement logic on Nile without a real proof.

1. `npm run build` in `contracts/tron/` (compile artifacts).
2. Deploy `test/MockProofVerifier.sol` → temporary `TRON_VERIFIER`.
3. Predict the vault address (CREATE2), build `cfg` (Nile + `TRON_USDT` + derived
   fields), deploy `UnicityBridgeVault(cfg, mockVerifier, vkey, admin)`.
4. `setTrustBaseAllowed(trustBaseHash, true)` for the testnet2 trust base hash
   (`canonical_hash` of `bft-trustbase.testnet2.json`).
5. Seed a `lock()` so `lockDigest[nonce]` is set, fund the vault with `TRON_USDT`,
   then `fulfillBatch(publicValues, proof=<any>, leaves, lockRefs)` with a
   mock-accepted proof. Confirm `Released` events + TRC20 transfer.

## Stage C — real proof on Nile (M3) — needs blockers #1, #3, #4

1. Deploy the **real** verifier from Stage A → `TRON_VERIFIER`.
2. Freeze the real `BridgeConfig` (shared with circuit + wallet) and regenerate a
   **real** proof from live witness data (S1 fetch, blocker #4) so its
   `configHash`/`trustBaseHash` match the deployed vault.
3. Deploy the vault as in Stage B (real verifier, real `vkey`), allow the trust
   base, perform a real bridge-in `lock()`, then `fulfillBatch` with the real
   `b1-groth16.json` triple. Confirm settlement on Nile.
4. **Measure energy** for `verifyProof` + `fulfillBatch` and check the
   `triggerconstantcontract` ~80 ms dry-run limit (Tron #6288, `01` §"Proof
   verification"). The Groth16 wrap exposes one public input, which keeps the
   verifier small — confirm empirically.

---

## Post-deploy

- Write the deployed addresses back to `.env` (`TRON_VERIFIER`, `TRON_VAULT`) and
  record them with the tx hashes here (a "Deployed addresses" table).
- Cross-check: the vault's on-chain `CONFIG_HASH`/`VKEY`/`DOMAIN_TAG` must equal
  the prover's `config_hash`/bundle `vkey`/`domain_tag`. A mismatch means the
  config froze inconsistently (blocker #3) — do not proceed.
