const fs = require("fs");
const path = require("path");
const ethers = require("ethers");
const { loadEnv } = require("./env");

const ABI = ["function verifyProof(bytes32 programVKey, bytes publicValues, bytes proofBytes) view"];

async function main() {
  const env = loadEnv();
  const rpc = env.ETH_RPC_URL || "https://ethereum-sepolia-rpc.publicnode.com";
  const provider = new ethers.JsonRpcProvider(rpc, Number(env.ETH_CHAIN_ID || 11155111), { staticNetwork: true });
  const bundlePath = process.argv[2] || path.join(__dirname, "..", "..", "..", "protocol/vectors/proof/b1-groth16.json");
  const b = JSON.parse(fs.readFileSync(bundlePath, "utf8"));
  const targets = {
    gateway: env.ETH_SP1_GATEWAY || "0x397A5f7f3dBd538f23DE225B51f532c34448dA9B",
    "v6.1.0 verifier": env.ETH_SP1_VERIFIER_V610 || "0xb69f2584CBcFf99a58C4e7002E8b89Af54a6f4e2",
  };
  console.log("bundle:", bundlePath);
  console.log("vkey:", b.vkey, "(circuit", b.circuit_version + ", sp1", b.sp1_version + ")");
  const block = await provider.getBlock("latest");
  console.log(`chain ${env.ETH_CHAIN_ID || 11155111} block ${block.number}, gas limit ${block.gasLimit}, base fee ${ethers.formatUnits(block.baseFeePerGas ?? 0n, "gwei")} gwei`);

  async function call(address, publicValues, proofBytes) {
    const c = new ethers.Contract(address, ABI, provider);
    const t0 = performance.now();
    try {
      await c.verifyProof(b.vkey, publicValues, proofBytes);
      const ms = Math.round(performance.now() - t0);
      const gas = await c.verifyProof.estimateGas(b.vkey, publicValues, proofBytes);
      return { verified: true, ms, gas: gas.toString(), msg: "" };
    } catch (e) {
      const ms = Math.round(performance.now() - t0);
      return { verified: false, ms, gas: "-", msg: (e.shortMessage || e.message || "").slice(0, 120) };
    }
  }

  let failed = false;
  for (const [label, address] of Object.entries(targets)) {
    console.log(`\n${label} ${address}`);
    const ok = await call(address, b.public_values, b.proof_bytes);
    console.log(`  valid proof     -> ${ok.verified ? "VERIFIED" : "rejected"}  gas ${ok.gas}  round trip ${ok.ms} ms  ${ok.msg}`);
    const badProof = b.proof_bytes.slice(0, -2) + (b.proof_bytes.slice(-2) === "00" ? "01" : "00");
    const t1 = await call(address, b.public_values, badProof);
    console.log(`  tampered proof  -> ${t1.verified ? "VERIFIED (BUG)" : "rejected"}  ${t1.msg}`);
    const badPv = b.public_values.slice(0, -2) + "01";
    const t2 = await call(address, badPv, b.proof_bytes);
    console.log(`  tampered pubval -> ${t2.verified ? "VERIFIED (BUG)" : "rejected"}  ${t2.msg}`);
    if (!ok.verified || t1.verified || t2.verified) failed = true;
  }
  if (failed) process.exit(1);
  console.log("\nThe bundle verifies on Sepolia; tampered inputs are rejected.");
}

main().catch((e) => {
  console.error(e.message || e);
  process.exit(1);
});
