// Deploy bridge contracts to Tron Nile (04-deployment.md). Reads the repo-root
// .env for TRON_RPC_URL / TRON_SK / TRON_ACCOUNT and the bridged asset/config.
//
// Usage (from contracts/tron, after `npm run build` to produce artifacts/):
//   node scripts/deploy-nile.js mock-verifier        # deploy MockProofVerifier
//   node scripts/deploy-nile.js verifier             # deploy SP1Verifier (v6.1.0)
//   node scripts/deploy-nile.js balance              # print deployer balance
//
// NOTE on the vault: UnicityBridgeVault's constructor requires
//   cfg.vault == address(this)
// On EVM you predict the CREATE address from (deployer, nonce), set cfg.vault to
// it, and deploy. On Tron the new contract address derives from the deployment
// *txID* (sha3omit12(txID)), and the txID covers the constructor args — so the
// address depends on cfg.vault, which must equal the address: a circular
// dependency with no fixed point. The vault therefore CANNOT be deployed to Tron
// as-is; it needs a Tron-compatible variant (drop the cfg.vault arg and use
// `address(this)`, or set it via a one-time initializer). Tracked in
// 04-deployment.md. This script deploys the no-arg contracts only.

const fs = require("fs");
const path = require("path");
const crypto = require("crypto");

const TronWebLib = require("tronweb");
const TronWeb = TronWebLib.TronWeb || TronWebLib.default || TronWebLib;
const ethers = require("ethers");

// Canonical bridge config derivations (must match Rust bridge-return-core and the
// TS plugin): token_type / coin_id are SHA-256 over a domain-tagged string of the
// decimal chain id and the lowercase 20-byte EVM-form asset hex (00 §1/§2).
function sha256Hex(s) {
  return "0x" + crypto.createHash("sha256").update(Buffer.from(s, "utf8")).digest("hex");
}
function deriveTokenType(chainIdStr, assetEvmHex) {
  return sha256Hex(`unicity-bridge:tron:${chainIdStr}:${assetEvmHex}`);
}
function deriveCoinId(chainIdStr, assetEvmHex) {
  return sha256Hex(`unicity-bridge-coin:tron:${chainIdStr}:${assetEvmHex}`);
}

// Canonical domain separators (00 §1; same values as bridge-vectors/config-00).
const LOCK_DOMAIN = "0x158b847f78b3910a5f5f42820de61abba1bf5ae1fbb29dabfba09118f393f932";
const NULLIFIER_DOMAIN = "0xd4530e4ea58fc8e38f84506e62b421476c3eeec70f4cbebefc32688a510e2d5d";
const REASON_TAG = 39048;
const VKEY_PLACEHOLDER = "0x" + "00".repeat(32); // M2 mock: vkey unused by the mock verifier

function loadEnv() {
  const envPath = path.join(__dirname, "..", "..", "..", ".env");
  const env = {};
  for (const line of fs.readFileSync(envPath, "utf8").split("\n")) {
    const m = line.match(/^\s*([A-Za-z0-9_]+)\s*=\s*(.*)\s*$/);
    if (m && !line.trim().startsWith("#")) env[m[1]] = m[2];
  }
  return env;
}

function artifact(name, file) {
  const p = path.join(__dirname, "..", "artifacts", "contracts", file);
  const j = JSON.parse(fs.readFileSync(p, "utf8"));
  return { abi: j.abi, bytecode: j.bytecode };
}

// Tron base58 / 41-hex address -> EVM 0x-20-byte form (for ABI encoding).
function toEvm(tronWeb, addr) {
  return "0x" + tronWeb.address.toHex(addr).slice(2);
}

