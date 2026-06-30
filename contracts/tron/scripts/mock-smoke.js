// Full Stage B settlement smoke on Nile with a STANDARD MockTRC20 asset.
//
// The user-provided Nile "USDT" (TXYZ…) is non-standard: its transfer moves
// funds but returns `false`, which the vault's safe-transfer check correctly
// rejects (real Tether returns void, which the vault handles). So the deployed
// production-config vault can't release that token. This smoke deploys a
// conformant MockTRC20 + a mock-asset vault and proves lock -> fulfillBatch ->
// release end-to-end on real Tron. Run from contracts/tron:
//   node scripts/mock-smoke.js
const fs = require("fs");
const path = require("path");
const crypto = require("crypto");
const ethers = require("ethers");
const TronWebLib = require("tronweb");
const TronWeb = TronWebLib.TronWeb || TronWebLib.default || TronWebLib;

const TRUST_BASE_HASH = "0x72a67260a9ce50ccbd88c889334042bda509115f85ec352a5e50d8bf90c358c0";
const LOCK_DOMAIN = "0x158b847f78b3910a5f5f42820de61abba1bf5ae1fbb29dabfba09118f393f932";
const NULLIFIER_DOMAIN = "0xd4530e4ea58fc8e38f84506e62b421476c3eeec70f4cbebefc32688a510e2d5d";
const REASON_TAG = 39048;
const AMOUNT = 1_000_000n;

function loadEnv() {
  const env = {};
  for (const line of fs.readFileSync(path.join(__dirname, "..", "..", "..", ".env"), "utf8").split("\n")) {
    const m = line.match(/^\s*([A-Za-z0-9_]+)\s*=\s*(.*)\s*$/);
    if (m && !line.trim().startsWith("#")) env[m[1]] = m[2];
  }
  return env;
}
const abiBc = (p) => {
  const j = JSON.parse(fs.readFileSync(path.join(__dirname, "..", "artifacts", "contracts", p), "utf8"));
  return { abi: j.abi, bytecode: j.bytecode.replace(/^0x/, "") };
};
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const sha = (s) => "0x" + crypto.createHash("sha256").update(Buffer.from(s, "utf8")).digest("hex");

