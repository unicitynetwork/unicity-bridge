# 09 — Ethereum Sepolia deployment

Why, what was checked, and how the vault is deployed on Ethereum Sepolia
against Circle's USDC. The Tron material (`contracts/tron`, `deployments/nile`,
the Nile scripts and docs) stays in place; nothing here replaces it.

## The asset: USDC

Tether issues no Sepolia token; the "USDT" contracts on Sepolia explorers are
third-party deployments. Circle issues USDC on Sepolia at
`0x1c7D4B196Cb0C7B01d743Fbc6116a902379C7238` (name and symbol `USDC`, 6
decimals, FiatToken version 2 with the blocklist, read on chain 2026-09-23),
and its faucet at faucet.circle.com pays 20 USDC per address every two hours.
Because the blocklist can make a recipient's transfer revert, the vault is
deployed in pull-payment mode (`ETH_PULL_PAYMENTS=1`); recipients claim
with `withdraw()`. The mainnet asset is a separate decision: Ethereum has both
Tether's USDT and Circle's USDC natively, and the vault takes either.

## Why Ethereum

Tron bounds a transaction by CPU time as well as energy. Mainnet and Shasta
have always allowed 80 ms; Nile allowed 160 ms until proposal 20699 lowered it
to 80 ms on 2026-09-08. The SP1 Groth16 verification needs more than 80 ms on
java-tron's pure-Java bn128 pairing precompile, so no batch can settle on any
Tron network (OPERATIONS.md §10). The fix on Tron's side, a native pairing
implementation, was closed unmerged in May 2026 (java-tron PR 5507) and its
replacement (issue 6374) has no release or date.

The EVM bounds execution by gas only. The vault is plain Solidity, compiled and
unit-tested under Hardhat's EVM since the start, and Succinct deploys the same
SP1 v6.1.0 Groth16 verifier behind a gateway on Ethereum, Sepolia, Arbitrum One,
Base, Optimism and BNB Chain. So the vault deploys unchanged, with the gateway
as its `IProofVerifier`.

## Check 1 — the published proof verifies on Sepolia (2026-09-23)

`node scripts/verify-onchain-eth.js` (from `contracts/tron`) calls
`verifyProof(vkey, publicValues, proofBytes)` with
`protocol/vectors/proof/b1-groth16.json` as a free `eth_call`:

| Target on Sepolia | Result | Gas | Round trip |
|---|---|---:|---:|
| SP1 gateway `0x397A5f7f3dBd538f23DE225B51f532c34448dA9B` | verified | 267,443 | 104 ms |
| v6.1.0 verifier `0xb69f2584CBcFf99a58C4e7002E8b89Af54a6f4e2` | verified | 256,710 | 69 ms |

A flipped proof byte and a flipped public-values byte are both rejected on both
targets. The round trip includes the network; Sepolia's block gas limit is 60M
and there is no time bound. On Nile the same call cost 218,165 energy in July and
times out since September.

Addresses come from `sp1-contracts/contracts/deployments/11155111.json`. The
gateway routes on the first four bytes of the proof (`0x4388a21c` for v6.1.0),
so a proof from our v6.3.1 prover, which the July Nile settlement verified with
the vendored v6.1.0 contract, takes the same route.

## Check 2 — full procedure rehearsed on a local Hardhat node (2026-09-23)

`scripts/deploy-eth.js` was run against `npx hardhat node` with the
vendored verifier standing in for the gateway (chain 31337, Hardhat account 0):

| Step | Gas | Note |
|---|---:|---|
| `verifier` (vendored SP1Verifier, rehearsal only) | 1,543,132 | Sepolia uses the gateway instead |
| `mock-asset` (MockTRC20, 6 decimals, open mint) | 479,716 | rehearsal only; Sepolia uses Circle's USDC |
| `vault <asset> <vkey>` | 1,470,460 | Nile v2 cost 1,348,988 energy |
| `allow-trust-base <vault> <hash>` | 47,825 | testnet2 hash `0x72a67260…` |
| `lock-smoke`: approve | 44,123 | |
| `lock-smoke`: lock | 126,692 | Nile: 125,734 energy |

`freeze <vault>` printed the deployment record and
`bridge-return-host emit-config` reproduced its `config_hash`, `token_type` and
`coin_id` byte for byte (the same binary reproduces the Nile v2 hash
`0xfa77a13a…`).

