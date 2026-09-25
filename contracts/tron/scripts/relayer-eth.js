"use strict";
const fs = require("fs");
const path = require("path");
const { execFileSync } = require("child_process");
const ethers = require("ethers");
const { loadEnv } = require("./env.js");
const { groupBatches, hex32 } = require("./relayer-lib.js");

const VAULT_ABI = JSON.parse(
  fs.readFileSync(path.join(__dirname, "..", "artifacts/contracts/UnicityBridgeVault.sol/UnicityBridgeVault.json"), "utf8"),
).abi;
const VERIFIER_ERRORS = [
  "error InvalidProof()",
  "error WrongVerifierSelector(bytes4 received, bytes4 expected)",
  "error InvalidExitCode()",
  "error InvalidVkRoot()",
  "error ProofInvalid()",
  "error PublicInputNotInField()",
  "error RouteNotFound(bytes4 selector)",
  "error RouteIsFrozen(bytes4 selector)",
];
const ERC20_ABI = ["function transfer(address to, uint256 value) returns (bool)"];
const LOG_WINDOW = 10_000;
const HOST_BIN =
  process.env.BRIDGE_HOST_BIN || path.join(__dirname, "..", "..", "..", "prover", "target", "debug", "bridge-return-host");

function context() {
  const env = loadEnv();
  const rpc = env.ETH_RPC_URL || "https://ethereum-sepolia-rpc.publicnode.com";
  const chainId = Number(env.ETH_CHAIN_ID || 11155111);
  if (!env.ETH_VAULT) throw new Error("ETH_VAULT not set in .env");
  const provider = new ethers.JsonRpcProvider(rpc, chainId, { staticNetwork: true });
  const signer = env.ETH_SK ? new ethers.NonceManager(new ethers.Wallet(env.ETH_SK, provider)) : null;
  const vault = new ethers.Contract(env.ETH_VAULT, [...VAULT_ABI, ...VERIFIER_ERRORS], signer ?? provider);
  return { rpc, provider, vault, fromBlock: Number(env.ETH_VAULT_DEPLOY_BLOCK || 0) };
}

async function settledLog(vault, fromBlock) {
  const provider = vault.runner.provider ?? vault.runner;
  const latest = await provider.getBlockNumber();
  const events = [];
  for (let start = fromBlock; start <= latest; start += LOG_WINDOW) {
    const end = Math.min(start + LOG_WINDOW - 1, latest);
    const logs = [
      ...(await vault.queryFilter(vault.filters.BatchFulfilled(), start, end)),
      ...(await vault.queryFilter(vault.filters.Released(), start, end)),
    ];
    for (const log of logs) events.push(normalizeLog(log));
  }
  const grouped = groupBatches(events);
  return { batches: grouped.batches, spent_root: hex32(await vault.spentRoot()) };
}

function normalizeLog(log) {
  const base = { blockNumber: log.blockNumber, eventIndex: log.index, txId: log.transactionHash };
  if (log.fragment.name === "BatchFulfilled") {
    return {
      ...base,
      kind: "BatchFulfilled",
      spentRootOld: hex32(log.args.spentRootOld),
      spentRootNew: hex32(log.args.spentRootNew),
      batchSize: Number(log.args.batchSize),
    };
  }
  if (log.fragment.name === "Released") {
    return { ...base, kind: "Released", nullifier: hex32(log.args.nullifier) };
  }
  return { ...base, kind: log.fragment.name };
}

function revertReason(contract, e) {
  if (e.reason) return e.reason;
  const data = e.data ?? e.info?.error?.data ?? e.error?.data;
  if (typeof data === "string") {
    try {
      const parsed = contract.interface.parseError(data);
      if (parsed) return parsed.name === "Error" ? String(parsed.args[0]) : parsed.name;
    } catch (_) {
    }
  }
  return e.shortMessage || e.message || "unknown revert";
}

function leafTuple(l) {
  return [l.nullifier, l.recipient, BigInt(l.amount), l.feeRecipient, BigInt(l.feeAmount), BigInt(l.deadline)];
}