async function main() {
  const env = loadEnv();
  const base = (env.TRON_RPC_URL || "https://nile.trongrid.io").replace(/\/$/, "");
  const tw = new TronWeb({ fullHost: base, privateKey: env.TRON_SK });
  const me = tw.defaultAddress.base58;
  const meHex = tw.address.toHex(me);
  const meEvm = "0x" + meHex.slice(2);
  const toEvm = (a) => "0x" + tw.address.toHex(a).slice(2);
  const coder = ethers.AbiCoder.defaultAbiCoder();
  const norm = (x) => (typeof x === "string" ? (x.startsWith("0x") ? x : "0x" + x) : "0x" + BigInt(x).toString(16).padStart(64, "0"));
  const receipt = async (id) => {
    for (let i = 0; i < 16; i++) {
      await sleep(3000);
      const r = await fetch(base + "/wallet/gettransactioninfobyid", { method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify({ value: id }) });
      const j = await r.json();
      if (j && j.receipt) return j;
    }
    return null;
  };

  console.log(`deployer ${me}  (${(await tw.trx.getBalance(me)) / 1e6} TRX)`);

  // 1. MockTRC20 (standard: transfer returns true) + mint to deployer.
  const trc = abiBc("test/MockTRC20.sol/MockTRC20.json");
  let tokenBase58 = process.argv[2]; // optional: reuse an already-deployed+minted token
  if (!tokenBase58) {
    const deployed = await tw.contract().new({ abi: trc.abi, bytecode: trc.bytecode, feeLimit: 1_500_000_000, parameters: [] });
    tokenBase58 = tw.address.fromHex(deployed.address);
    console.log("MockTRC20 deployed:", tokenBase58);
    for (let i = 0; i < 16; i++) {
      await sleep(3000);
      try {
        if ((await tw.trx.getContract(deployed.address)).bytecode) break;
      } catch (_) {}
    }
    const t = tw.contract(trc.abi, deployed.address);
    await t.mint(me, (AMOUNT * 10n).toString()).send({ feeLimit: 200_000_000 });
    await sleep(5000);
  } else {
    console.log("MockTRC20 (reused):", tokenBase58);
  }
  const token = tw.contract(trc.abi, tw.address.toHex(tokenBase58));

  // 2. Vault with asset = MockTRC20, reusing the deployed mock verifier.
  const verifier = env.TRON_VERIFIER;
  const assetEvmHex = tw.address.toHex(tokenBase58).slice(2).toLowerCase();
  const cfg = [3448148188, meEvm, toEvm(tokenBase58), sha(`unicity-bridge:tron:3448148188:${assetEvmHex}`), sha(`unicity-bridge-coin:tron:3448148188:${assetEvmHex}`), REASON_TAG, LOCK_DOMAIN, NULLIFIER_DOMAIN];
  const vaultArt = abiBc("UnicityBridgeVault.sol/UnicityBridgeVault.json");
  const rawCtor = new ethers.Interface(vaultArt.abi).encodeDeploy([cfg, toEvm(verifier), "0x" + "00".repeat(32), meEvm]).slice(2);
  // createSmartContract returns the tx object directly (unlike triggerSmartContract).
  const built = await tw.transactionBuilder.createSmartContract({ abi: vaultArt.abi, bytecode: vaultArt.bytecode, rawParameter: rawCtor, feeLimit: 1_900_000_000, name: "UnicityBridgeVaultMock" }, meHex);
  const dep = await tw.trx.sendRawTransaction(await tw.trx.sign(built));
  const vaultHex = built.contract_address;
  const vaultBase58 = tw.address.fromHex(vaultHex);
  console.log("Vault(mock asset):", vaultBase58, "deploy", (await receipt(dep.txid || built.txID))?.receipt?.result);
  const v = tw.contract(vaultArt.abi, vaultHex);

  // 3. allow trust base, approve, lock.
  console.log("setTrustBaseAllowed:", (await receipt(await v.setTrustBaseAllowed(TRUST_BASE_HASH, true).send({ feeLimit: 200_000_000 })))?.receipt?.result);
  await token.approve(vaultBase58, AMOUNT.toString()).send({ feeLimit: 200_000_000 });
  await sleep(4000);
  const nonce = BigInt((await v.nextNonce().call()).toString());
  const tokenId = ethers.keccak256(ethers.toUtf8Bytes("mock-token-" + nonce));
  const recipientCommitment = ethers.keccak256(ethers.toUtf8Bytes("mock-recipient"));
  console.log("lock:", (await receipt(await v.lock(AMOUNT.toString(), tokenId, recipientCommitment).send({ feeLimit: 300_000_000 })))?.receipt?.result, "nonce", nonce);

  // 4. fulfillBatch one leaf back to the deployer.
  const digest = norm(await v.lockDigest(nonce.toString()).call());
  const spentRoot = norm(await v.spentRoot().call());
  const domainTag = norm(await v.DOMAIN_TAG().call());
  const configHash = norm(await v.CONFIG_HASH().call());
  const nullifier = ethers.keccak256(ethers.toUtf8Bytes("mock-nullifier-" + nonce));
  const leaf = [nullifier, meEvm, AMOUNT, "0x" + "00".repeat(20), 0n, 0n];
  const lockRef = [nonce, digest];
  const returnRoot = ethers.keccak256(coder.encode(["bytes32", "address", "uint256", "address", "uint256", "uint64"], leaf));
  const lockRefRoot = ethers.keccak256(coder.encode(["uint256", "bytes32"], lockRef));
  const pv = [domainTag, configHash, TRUST_BASE_HASH, spentRoot, ethers.keccak256(ethers.toUtf8Bytes("new")), returnRoot, lockRefRoot, 1, AMOUNT];
  const publicValues = coder.encode(["tuple(bytes32,bytes32,bytes32,bytes32,bytes32,bytes32,bytes32,uint32,uint256)"], [pv]);
  const funcSig = "fulfillBatch(bytes,bytes,(bytes32,address,uint256,address,uint256,uint64)[],(uint256,bytes32)[])";
  const rawParameter = coder.encode(["bytes", "bytes", "tuple(bytes32,address,uint256,address,uint256,uint64)[]", "tuple(uint256,bytes32)[]"], [publicValues, "0x", [leaf], [lockRef]]).slice(2);

  const balBefore = BigInt((await token.balanceOf(me).call()).toString());
  const ftx = await tw.transactionBuilder.triggerSmartContract(vaultHex, funcSig, { feeLimit: 500_000_000, rawParameter }, [], meHex);
  const fres = await tw.trx.sendRawTransaction(await tw.trx.sign(ftx.transaction));
  const fid = fres.txid || ftx.transaction.txID;
  const finfo = await receipt(fid);
  console.log(`fulfillBatch tx ${fid} -> ${finfo?.receipt?.result} (energy ${finfo?.receipt?.energy_usage_total})`);
  if (finfo?.receipt?.result !== "SUCCESS") {
    console.log("  resMessage:", finfo?.resMessage ? Buffer.from(finfo.resMessage, "hex").toString() : JSON.stringify(fres));
    process.exit(1);
  }
  const balAfter = BigInt((await token.balanceOf(me).call()).toString());
  console.log(`\n✅ fulfillBatch SUCCESS — released ${balAfter - balBefore} units back to deployer (vault drained the locked amount).`);
  console.log(`   vault ${vaultBase58}  token ${tokenBase58}`);
  console.log(`   explorer: https://nile.tronscan.org/#/transaction/${fid}`);
}

main().catch((e) => {
  console.error(e.message || e);
  process.exit(1);
});
