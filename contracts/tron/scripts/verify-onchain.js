// Stage C (M3): verify the published real Groth16 proof against the SP1Verifier
// deployed on Nile (TRON_VERIFIER_SP1). `verifyProof` is `view` and reverts on
// failure, so a clean (no-revert) constant call == verified. Also reports the
// energy and checks that a tampered proof is rejected. Run from contracts/tron:
//   node scripts/verify-onchain.js
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

async function main() {
  const env = loadEnv();
  const base = (env.TRON_RPC_URL || "https://nile.trongrid.io").replace(/\/$/, "");
  const tw = new TronWeb({ fullHost: base, privateKey: env.TRON_SK });
  const owner = tw.address.toHex(tw.defaultAddress.base58);
  const verifier = env.TRON_VERIFIER_SP1;
  if (!verifier) throw new Error("TRON_VERIFIER_SP1 not set");
  const verifierHex = tw.address.toHex(verifier);
  // Default bundle = the published B=1; pass a path (argv[2]) to verify another.
  const bundlePath = process.argv[2] || path.join(__dirname, "..", "..", "..", "protocol/vectors/proof/b1-groth16.json");
  const b = JSON.parse(fs.readFileSync(bundlePath, "utf8"));
  console.log("bundle:", bundlePath);
  const coder = ethers.AbiCoder.defaultAbiCoder();

  async function call(publicValues, proofBytes) {
    const parameter = coder.encode(["bytes32", "bytes", "bytes"], [b.vkey, publicValues, proofBytes]).slice(2);
    const r = await fetch(base + "/wallet/triggerconstantcontract", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ owner_address: owner, contract_address: verifierHex, function_selector: "verifyProof(bytes32,bytes,bytes)", parameter, visible: false }),
    });
    const j = await r.json();
    // triggerconstantcontract returns result.result===true even on a contract
    // REVERT/OutOfEnergy (it reports a `message`). Verified == clean call (no msg).
    const verified = !!(j.result && j.result.result === true && !j.result.message);
    const msg = j.result && j.result.message ? Buffer.from(j.result.message, "hex").toString().split(":").pop().trim() : "";
    return { verified, energy: j.energy_used, msg };
  }

  console.log(`SP1Verifier ${verifier} on ${base}`);
  console.log("vkey:", b.vkey, "(circuit", b.circuit_version + ")");

  const ok = await call(b.public_values, b.proof_bytes);
  console.log(`valid proof     -> ${ok.verified ? "VERIFIED ✅" : "rejected ❌"}  (energy ${ok.energy})`);

  const badProof = b.proof_bytes.slice(0, -2) + (b.proof_bytes.slice(-2) === "00" ? "01" : "00");
  const t1 = await call(b.public_values, badProof);
  console.log(`tampered proof  -> ${t1.verified ? "VERIFIED ❌(BUG)" : "rejected ✅"}  ${t1.msg}`);

  const badPv = b.public_values.slice(0, -2) + "01";
  const t2 = await call(badPv, b.proof_bytes);
  console.log(`tampered pubval -> ${t2.verified ? "VERIFIED ❌(BUG)" : "rejected ✅"}  ${t2.msg}`);

  if (!ok.verified || t1.verified || t2.verified) process.exit(1);
  console.log("\n✅ Real SP1 Groth16 proof verifies on Tron; tamper cases rejected.");
}

main().catch((e) => {
  console.error(e.message || e);
  process.exit(1);
});