`fulfill-probe` calls `fulfillBatch` with the published bundle as a free call.
Against a vault whose `VKEY` is the current guest key it reverts inside the
verifier, because the bundle was proven for another key. Against a rehearsal
vault deployed with the bundle's own key it reverts with `vault: bad config`:
the proof verified inside `fulfillBatch` and the next check, the config hash,
stopped it, as it must for a bundle proven for another vault.

## Procedure

From `contracts/tron`, after `npm run build`. The repo-root `.env` holds the
`ETH_*` variables listed in `.env.example`; `process.env` overrides it.

1. `node scripts/verify-onchain-eth.js` must print two `VERIFIED` lines.
2. Request 20 USDC for the deployer at faucet.circle.com (`ETH_ASSET` is
   Circle's contract).
3. `node scripts/deploy-eth.js vault $ETH_ASSET <vkey>` with the vkey
   from `sp1-vkey.json` (the key of the guest the service proves with; the
   same key the Nile v2 vault carries). Put the address in `ETH_VAULT`.
4. `node scripts/deploy-eth.js allow-trust-base <vault> $(bridge-return-host emit-trust-base-hash bft-trustbase.testnet2.json)`.
5. `node scripts/deploy-eth.js lock-smoke <vault> $ETH_ASSET` for one
   Lock event and a stored `lockDigest`; it approves and locks 1 USDC the
   deployer already holds.
6. `node scripts/deploy-eth.js freeze <vault> > deployments/sepolia/sepolia-usdc.json`,
   then feed its `config` fields to `bridge-return-host emit-config` and check
   `config_hash` matches the on-chain `CONFIG_HASH`.

The deployer key was generated on 2026-09-23 into the gitignored `.env`
(`ETH_SK`); its address is `0x2B00d708fc777F174A248B9bE01c8E8379d69Caf`.
It is the vault admin. The steps above need roughly 1.8M gas, so 0.05 Sepolia
ETH covers them with room for a settlement.

## What is unchanged and what is not

Unchanged: `UnicityBridgeVault.sol`, `BridgeEncoding.sol`, the guest program
and its key, the trust base, the protocol vectors. The token transfer helpers
already require only no-revert, which covers Ethereum USDT's void return; the
explicit gas stipend on the transfer call is harmless on the EVM.

The token type and coin id derivations now take the chain family, the CAIP-2
namespace (`unicity-bridge:<family>:<chainId>:<asset>`, interop §2): `tron`
for the Nile deployments, whose values are unchanged, and `eip155` here. The
guest reads both values from the config as opaque bytes, so the label is a
host, plugin and script matter with no new key. The first Sepolia vault
(`0x5153EE37…`) was deployed under the inherited `tron` label and is written
off with the 1 USDC it holds; the deployment records carry `chain_family`.

Still Tron-only and needed before a Sepolia burn can settle end to end:
`relayer.js` (TronWeb transport, TronGrid events, fee limit), the service's
`TRON_GRID_URL` and events command, and the plugin's RPC client, address
handling, signer, providers and explorer links. The lock-event decoder and the
mint-justification verifier already work on keccak topics and 20-byte
addresses. `bridge-core` and the wallet's bridge module were built for a second
chain family; the manifest union gets an `eip155` variant and the wallet gets a
second asset folder.

## The return service on an Ethereum-family chain

The service reaches its chain through three shell commands (events, submit,
simulate). `contracts/tron/scripts/relayer-eth.js` implements them with
ethers, reading `ETH_RPC_URL`, `ETH_SK`, `ETH_VAULT` and
`ETH_VAULT_DEPLOY_BLOCK` from `.env`, next to the Tron `relayer.js`; the
container and `run-return-service.sh` pick one by `BRIDGE_RELAYER`.
`events` scans `BatchFulfilled` and `Released` from the deploy block in
10,000-block windows and prints the S2 log with the live `spentRoot`; `scan`
rebuilds the accumulator with the host binary and compares; `settle --stdin`
submits `fulfillBatch` and prints the txid, with a `require` string or the
verifier's custom error name on stderr so the service's `stale root` rebase
keys on the same text as on Tron; `simulate --stdin` static-calls each leaf's
transfer from the vault.

`test/relayer-eth.test.js` runs the relayer against the vault bytecode on the
in-process network: a mocked-proof batch settles, the log reproduces it, a
second batch on the consumed root is refused with `vault: stale root`, and the
simulation names a leaf whose payout exceeds the balance. Against the live
vault on 2026-09-23, `events` reported no batches and root zero, `scan`
reported synced, and `simulate` rejected an oversize leaf with USDC's
`ERC20: transfer amount exceeds balance`.

`docker-compose.yml` defaults to the Sepolia deployment and `relayer-eth.js`;
the Tron variables stay for the disabled Nile deployment.

## The wallet meanwhile

The Nile manifest carries a `disabledReason` (a new optional field on
`BridgeManifestBase`). The wallet still lists Tron and its USDT, shows the
reason on the asset row, and refuses to start a lock or a burn for it: a burn
of a Tron-backed token could not settle and would strand the funds. Tokens
already held keep their verifier and stay visible. The row becomes active
again by removing the field from the manifest.

The plugin is `@unicitylabs/bridge-plugin` since 2026-09-23 (it was
`bridge-plugin-tron-usdt`): one package, a `tron` and an `eip155` family
behind five neutral seams (`SourceChainRpc`, `ConstantCaller`, `SourceSigner`,
the `BridgeManifest` union, `ChainFamilyAdapter`), one lock justification, one
verifier, one source adapter and one manifest loader. The Ethereum family adds
a JSON-RPC client, an EIP-1193 (MetaMask) signer that switches the wallet to
the bridge's chain, Etherscan links and `0x` address validation. The wallet
lists the Sepolia USDC asset from `assets/evm-usdc` next to the disabled Tron
one, both through the shared assets-out side in `assets/out.ts`.

## Live deployment (2026-09-23, v2)

Deployed from `0x2B00d708fc777F174A248B9bE01c8E8379d69Caf` after it received
0.05 Sepolia ETH. The record is `deployments/sepolia/sepolia-usdc.json`. A
first vault the same day (`0x5153EE37…`, tx `0xfa154a7a…`, block 11764606)
used the `tron` derivation label and was superseded within the hour.

| Item | Sepolia |
|---|---|
| `UnicityBridgeVault` | `0x9C2BF4Ed5b85130fFD14BE8FA65c60F299Fc9a2E` |
| deploy tx, block | `0x32567ca425a698218de02907096f8f699ac39ad9ed36a4e882bcc238b93a12f2`, 11764672 |
| deploy gas | 1,470,460 |
| verifier | SP1 gateway `0x397A5f7f3dBd538f23DE225B51f532c34448dA9B` (v6.1.0 behind it: `0xb69f2584CBcFf99a58C4e7002E8b89Af54a6f4e2`) |
| asset | Circle USDC `0x1c7D4B196Cb0C7B01d743Fbc6116a902379C7238`, pull payments |
| vkey | `0x0039a5424014e57caf45d3451053e6c014547837ae09c9eb724aa569389b90d5` |
| `CONFIG_HASH` | `0x4058e87dff330d9c6925af755e0732dbc80be85643fdce46c8315eae51c9f7e8` |
| token type | `0x2ccbf3157add2b9a2dcc10e772abf5cf328e2723f9f290a9d2b6c4a42a132d6c` |
| coin id | `0xeae954053183b9d1836d6b5c892867014bcc1571fcc6813f5b56b16a78d0497f` |
| trust base allowed | `0x72a67260…` in tx `0x533f90406fb1015764d3edaddfafe91e2956455f3d0e344f929f3fffc27f794c`, 47,825 gas |
| admin | the deployer |

`bridge-return-host emit-config` reproduces the on-chain `CONFIG_HASH`, token
type, coin id and domain tag from the frozen fields.

The free `fulfill-probe` against the live vault reverts with `ProofInvalid()`:
the call went from the vault through the gateway to the v6.1.0 verifier, which
rejected the published bundle because it was proven for another key. That is
the expected outcome and shows the settlement path reaches the verifier with no
time limit in the way.

Lock smoke with 1 USDC from the Circle faucet:

| Step | Tx | Gas |
|---|---|---:|
| approve | `0xb0a9b66cc225ab9a1868f10e500b96e2c1f44aace40c3e3f4c8eee698ae8893a` | 55,437 |
| lock, nonce 0 | `0x9dd8fd6344b15ecc0161279923dcbc57933f1ea549eef995d7bbd08b5504982c` | 139,668 |

`lockDigest[0]` is `0xfcce28c867c8ff4ba609c8c858fbf4115c4d1384cc1ffb8d6cd00bc377afe1ad`
and the vault holds 1 USDC. USDC's `transferFrom` costs more than the mock's,
hence the higher lock gas than in the rehearsal. The deployer keeps about
0.046 Sepolia ETH and 18 USDC.