// Deploy a contract with constructor args. TronWeb's contract().new mis-encodes
// struct (tuple) params, so we ABI-encode the constructor with ethers and pass
// `rawParameter` to createSmartContract.
async function deployWithCtor(tronWeb, ownerHex, name, art, ctorValues) {
  console.log(`Deploying ${name} ...`);
  const iface = new ethers.Interface(art.abi);
  const rawParameter = iface.encodeDeploy(ctorValues).slice(2);
  const tx = await tronWeb.transactionBuilder.createSmartContract(
    {
      abi: art.abi,
      bytecode: art.bytecode.replace(/^0x/, ""),
      rawParameter,
      feeLimit: 1_900_000_000, // <= account balance; the actual energy is what's charged
      callValue: 0,
      name: name.slice(0, 32), // Tron limit: contract name <= 32 chars
    },
    ownerHex,
  );
  const signed = await tronWeb.trx.sign(tx);
  const res = await tronWeb.trx.sendRawTransaction(signed);
  if (!res.result) throw new Error(`broadcast rejected: ${JSON.stringify(res)}`);
  const hexAddr = tx.contract_address;
  const base58 = tronWeb.address.fromHex(hexAddr);
  console.log(`  ${name} deployed at: ${base58}  (tx ${res.txid || res.transaction?.txID})`);
  return { base58, hexAddr };
}

// Deploy the Tron-compatible vault against a given verifier (base58).
async function deployVault(tronWeb, env, addr, verifierBase58, opts = {}) {
  const chainId = Number(env.TRON_CHAIN_ID || 3448148188);
  const assetBase58 = opts.assetBase58 || env.TRON_USDT;
  const vkey = opts.vkey || VKEY_PLACEHOLDER;
  // Pull-payment mode: settle by crediting owed[] (recipients withdraw), so a
  // blocklisted/reverting recipient can't brick the batch (§9). Opt in per deploy
  // via opts.pullPayments or TRON_PULL_PAYMENTS=1 (recommended for USDT).
  const pullPayments = (opts.pullPayments ?? env.TRON_PULL_PAYMENTS === "1") === true;
  const assetEvmHex = tronWeb.address.toHex(assetBase58).slice(2).toLowerCase(); // drop 41 prefix
  const ownerHex = tronWeb.address.toHex(addr);
  const cfg = [
    chainId, // sourceChainId
    toEvm(tronWeb, addr), // vault: IGNORED (stamped to address(this)); any valid addr
    toEvm(tronWeb, assetBase58), // asset
    deriveTokenType(String(chainId), assetEvmHex), // tokenType
    deriveCoinId(String(chainId), assetEvmHex), // coinId
    REASON_TAG,
    LOCK_DOMAIN,
    NULLIFIER_DOMAIN,
  ];
  const vaultArt = artifact("UnicityBridgeVault", "UnicityBridgeVault.sol/UnicityBridgeVault.json");
  console.log(
    `(cfg.vault stamped to address(this); asset ${assetBase58}, vkey ${vkey.slice(0, 12)}…, ` +
      `${pullPayments ? "PULL" : "push"}-payment)`,
  );
  const { base58: vaultBase58, hexAddr } = await deployWithCtor(
    tronWeb,
    ownerHex,
    "UnicityBridgeVault",
    vaultArt,
    [cfg, toEvm(tronWeb, verifierBase58), vkey, toEvm(tronWeb, addr), pullPayments],
  );

  // Poll for the deployed code, then read CONFIG_HASH to confirm the stamp.
  const vaultContract = tronWeb.contract(vaultArt.abi, hexAddr);
  let configHash = null;
  for (let i = 0; i < 12 && configHash === null; i++) {
    await new Promise((r) => setTimeout(r, 3000));
    try {
      configHash = await vaultContract.CONFIG_HASH().call();
    } catch (_) {
      /* not yet confirmed */
    }
  }
  console.log(`  CONFIG_HASH: ${configHash}`);
  console.log(`  tokenType:   ${cfg[3]}`);
  console.log(`  coinId:      ${cfg[4]}`);
  console.log("\nSet in .env:");
  console.log(`  TRON_VERIFIER=${verifierBase58}`);
  console.log(`  TRON_VAULT=${vaultBase58}`);
  return vaultBase58;
}

