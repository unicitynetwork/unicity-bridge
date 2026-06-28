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

> **Deployed to Nile ✅ (2026-06-28).** Mock verifier + the Tron-compatible vault
> are live on Nile via `scripts/deploy-nile.js stage-b`:
>
> | Contract | Nile address |
> |---|---|
> | `MockProofVerifier` | `TBwGYUY9BimAjnaPyFd6YwTit2o2zSRjn9` |
> | `UnicityBridgeVault` | `TNXx9Pv6T8L983y3FM66xBYRip5G4MQH2a` |
>
> Deployer `TPu3AykWeTSC1hBNnAHvqib7Hu9jbpvjG1` (the account `TRON_SK` controls);
> vault deploy cost ~145 TRX (1.35M energy). The vault's
> `CONFIG_HASH = 0xe06d52d9006479a11680bdc350f0e37c745a2fe752ce9e5dcb23000e06204203`
> was verified off-chain to equal
> `keccak(abi.encode(DOMAIN_CONFIG, …, vault = the deployed address, …))`,
> confirming the **self-stamp** worked.
>
> **The vault was made Tron-compatible.** Originally the constructor required
> `cfg.vault == address(this)`. On EVM you predict the CREATE address from
> `(deployer, nonce)` (independent of constructor args) and pass it; on Tron the
> address is `sha3omit12(txID)` and the txID covers the constructor args, so the
> requirement is circular (CREATE2 doesn't help — `cfg.vault` is in the initcode).
> Fix: the constructor now **stamps `cfg.vault = address(this)`** before hashing,
> so `CONFIG_HASH` binds the deploy address without predicting it (on EVM the
> stamped value equals the CREATE address, so behavior is unchanged — Hardhat
> tests still green). **The off-chain prover/wallet must set
> `BridgeConfig.vault = TNXx9Pv6T8L983y3FM66xBYRip5G4MQH2a`** so its `configHash`
> matches this vault.
>
> Two TronWeb gotchas the script handles: struct constructor args must be
> ABI-encoded via ethers and passed as `rawParameter` (TronWeb mis-encodes
> tuples); and the on-chain contract `name` must be ≤ 32 chars.

Goal: exercise the vault's settlement logic on Nile without a real proof. Done:

1. ✅ `npm run build` in `contracts/tron/` (compile artifacts).
2. ✅ `node scripts/deploy-nile.js stage-b` — deploys `MockProofVerifier` then the
   vault (stamping `address(this)`), prints addresses + `CONFIG_HASH`.
3. ✅ `setTrustBaseAllowed(trustBaseHash, true)` for the testnet2 trust base hash
   `0x72a67260a9ce50ccbd88c889334042bda509115f85ec352a5e50d8bf90c358c0`
   (`emit-trust-base-hash bft-trustbase.testnet2.json`).
4. ✅ `lock()` → fund → `fulfillBatch(publicValues, proof, leaves, lockRefs)` with
   a mock-accepted proof — **succeeded on Nile** (`scripts/mock-smoke.js`):
   lock SUCCESS, fulfillBatch SUCCESS (~70k energy), 1 unit released back to the
   recipient via `_safeTransfer`. The crafted `publicValues` satisfied every
   on-chain check (domain/config/trustbase/spentRoot/returnRoot/lockRefRoot/
   lockDigest/total).

> **fulfillBatch smoke uses a standard `MockTRC20`, not `TRON_USDT`.** The
> user-provided Nile "USDT" (`TXYZopYRdj2D9XRtbG411XZZ3kM5VkAeBf`) is
> **non-standard**: its `transfer` moves funds but returns `false`. The vault's
> safe-transfer requires a returned `true` (or void, as real Tether returns), so
> it correctly rejects that token — a `fulfillBatch` against the real-USDT vault
> reverts `"vault: transfer failed"` at the release (all other checks pass). The
> smoke therefore deploys a conformant `MockTRC20` + a mock-asset vault to prove
> the settlement path:
>
> | Contract (smoke) | Nile address |
> |---|---|
> | `MockTRC20` | `TD14oaT2QX3TYwqFYZ1UGDbLi2EBECsPiH` |
> | `UnicityBridgeVault` (mock asset) | `TW9JPcZcBAVyuUifftWQbEbZ4nRRzgiR3L` |
>
> fulfillBatch tx: `348e744a83f4f51a8a9e275e7e42825d58bc8230daabc168a4100735aa76da34`.
> Run: `node scripts/mock-smoke.js [existing-token]`. For mainnet/production, use a
> real Tether-style (void-returning) USDT, or loosen the vault's `_check` if a
> false-returning token must be supported.
>
> Two more TronWeb gotchas the scripts handle: `createSmartContract` returns the
> tx **directly** while `triggerSmartContract` returns `{transaction}`; and a
> freshly deployed contract must be polled until its code is queryable before it
> can be called.

## Stage C — real proof on Nile (M3)

> **Real verifier proven on Tron ✅ (2026-06-28).** The vendored SP1 v6.1.0
> Groth16 verifier is deployed to Nile and **verifies the published real proof
> on-chain**:
>
> | Contract | Nile address |
> |---|---|
> | `SP1Verifier` (v6.1.0 Groth16) | `TN4nQmnVz3H3zDnN77NQZTAfBpzkEdoeBR` |
>
> Calling `verifyProof(vkey, publicValues, proofBytes)` with
> `bridge-vectors/proof/b1-groth16.json` against the Nile verifier: the **valid
> proof verifies** (`verifyProof` is void, reverts on failure — it returned with
> no revert), at **~218,165 energy**, and the dry-run (`triggerconstantcontract`)
> **succeeds within Tron's limit**. A tampered proof / tampered public values are
> both rejected. This settles the open M3 risk (`01` §"Proof verification"):
> **bn254 Groth16 verification works on Tron's precompiles, cheaply, within the
> ~80 ms dry-run budget** — the Groth16 wrap's single public input keeps it small.
> (Same verifier + same bundle also verify locally in Hardhat — `test/verifier.test.js`.)

**Remaining for a full real-proof settlement (`fulfillBatch` with a real proof):**
the published `b1-groth16.json` was generated from the *synthetic* B=1 fixture,
so its `publicValues` (configHash with `vault=0x…a1`, spentRootOld, leaves,
lockRefs, trustBaseHash) do **not** match a live Nile vault. To settle a real
proof end-to-end you must generate a proof **tailored to the deployment**:

1. ✅ Deploy the real verifier → `TRON_VERIFIER_SP1`.
2. Deploy a vault with the real verifier + the bundle's `vkey`
   (`0x004d10…2133f`); note its address `A`.
3. Build a fixture whose `BridgeConfig.vault = A` and whose other fields match the
   vault's cfg, with `spentRootOld = 0` (the vault's `EMPTY_TREE_ROOT`), a leaf
   `recipient` that is a real Tron address, and a `lockRef` whose digest equals
   what the vault's `lock()` stores; regenerate the Groth16 proof (~50 min CPU,
   `sp1-groth16`).
4. `setTrustBaseAllowed(pv.trustBaseHash)`, seed the matching `lock()`, fund the
   vault with a **standard** TRC20 (not the false-returning Nile USDT — see Stage
   B), then `fulfillBatch(publicValues, proofBytes, leaves, lockRefs)`.

The verifier (the genuinely Tron-risky piece) is proven; step 3's tailored proof
is the remaining compute.

---

## Post-deploy

- Write the deployed addresses back to `.env` (`TRON_VERIFIER`, `TRON_VAULT`) and
  record them with the tx hashes here (a "Deployed addresses" table).
- Cross-check: the vault's on-chain `CONFIG_HASH`/`VKEY`/`DOMAIN_TAG` must equal
  the prover's `config_hash`/bundle `vkey`/`domain_tag`. A mismatch means the
  config froze inconsistently (blocker #3) — do not proceed.
