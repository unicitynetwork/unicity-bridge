// Re-run only fulfillBatch against an already-locked nonce (default 0), with
// reliable receipt polling. Assumes the trust base is allowed and lockDigest[nonce]
// is set (see fulfill-smoke.js / earlier lock). Run from contracts/tron:
//   node scripts/fulfill-run.js [nonce]
const fs = require("fs");
const path = require("path");
const ethers = require("ethers");
const TronWebLib = require("tronweb");
const TronWeb = TronWebLib.TronWeb || TronWebLib.default || TronWebLib;

const TRUST_BASE_HASH = "0x72a67260a9ce50ccbd88c889334042bda509115f85ec352a5e50d8bf90c358c0";
const AMOUNT = 1_000_000n;

function loadEnv() {
  const env = {};
  for (const line of fs.readFileSync(path.join(__dirname, "..", "..", "..", ".env"), "utf8").split("\n")) {
    const m = line.match(/^\s*([A-Za-z0-9_]+)\s*=\s*(.*)\s*$/);
    if (m && !line.trim().startsWith("#")) env[m[1]] = m[2];
  }
  return env;
}
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

async function receipt(base, id) {
  for (let i = 0; i < 16; i++) {
    await sleep(3000);
    const r = await fetch(base + "/wallet/gettransactioninfobyid", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ value: id }),
    });
    const j = await r.json();
    if (j && j.receipt) return j;
  }
  return null;
}

async function main() {
  const env = loadEnv();
  const base = (env.TRON_RPC_URL || "https://nile.trongrid.io").replace(/\/$/, "");
  const tw = new TronWeb({ fullHost: base, privateKey: env.TRON_SK });
  const me = tw.defaultAddress.base58;
  const meEvm = "0x" + tw.address.toHex(me).slice(2);
  const vaultHex = tw.address.toHex(env.TRON_VAULT);
  const abi = JSON.parse(fs.readFileSync(path.join(__dirname, "..", "artifacts/contracts/UnicityBridgeVault.sol/UnicityBridgeVault.json"), "utf8")).abi;
  const v = tw.contract(abi, vaultHex);
  const usdt = await tw.contract().at(env.TRON_USDT);
  const nonce = BigInt(process.argv[2] || "0");
  const norm = (x) => (typeof x === "string" ? (x.startsWith("0x") ? x : "0x" + x) : "0x" + BigInt(x).toString(16).padStart(64, "0"));

  const digest = norm(await v.lockDigest(nonce.toString()).call());
  const spentRoot = norm(await v.spentRoot().call());
  const domainTag = norm(await v.DOMAIN_TAG().call());
  const configHash = norm(await v.CONFIG_HASH().call());
  console.log(`nonce ${nonce} digest ${digest}\nspentRoot ${spentRoot}`);

  const coder = ethers.AbiCoder.defaultAbiCoder();
  const nullifier = ethers.keccak256(ethers.toUtf8Bytes("smoke-nullifier-" + nonce));
  const leaf = [nullifier, meEvm, AMOUNT, "0x" + "00".repeat(20), 0n, 0n];
  const lockRef = [nonce, digest];
  const returnRoot = ethers.keccak256(coder.encode(["bytes32", "address", "uint256", "address", "uint256", "uint64"], leaf));
  const lockRefRoot = ethers.keccak256(coder.encode(["uint256", "bytes32"], lockRef));
  const spentRootNew = ethers.keccak256(ethers.toUtf8Bytes("smoke-spent-new-" + nonce));
  const pv = [domainTag, configHash, TRUST_BASE_HASH, spentRoot, spentRootNew, returnRoot, lockRefRoot, 1, AMOUNT];
  const publicValues = coder.encode(["tuple(bytes32,bytes32,bytes32,bytes32,bytes32,bytes32,bytes32,uint32,uint256)"], [pv]);
  const funcSig = "fulfillBatch(bytes,bytes,(bytes32,address,uint256,address,uint256,uint64)[],(uint256,bytes32)[])";
  const rawParameter = coder
    .encode(["bytes", "bytes", "tuple(bytes32,address,uint256,address,uint256,uint64)[]", "tuple(uint256,bytes32)[]"], [publicValues, "0x", [leaf], [lockRef]])
    .slice(2);

  const balBefore = BigInt((await usdt.balanceOf(me).call()).toString());
  console.log("fulfillBatch ...");
  const built = await tw.transactionBuilder.triggerSmartContract(vaultHex, funcSig, { feeLimit: 500_000_000, rawParameter }, [], tw.address.toHex(me));
  const res = await tw.trx.sendRawTransaction(await tw.trx.sign(built.transaction));
  const id = res.txid || built.transaction.txID;
  const info = await receipt(base, id);
  const result = info?.receipt?.result;
  console.log(`  tx ${id} -> ${result} (energy ${info?.receipt?.energy_usage_total})`);
  if (result !== "SUCCESS") {
    console.log("  resMessage:", info?.resMessage ? Buffer.from(info.resMessage, "hex").toString() : JSON.stringify(res));
    process.exit(1);
  }
  const balAfter = BigInt((await usdt.balanceOf(me).call()).toString());
  console.log(`\n✅ fulfillBatch SUCCESS — released ${balAfter - balBefore} USDT to ${me}`);
  console.log(`   Released event emitted; explorer: https://nile.tronscan.org/#/transaction/${id}`);
}

main().catch((e) => {
  console.error(e.message || e);
  process.exit(1);
});
