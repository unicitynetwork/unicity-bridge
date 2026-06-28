// Stage C real-proof settlement on Nile, in two phases:
//   node scripts/stage-c-settle.js prepare            # allow trust base + lock; verify lockDigest
//   node scripts/stage-c-settle.js fulfill <bundle>   # fulfillBatch with the real proof bundle
//
// Reads /tmp/stagec-settle.json (emit-settlement output: leaf, lock_ref, lock_seed,
// public_values, trust_base_hash) and .env (TRON_VAULT, TRON_USDT asset, TRON_SK).
const fs = require("fs");
const path = require("path");
const ethers = require("ethers");
const TronWebLib = require("tronweb");
const TronWeb = TronWebLib.TronWeb || TronWebLib.default || TronWebLib;

function loadEnv() {
  const env = {};
  for (const line of fs.readFileSync(path.join(__dirname, "..", "..", "..", ".env"), "utf8").split("\n")) {
    const m = line.match(/^\s*([A-Za-z0-9_]+)\s*=\s*(.*)\s*$/);
    if (m && !line.trim().startsWith("#")) env[m[1]] = m[2];
  }
  return env;
}
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const SETTLE = JSON.parse(fs.readFileSync("/tmp/stagec-settle.json", "utf8"));

async function main() {
  const env = loadEnv();
  const base = (env.TRON_RPC_URL || "https://nile.trongrid.io").replace(/\/$/, "");
  const tw = new TronWeb({ fullHost: base, privateKey: env.TRON_SK });
  const me = tw.defaultAddress.base58;
  const meHex = tw.address.toHex(me);
  const vaultHex = tw.address.toHex(env.TRON_VAULT);
  const vaultAbi = JSON.parse(fs.readFileSync(path.join(__dirname, "..", "artifacts/contracts/UnicityBridgeVault.sol/UnicityBridgeVault.json"), "utf8")).abi;
  const v = tw.contract(vaultAbi, vaultHex);
  // The asset under custody is whatever the vault was deployed with.
  let assetAddr = await v.assetAddr().call();
  if (typeof assetAddr !== "string") assetAddr = tw.address.fromHex(assetAddr);
  else if (assetAddr.startsWith("0x")) assetAddr = tw.address.fromHex("41" + assetAddr.slice(2));
  else if (/^41[0-9a-fA-F]{40}$/.test(assetAddr)) assetAddr = tw.address.fromHex(assetAddr);
  const asset = tw.contract(JSON.parse(fs.readFileSync(path.join(__dirname, "..", "artifacts/contracts/test/MockTRC20.sol/MockTRC20.json"), "utf8")).abi, tw.address.toHex(assetAddr));
  console.log("asset:", assetAddr);
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

  const cmd = process.argv[2];
  console.log(`vault ${env.TRON_VAULT}  deployer ${me}`);

  if (cmd === "prepare") {
    const tb = SETTLE.public_values.trust_base_hash;
    if (!(await v.trustBaseAllowed(tb).call())) {
      console.log("setTrustBaseAllowed:", (await receipt(await v.setTrustBaseAllowed(tb, true).send({ feeLimit: 200_000_000 })))?.receipt?.result);
    } else console.log("trust base already allowed");

    const seed = SETTLE.lock_seed;
    console.log("approve:", (await receipt(await asset.approve(env.TRON_VAULT, String(seed.amount)).send({ feeLimit: 200_000_000 })))?.receipt?.result);
    const nonce = BigInt((await v.nextNonce().call()).toString());
    if (nonce !== BigInt(seed.nonce)) throw new Error(`vault nextNonce ${nonce} != fixture nonce ${seed.nonce}; use a fresh vault`);
    console.log("lock:", (await receipt(await v.lock(String(seed.amount), seed.unicity_token_id, seed.recipient_commitment).send({ feeLimit: 300_000_000 })))?.receipt?.result);

    const onchain = norm(await v.lockDigest(String(seed.nonce)).call());
    console.log("lockDigest[%s] on-chain: %s", seed.nonce, onchain);
    console.log("fixture   lockRef.digest: %s", SETTLE.lock_ref.digest);
    console.log(onchain.toLowerCase() === SETTLE.lock_ref.digest.toLowerCase() ? "✅ lockDigest MATCHES — proof will settle" : "❌ MISMATCH — kill the prove");
    return;
  }

  if (cmd === "fulfill") {
    const bundle = JSON.parse(fs.readFileSync(process.argv[3], "utf8"));
    const coder = ethers.AbiCoder.defaultAbiCoder();
    const L = SETTLE.leaf;
    const leaf = [L.nullifier, L.recipient, BigInt(L.amount), L.fee_recipient, BigInt(L.fee_amount), BigInt(L.deadline)];
    const lockRef = [BigInt(SETTLE.lock_ref.nonce), SETTLE.lock_ref.digest];
    const funcSig = "fulfillBatch(bytes,bytes,(bytes32,address,uint256,address,uint256,uint64)[],(uint256,bytes32)[])";
    const rawParameter = coder.encode(["bytes", "bytes", "tuple(bytes32,address,uint256,address,uint256,uint64)[]", "tuple(uint256,bytes32)[]"], [bundle.public_values, bundle.proof_bytes, [leaf], [lockRef]]).slice(2);
    const balBefore = BigInt((await asset.balanceOf(meHex).call()).toString());
    const built = await tw.transactionBuilder.triggerSmartContract(vaultHex, funcSig, { feeLimit: 1_000_000_000, rawParameter }, [], meHex);
    const res = await tw.trx.sendRawTransaction(await tw.trx.sign(built.transaction));
    const id = res.txid || built.transaction.txID;
    const info = await receipt(id);
    console.log(`fulfillBatch tx ${id} -> ${info?.receipt?.result} (energy ${info?.receipt?.energy_usage_total})`);
    if (info?.receipt?.result !== "SUCCESS") {
      console.log("  resMessage:", info?.resMessage ? Buffer.from(info.resMessage, "hex").toString() : JSON.stringify(res));
      process.exit(1);
    }
    const balAfter = BigInt((await asset.balanceOf(meHex).call()).toString());
    console.log(`\n🎉 REAL-PROOF fulfillBatch SUCCESS — released ${balAfter - balBefore} units to ${L.recipient}`);
    console.log(`   verified the SP1 Groth16 proof on-chain + settled. tx: https://nile.tronscan.org/#/transaction/${id}`);
    return;
  }
  throw new Error("usage: prepare | fulfill <bundle.json>");
}

main().catch((e) => {
  console.error(e.message || e);
  process.exit(1);
});