async function settle(vault, bundle) {
  const leaves = (bundle.leaves || []).map(leafTuple);
  const lockRefs = (bundle.lockRefs || []).map((r) => [BigInt(r.nonce), r.digest]);
  let tx;
  try {
    tx = await vault.fulfillBatch(bundle.publicValues, bundle.proofBytes, leaves, lockRefs);
  } catch (e) {
    throw new Error(`fulfillBatch rejected: ${revertReason(vault, e)}`);
  }
  const receipt = await tx.wait();
  if (!receipt || receipt.status !== 1) {
    throw new Error(`fulfillBatch tx ${tx.hash} reverted on chain`);
  }
  return { txid: tx.hash, gasUsed: receipt.gasUsed, blockNumber: receipt.blockNumber };
}

async function simulate(vault, leaves) {
  const provider = vault.runner.provider ?? vault.runner;
  const asset = new ethers.Contract(await vault.ASSET(), ERC20_ABI, provider);
  const vaultAddress = await vault.getAddress();
  const rejected = [];
  for (const l of leaves) {
    try {
      await asset.transfer.staticCall(l.recipient, BigInt(l.amount), { from: vaultAddress });
    } catch (e) {
      rejected.push({ nullifier: l.nullifier, reason: revertReason(asset, e) });
    }
  }
  return { rejected };
}

function readStdin() {
  return new Promise((resolve, reject) => {
    let data = "";
    process.stdin.setEncoding("utf8");
    process.stdin.on("data", (chunk) => (data += chunk));
    process.stdin.on("end", () => resolve(data));
    process.stdin.on("error", reject);
  });
}

async function main() {
  const cmd = process.argv[2];
  const stdin = process.argv.includes("--stdin");
  const { rpc, vault, fromBlock } = context();
  const address = await vault.getAddress();
  if (cmd === "events") {
    const log = await settledLog(vault, fromBlock);
    console.error(`relayer-eth events: vault ${address} on ${rpc} -> ${log.batches.length} settled batch(es), spentRoot ${log.spent_root}`);
    process.stdout.write(JSON.stringify(log));
  } else if (cmd === "scan") {
    const log = await settledLog(vault, fromBlock);
    const eventsOut = process.env.RELAYER_EVENTS_OUT || path.join(require("os").tmpdir(), "relayer-eth-events.json");
    fs.writeFileSync(eventsOut, JSON.stringify(log, null, 2));
    const rebuilt = JSON.parse(execFileSync(HOST_BIN, ["s2-rebuild", eventsOut], { encoding: "utf8", stdio: ["ignore", "pipe", "pipe"] }));
    const synced = rebuilt.spent_root.toLowerCase() === log.spent_root.toLowerCase();
    console.log(`relayer-eth scan: vault ${address}, ${log.batches.length} settled batch(es)`);
    console.log(`  rebuilt spent_root: ${rebuilt.spent_root}`);
    console.log(`  on-chain spentRoot: ${log.spent_root}`);
    console.log(synced ? "  synced: safe to build the next batch" : "  diverged: the log does not reproduce the vault, do not settle");
    if (!synced) process.exit(1);
  } else if (cmd === "settle" && stdin) {
    const bundle = JSON.parse(await readStdin());
    console.error(`relayer-eth settle: batch ${bundle.batchId}, ${(bundle.leaves || []).length} leaf(ves) on vault ${address}`);
    const { txid, gasUsed, blockNumber } = await settle(vault, bundle);
    console.error(`settled in block ${blockNumber}, gas ${gasUsed}: ${txid}`);
    process.stdout.write(txid);
  } else if (cmd === "simulate" && stdin) {
    const payload = JSON.parse(await readStdin());
    process.stdout.write(JSON.stringify(await simulate(vault, payload.leaves || [])));
  } else {
    console.error("usage: relayer-eth.js events | scan | settle --stdin | simulate --stdin");
    process.exit(2);
  }
}

module.exports = { settledLog, settle, simulate, normalizeLog, VAULT_ABI, VERIFIER_ERRORS };

if (require.main === module) {
  main().catch((e) => {
    console.error(e.message || e);
    process.exit(1);
  });
}
