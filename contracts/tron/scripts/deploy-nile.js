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

const TronWebLib = require("tronweb");
const TronWeb = TronWebLib.TronWeb || TronWebLib.default || TronWebLib;

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
  const addr = env.TRON_ACCOUNT;
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
  } else {
    console.error(`unknown command: ${cmd}`);
    process.exit(1);
  }
}

main().catch((e) => {
  console.error(e.message || e);
  process.exit(1);
});
