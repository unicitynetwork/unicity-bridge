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

## Stage A — verifier smoke (local, no Tron) — blocker #1 ✅ DONE

Goal: prove the published bundle verifies against the **same** SP1 verifier
bytecode the vault will call, before spending anything on Nile.

**Result:** done. The SP1 `v6.1.0` Groth16 verifier is vendored under
`contracts/tron/contracts/verifier/` (from the locally downloaded circuit
artifacts `~/.sp1/circuits/groth16/v6.1.0/`, byte-for-byte):
- `verifier/v6.1.0/Groth16Verifier.sol` — gnark-generated `Verifier` (embeds the
  v6.1.0 vk; pins `pragma solidity 0.8.20`);
- `verifier/v6.1.0/SP1VerifierGroth16.sol` — the `SP1Verifier` wrapper exposing
  `verifyProof(bytes32 programVKey, bytes publicValues, bytes proofBytes)` (the
  exact `IProofVerifier` shape; `VERSION()==v6.1.0`, `VERIFIER_HASH()` begins
  `0x4388a21c` = the bundle's proof selector);
- `verifier/ISP1Verifier.sol` — the interface it imports.

`hardhat.config.js` now lists both `0.8.24` (bridge contracts) and `0.8.20`
(verifier subtree, pinned via `overrides`); the two never link (the vault depends
only on `IProofVerifier`).

`test/verifier.test.js` deploys `SP1Verifier` and calls
`verifyProof(vkey, publicValues, proofBytes)` with the three fields from
`bridge-vectors/proof/b1-groth16.json`: **the real B=1 proof verifies**, and a
flipped proof / publicValues / vkey each revert. Run:

```bash
cd contracts/tron && npx hardhat test test/verifier.test.js
```

This `SP1Verifier` contract is the one to deploy in Stage C (`TRON_VERIFIER`).

## Stage B — mock end-to-end on Nile (M2)

> **Tooling ready, two live blockers found (2026-06-28).** The TronWeb deploy
> script `scripts/deploy-nile.js` is in place and connects to Nile (reads
> balance, builds deploy txs). Attempting the deploy surfaced two blockers:
>
> 1. **Credential mismatch in `.env`.** `TRON_SK` derives
>    `TPu3AykWeTSC1hBNnAHvqib7Hu9jbpvjG1` (0 TRX, **unactivated**), but the funded
>    account is `TRON_ACCOUNT = TBAubrN14Zm3mbWACjPMte9HeqQgJ1cDxQ` (2000 TRX).
>    The key does not control the funded account, so nothing can be deployed.
>    **Fix:** put the funded account's private key in `TRON_SK`, **or** fund
>    `TPu3Ayk…` via the Nile faucet (https://nileex.io/join/getJoinPage). The
>    deploy attempt failed pre-broadcast (`account does not exist`) — no TRX spent.
> 2. **Vault self-reference is not Tron-deployable.** `UnicityBridgeVault`'s
>    constructor requires `cfg.vault == address(this)`. On EVM you predict the
>    CREATE address from `(deployer, nonce)` — independent of constructor args —
>    set `cfg.vault`, and deploy (this is how the Hardhat tests do it). On Tron
>    the new contract address is `sha3omit12(txID)` and the **txID covers the
>    constructor args**, so the address depends on `cfg.vault` which must equal the
>    address: a circular dependency with no fixed point (CREATE2 doesn't help —
>    `cfg.vault` is in the initcode too). **Fix (01 track):** a Tron-compatible
>    vault that sets `vault = address(this)` internally (drop the `cfg.vault`
>    constructor arg, fold `address(this)` into `CONFIG_HASH`) or uses a one-time
>    initializer. Until then only the no-arg contracts (mock/real verifier) deploy.

Goal: exercise the vault's settlement logic on Nile without a real proof.

1. `npm run build` in `contracts/tron/` (compile artifacts).
2. Deploy `test/MockProofVerifier.sol` → temporary `TRON_VERIFIER`
   (`node scripts/deploy-nile.js mock-verifier`, once a funded key is set).
3. **(blocked, see above)** Predict the vault address, build `cfg` (Nile +
   `TRON_USDT` + derived fields), deploy
   `UnicityBridgeVault(cfg, mockVerifier, vkey, admin)`.
4. `setTrustBaseAllowed(trustBaseHash, true)` for the testnet2 trust base hash
   (`canonical_hash` of `bft-trustbase.testnet2.json`).
5. Seed a `lock()` so `lockDigest[nonce]` is set, fund the vault with `TRON_USDT`,
   then `fulfillBatch(publicValues, proof=<any>, leaves, lockRefs)` with a
   mock-accepted proof. Confirm `Released` events + TRC20 transfer.

## Stage C — real proof on Nile (M3) — needs blockers #1, #3, #4

> **Mode gap (blocker #4).** A live aggregator token is *certified* (each
> transition carries its own `UnicityCertificate`); the S1 host path already
> verifies these (`s1::verify_certified_burn`, validated on a real testnet2
> sample). But the zk **guest** relation proves *anchored* mode (one shared
> `UC*`). Producing a real **proof** therefore needs either (a) a guest certified
> path, or (b) the aggregator serving historical inclusion proofs against one
> anchor root (ZK_BACK3 §2.1) so S1 can assemble an anchored `WitnessPackage`.
> The host-side verification + derivations are done; the zk side of a *live* token
> is the remaining work before Stage C can run.


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