async function deployNoArg(tronWeb, label, art) {
  console.log(`Deploying ${label} ...`);
  const instance = await tronWeb.contract().new({
    abi: art.abi,
    bytecode: art.bytecode.replace(/^0x/, ""),
    feeLimit: 5_000_000_000, // 5000 TRX cap
    callValue: 0,
    parameters: [],
  });
  const base58 = tronWeb.address.fromHex(instance.address);
  console.log(`  ${label} deployed at: ${base58}  (hex ${instance.address})`);
  return base58;
}

async function main() {
  const env = loadEnv();
  const base = (env.TRON_RPC_URL || "https://nile.trongrid.io").replace(/\/$/, "");
  const tronWeb = new TronWeb({
    fullHost: base,
    privateKey: env.TRON_SK,
    headers: env.TRON_API_KEY ? { "TRON-PRO-API-KEY": env.TRON_API_KEY } : {},
  });

  const cmd = process.argv[2] || "balance";
  // The deployer is the account the private key controls (TronWeb sets
  // defaultAddress from TRON_SK) — not necessarily env.TRON_ACCOUNT.
  const addr = tronWeb.defaultAddress.base58;
  if (env.TRON_ACCOUNT && env.TRON_ACCOUNT !== addr) {
    console.log(`note: TRON_SK controls ${addr}, but TRON_ACCOUNT=${env.TRON_ACCOUNT} (using the key's address)`);
  }
  const bal = await tronWeb.trx.getBalance(addr);
  console.log(`Deployer ${addr}: ${bal / 1e6} TRX on ${base}`);

  if (cmd === "balance") return;

  if (cmd === "mock-verifier") {
    await deployNoArg(
      tronWeb,
      "MockProofVerifier",
      artifact("MockProofVerifier", "test/MockProofVerifier.sol/MockProofVerifier.json"),
    );
  } else if (cmd === "verifier") {
    await deployNoArg(
      tronWeb,
      "SP1Verifier",
      artifact("SP1Verifier", "verifier/v6.1.0/SP1VerifierGroth16.sol/SP1Verifier.json"),
    );
  } else if (cmd === "stage-b") {
    // Mock-verifier Stage B: deploy MockProofVerifier + the Tron-compatible vault.
    const verifierAddr = await deployNoArg(
      tronWeb,
      "MockProofVerifier",
      artifact("MockProofVerifier", "test/MockProofVerifier.sol/MockProofVerifier.json"),
    );
    await deployVault(tronWeb, env, addr, verifierAddr);
  } else if (cmd === "vault-only") {
    // Retry just the vault against an already-deployed verifier (argv[3]).
    const verifierAddr = process.argv[3];
    if (!verifierAddr) throw new Error("usage: vault-only <verifier base58 address>");
    await deployVault(tronWeb, env, addr, verifierAddr);
  } else if (cmd === "real-vault") {
    // Stage C vault: real SP1Verifier + bundle vkey + a chosen asset (argv[3]).
    const asset = process.argv[3];
    // Default = current guest ELF vkey (sp1-vkey target/sp1/bridge-return-sp1-guest);
    // pass argv[4] to override. MUST match the ELF the proof is generated with.
    const vkey = process.argv[4] || "0x00d75299dfc01ff06af28435bb830f6b477eb8d4eb88b760e4daee04b496b000";
    if (!env.TRON_VERIFIER_SP1 || !asset)
      throw new Error("usage: real-vault <asset base58>  (needs TRON_VERIFIER_SP1 in .env)");
    await deployVault(tronWeb, env, addr, env.TRON_VERIFIER_SP1, { assetBase58: asset, vkey });
  } else {
    console.error(`unknown command: ${cmd}`);
    process.exit(1);
  }
}

main().catch((e) => {
  console.error(e.message || e);
  process.exit(1);
});
