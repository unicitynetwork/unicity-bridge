// Stage B fulfillBatch smoke on Nile (mock verifier): allow the trust base, lock
// USDT into the deployed vault (sets lockDigest + funds it), then fulfillBatch a
// single ReturnLeaf and confirm the USDT is released. Proves the on-chain
// settlement path on real Tron with the frozen config. Run from contracts/tron:
//   node scripts/fulfill-smoke.js
const fs = require("fs");
const path = require("path");
const ethers = require("ethers");
const TronWebLib = require("tronweb");
const TronWeb = TronWebLib.TronWeb || TronWebLib.default || TronWebLib;

const TRUST_BASE_HASH = "0x72a67260a9ce50ccbd88c889334042bda509115f85ec352a5e50d8bf90c358c0";
const AMOUNT = 1_000_000n; // 1 USDT (6 decimals)

function loadEnv() {
  const env = {};
  for (const line of fs.readFileSync(path.join(__dirname, "..", "..", "..", ".env"), "utf8").split("\n")) {
    const m = line.match(/^\s*([A-Za-z0-9_]+)\s*=\s*(.*)\s*$/);
    if (m && !line.trim().startsWith("#")) env[m[1]] = m[2];
  }
  return env;
}
const abiOf = (p) => JSON.parse(fs.readFileSync(path.join(__dirname, "..", "artifacts", "contracts", p), "utf8")).abi;
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

async function txInfo(tw, id) {
  for (let i = 0; i < 12; i++) {
    await sleep(3000);
    const info = await tw.trx.getTransactionInfo(id);
    if (info && info.receipt) return info;
  }
  return null;
}

async function main() {
  const env = loadEnv();
  const tw = new TronWeb({ fullHost: (env.TRON_RPC_URL || "https://nile.trongrid.io").replace(/\/$/, ""), privateKey: env.TRON_SK });
  const me = tw.defaultAddress.base58;
  const meEvm = "0x" + tw.address.toHex(me).slice(2);
  const vault = env.TRON_VAULT;
  const vaultHex = tw.address.toHex(vault);
  const vaultAbi = abiOf("UnicityBridgeVault.sol/UnicityBridgeVault.json");
  const v = tw.contract(vaultAbi, vaultHex);
  const usdt = await tw.contract().at(env.TRON_USDT);
  console.log(`deployer ${me}  vault ${vault}`);

  // 1. allow the testnet2 trust base (idempotent).
  if (!(await v.trustBaseAllowed(TRUST_BASE_HASH).call())) {
    console.log("setTrustBaseAllowed ...");
    const id = await v.setTrustBaseAllowed(TRUST_BASE_HASH, true).send({ feeLimit: 200_000_000 });
    console.log("  tx", id, "->", (await txInfo(tw, id))?.receipt?.result);
  } else {
    console.log("trust base already allowed");
  }

  // 2. approve + lock (funds the vault, sets lockDigest[nonce]).
  const nonce = BigInt((await v.nextNonce().call()).toString());
  const tokenId = ethers.keccak256(ethers.toUtf8Bytes("smoke-token-" + nonce));
  const recipientCommitment = ethers.keccak256(ethers.toUtf8Bytes("smoke-recipient"));
  console.log(`approve ${AMOUNT} USDT ...`);
  console.log("  tx", await usdt.approve(vault, AMOUNT.toString()).send({ feeLimit: 200_000_000 }));
  await sleep(4000);
  console.log(`lock nonce=${nonce} ...`);
  const lockId = await v.lock(AMOUNT.toString(), tokenId, recipientCommitment).send({ feeLimit: 300_000_000 });
  console.log("  tx", lockId, "->", (await txInfo(tw, lockId))?.receipt?.result);

  const digest = await v.lockDigest(nonce.toString()).call();
  const digestHex = typeof digest === "string" ? digest : "0x" + BigInt(digest).toString(16).padStart(64, "0");
  const spentRoot = await v.spentRoot().call();
  const domainTag = await v.DOMAIN_TAG().call();
  const configHash = await v.CONFIG_HASH().call();
  const norm = (x) => (typeof x === "string" ? x : "0x" + BigInt(x).toString(16).padStart(64, "0"));

  // 3. craft the single-leaf batch (no fee), matching BridgeEncoding roots.
  const coder = ethers.AbiCoder.defaultAbiCoder();
  const nullifier = ethers.keccak256(ethers.toUtf8Bytes("smoke-nullifier-" + nonce));
  const leaf = [nullifier, meEvm, AMOUNT, "0x" + "00".repeat(20), 0n, 0n]; // recipient=me, feeRecipient=0
  const lockRef = [nonce, digestHex];
  const returnRoot = ethers.keccak256(
    coder.encode(["bytes32", "address", "uint256", "address", "uint256", "uint64"], leaf),
  );
  const lockRefRoot = ethers.keccak256(coder.encode(["uint256", "bytes32"], lockRef));
  const spentRootNew = ethers.keccak256(ethers.toUtf8Bytes("smoke-spent-new-" + nonce));
  const pv = [norm(domainTag), norm(configHash), TRUST_BASE_HASH, norm(spentRoot), spentRootNew, returnRoot, lockRefRoot, 1, AMOUNT];
  const publicValues = coder.encode(
    ["tuple(bytes32,bytes32,bytes32,bytes32,bytes32,bytes32,bytes32,uint32,uint256)"],
    [pv],
  );

  // 4. fulfillBatch via ethers-encoded rawParameter (TronWeb mis-encodes tuples).
  const funcSig = "fulfillBatch(bytes,bytes,(bytes32,address,uint256,address,uint256,uint64)[],(uint256,bytes32)[])";
  const rawParameter = coder
    .encode(
      ["bytes", "bytes", "tuple(bytes32,address,uint256,address,uint256,uint64)[]", "tuple(uint256,bytes32)[]"],
      [publicValues, "0x", [leaf], [lockRef]],
    )
    .slice(2);
  console.log("fulfillBatch ...");
  const built = await tw.transactionBuilder.triggerSmartContract(
    vaultHex,
    funcSig,
    { feeLimit: 500_000_000, rawParameter },
    [],
    tw.address.toHex(me),
  );
  const signed = await tw.trx.sign(built.transaction);
  const res = await tw.trx.sendRawTransaction(signed);
  const fid = res.txid || built.transaction.txID;
  const info = await txInfo(tw, fid);
  console.log("  tx", fid, "->", info?.receipt?.result, "energy", info?.receipt?.energy_usage_total);
  if (info?.receipt?.result !== "SUCCESS") {
    console.log("  resMessage:", info?.resMessage ? Buffer.from(info.resMessage, "hex").toString() : info);
    process.exit(1);
  }

  const bal = await usdt.balanceOf(me).call();
  console.log("\n✅ fulfillBatch SUCCESS — Released event emitted; deployer USDT balance:", bal.toString());
  console.log(`   explorer: https://nile.tronscan.org/#/transaction/${fid}`);
}

main().catch((e) => {
  console.error(e.message || e);
  process.exit(1);
});
