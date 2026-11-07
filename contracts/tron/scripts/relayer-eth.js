"use strict";
const fs = require("fs");
const path = require("path");
const { execFileSync } = require("child_process");
const ethers = require("ethers");
const { loadEnv } = require("./env.js");
const { groupBatches, hex32, releaseLogged, beforeDeadline, exitWith } = require("./relayer-lib.js");

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
// A read the events scan makes is repeated after 1, 2 and 4 seconds before the scan
// fails, since one failed request out of the dozens a scan makes would otherwise fail it.
const RPC_RETRY = { retries: 3, delayMs: 1000 };
// What a repeat of the same read can clear: a timeout, an HTTP 5xx, a dropped socket, and
// the JSON-RPC errors ethers cannot classify, which is how a node's rate limit arrives.
const TRANSIENT_RPC_ERRORS = new Set([
  "TIMEOUT",
  "SERVER_ERROR",
  "NETWORK_ERROR",
  "UNKNOWN_ERROR",
  "ECONNRESET",
  "ECONNREFUSED",
  "ETIMEDOUT",
  "EAI_AGAIN",
]);
const HOST_BIN =
  process.env.BRIDGE_HOST_BIN || path.join(__dirname, "..", "..", "..", "prover", "target", "debug", "bridge-return-host");
const RELEASED_TOPIC = ethers.id("Released(bytes32,address,uint256,address,uint256,uint64)");
// `settle` exits with this status (EX_TEMPFAIL) when it broadcast the settlement but could
// not read its receipt in time: stdout still carries the txid, and the service settles or
// refutes the batch from the vault's log instead of failing it.
const UNCONFIRMED_EXIT = 75;
const RECEIPT_POLL_MS = 4000;
// The service kills a command after BRIDGE_RETURN_COMMAND_TIMEOUT_SECS (600 unless set);
// the receipt wait ends this long before that, so the txid still reaches the service.
const STARTED_MS = Date.now();
const TIMEOUT_MARGIN_MS = 30_000;

function context() {
  const env = loadEnv();
  const rpc = env.ETH_RPC_URL || "https://ethereum-sepolia-rpc.publicnode.com";
  const chainId = Number(env.ETH_CHAIN_ID || 11155111);
  if (!env.ETH_VAULT) throw new Error("ETH_VAULT not set in .env");
  const provider = new ethers.JsonRpcProvider(rpc, chainId, { staticNetwork: true });
  const signer = env.ETH_SK ? new ethers.NonceManager(new ethers.Wallet(env.ETH_SK, provider)) : null;
  const vault = new ethers.Contract(env.ETH_VAULT, [...VAULT_ABI, ...VERIFIER_ERRORS], signer ?? provider);
  return {
    rpc,
    chainId,
    confirmRpc: env.ETH_CONFIRM_RPC_URL,
    provider,
    vault,
    fromBlock: Number(env.ETH_VAULT_DEPLOY_BLOCK || 0),
    settleConfirmations: Number(env.ETH_SETTLE_CONFIRMATIONS || 12),
    settleUntil: STARTED_MS + Number(env.BRIDGE_RETURN_COMMAND_TIMEOUT_SECS || 600) * 1000 - TIMEOUT_MARGIN_MS,
  };
}

// The second RPC releases are confirmed through: never the endpoint the settlement log
// was read from, since a release only one endpoint reports is not confirmed.
function confirmationProvider({ confirmRpc, rpc, chainId }) {
  if (!confirmRpc) throw new Error("ETH_CONFIRM_RPC_URL not set: no second RPC to confirm a release through");
  const endpoint = (url) => String(url).trim().replace(/\/+$/, "").toLowerCase();
  if (endpoint(confirmRpc) === endpoint(rpc)) {
    throw new Error("ETH_CONFIRM_RPC_URL is ETH_RPC_URL: a release must be confirmed through a second, independent RPC");
  }
  return new ethers.JsonRpcProvider(confirmRpc, chainId, { staticNetwork: true });
}

async function retried(what, read, { retries, delayMs } = RPC_RETRY) {
  for (let attempt = 0; ; attempt++) {
    try {
      return await read();
    } catch (e) {
      if (attempt >= retries || !TRANSIENT_RPC_ERRORS.has(e.code)) throw e;
      console.error(`relayer-eth: ${what} failed (${e.shortMessage || e.message}); retry ${attempt + 1} of ${retries}`);
      await new Promise((r) => setTimeout(r, delayMs * 2 ** attempt));
    }
  }
}

async function settledLog(vault, fromBlock, confirmations = 1, retry = RPC_RETRY) {
  const provider = vault.runner.provider ?? vault.runner;
  const head = await retried("block number", () => provider.getBlockNumber(), retry);
  const final = Math.max(head - (confirmations - 1), fromBlock);
  const events = [];
  for (let start = fromBlock; start <= final; start += LOG_WINDOW) {
    const end = Math.min(start + LOG_WINDOW - 1, final);
    for (const name of ["BatchFulfilled", "Released"]) {
      const logs = await retried(`${name} logs ${start}..${end}`, () => vault.queryFilter(vault.filters[name](), start, end), retry);
      for (const log of logs) events.push(normalizeLog(log));
    }
  }
  const grouped = groupBatches(events);
  const root = await retried("spentRoot", () => vault.spentRoot({ blockTag: final }), retry);
  return { batches: grouped.batches, spent_root: hex32(root) };
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

// Once broadcast, the settlement is reported by its txid whatever the receipt reads say:
// `confirmed` is false when no receipt with `confirmations` was read by `until`, and only a
// receipt that shows a revert fails it.
async function settle(vault, bundle, confirmations = 1, { provider, until = Infinity, pollMs = RECEIPT_POLL_MS } = {}) {
  const leaves = (bundle.leaves || []).map(leafTuple);
  const lockRefs = (bundle.lockRefs || []).map((r) => [BigInt(r.nonce), r.digest]);
  let tx;
  try {
    tx = await vault.fulfillBatch(bundle.publicValues, bundle.proofBytes, leaves, lockRefs);
  } catch (e) {
    throw new Error(`fulfillBatch rejected: ${revertReason(vault, e)}`);
  }
  const reader = provider ?? vault.runner.provider ?? vault.runner;
  const receipt = await receiptOf(reader, tx.hash, confirmations, { until, pollMs });
  if (!receipt) return { txid: tx.hash, confirmed: false };
  if (receipt.status !== 1) {
    throw new Error(`fulfillBatch tx ${tx.hash} reverted on chain`);
  }
  return { txid: tx.hash, confirmed: true, gasUsed: receipt.gasUsed, blockNumber: receipt.blockNumber };
}

// Polls for the receipt rather than calling tx.wait(): the block poller tx.wait() starts
// lets a failed request escape as an unhandled rejection, which ends the process, so one
// rate-limited read would report a settlement that went through as failed. Every failed
// read is read again, until `until`, which also cuts off a read still pending then.
async function receiptOf(provider, hash, confirmations, { until, pollMs }) {
  for (;;) {
    try {
      const receipt = await beforeDeadline(provider.getTransactionReceipt(hash), until);
      if (receipt && (await beforeDeadline(provider.getBlockNumber(), until)) - receipt.blockNumber + 1 >= confirmations) {
        return receipt;
      }
    } catch (e) {
      console.error(`relayer-eth: receipt of ${hash} unread (${e.shortMessage || e.message}); reading it again`);
    }
    const left = until - Date.now();
    if (left <= 0) return null;
    await new Promise((r) => setTimeout(r, Math.min(pollMs, left)));
  }
}

// Each release (a nullifier and the transaction the settlement log names for it) is
// confirmed only if `provider` holds that transaction's receipt, succeeded, at least
// `confirmations` deep, with the vault's Released event for the nullifier in its logs.
async function confirmReleases(provider, vaultAddress, releases, confirmations = 1, retry = RPC_RETRY) {
  const head = await retried("block number", () => provider.getBlockNumber(), retry);
  const confirmed = [];
  const unconfirmed = [];
  for (const { nullifier, txid } of releases) {
    const receipt = await retried(`receipt of ${txid}`, () => provider.getTransactionReceipt(txid), retry);
    const depth = receipt ? Math.max(head - receipt.blockNumber + 1, 0) : 0;
    const reason = !receipt
      ? "the confirmation RPC has no receipt for the transaction"
      : receipt.status !== 1
        ? "the transaction reverted"
        : !releaseLogged(receipt.logs, { vault: vaultAddress, topic: RELEASED_TOPIC, nullifier })
          ? "the transaction emitted no Released event for the nullifier from the vault"
          : depth < confirmations
            ? `${depth} of ${confirmations} confirmations`
            : null;
    if (reason) unconfirmed.push({ nullifier, txid, reason });
    else confirmed.push({ nullifier, txid });
  }
  return { confirmed, unconfirmed };
}

// Each rejection names its kind: "lock" (the vault holds no such lock, final for the
// burn), "unconfirmed" (the lock is not yet `confirmations` deep, retried) or
// "recipient" (the payout would revert now, retried). Lock digests are read at the
// same confirmed block the settled log is read at.
async function simulate(vault, leaves, confirmations = 1) {
  const provider = vault.runner.provider ?? vault.runner;
  const confirmed = Math.max((await provider.getBlockNumber()) - (confirmations - 1), 0);
  const rejected = [];
  for (const l of leaves) {
    const reason = await unknownLock(vault, l.lockRefs || [], confirmed);
    if (!reason) continue;
    const kind = (await unknownLock(vault, l.lockRefs || [], "latest")) ? "lock" : "unconfirmed";
    rejected.push({ nullifier: l.nullifier, reason, kind });
  }
  if (await vault.PULL_PAYMENTS()) return { rejected };
  const asset = new ethers.Contract(await vault.ASSET(), ERC20_ABI, provider);
  const vaultAddress = await vault.getAddress();
  const now = BigInt(Math.floor(Date.now() / 1000));
  const refused = new Set(rejected.map((r) => r.nullifier));
  for (const l of leaves) {
    if (refused.has(l.nullifier)) continue;
    const fee = BigInt(l.feeRecipient) !== 0n && now <= BigInt(l.deadline) ? BigInt(l.feeAmount) : 0n;
    const principal = BigInt(l.amount) - fee;
    const reason =
      (await transferFails(asset, vaultAddress, l.recipient, principal)) ??
      (await transferFails(asset, vaultAddress, l.feeRecipient, fee));
    if (reason) rejected.push({ nullifier: l.nullifier, reason, kind: "recipient" });
  }
  return { rejected };
}

// The vault skips a payout of zero or to itself, and on EVM chains a transfer that
// returns false has failed; a void return (Ethereum USDT) decodes to nothing and passes.
async function transferFails(asset, vaultAddress, to, amount) {
  if (amount === 0n || to.toLowerCase() === vaultAddress.toLowerCase()) return null;
  try {
    const ok = await asset.transfer.staticCall(to, amount, { from: vaultAddress });
    return ok === false ? "transfer returned false" : null;
  } catch (e) {
    if (e.code === "BAD_DATA") return null;
    return revertReason(asset, e);
  }
}

async function unknownLock(vault, lockRefs, blockTag) {
  for (const ref of lockRefs) {
    const stored = await vault.lockDigest(BigInt(ref.nonce), { blockTag });
    if (stored.toLowerCase() !== ref.digest.toLowerCase()) {
      return `lock ${BigInt(ref.nonce)} digest ${ref.digest} is not the vault's ${stored}`;
    }
  }
  return null;
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
  const ctx = context();
  const { rpc, vault, fromBlock, settleConfirmations } = ctx;
  const address = await vault.getAddress();
  if (cmd === "events") {
    const log = await settledLog(vault, fromBlock, settleConfirmations);
    console.error(`relayer-eth events: vault ${address} on ${rpc} -> ${log.batches.length} settled batch(es), spentRoot ${log.spent_root}`);
    process.stdout.write(JSON.stringify(log));
  } else if (cmd === "scan") {
    const log = await settledLog(vault, fromBlock, settleConfirmations);
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
    const result = await settle(vault, bundle, settleConfirmations, { until: ctx.settleUntil });
    if (!result.confirmed) {
      console.error(
        `broadcast ${result.txid}, but no receipt with ${settleConfirmations} confirmation(s) could be read; the service looks for it in the vault's log`,
      );
      exitWith(result.txid, UNCONFIRMED_EXIT);
      return;
    }
    console.error(`settled in block ${result.blockNumber}, gas ${result.gasUsed}: ${result.txid}`);
    process.stdout.write(result.txid);
  } else if (cmd === "simulate" && stdin) {
    const payload = JSON.parse(await readStdin());
    process.stdout.write(JSON.stringify(await simulate(vault, payload.leaves || [], settleConfirmations)));
  } else if (cmd === "confirm" && stdin) {
    const { releases = [] } = JSON.parse(await readStdin());
    const out = await confirmReleases(confirmationProvider(ctx), address, releases, settleConfirmations);
    console.error(`relayer-eth confirm: ${out.confirmed.length} of ${releases.length} release(s) confirmed on the second RPC`);
    process.stdout.write(JSON.stringify(out));
  } else {
    console.error("usage: relayer-eth.js events | scan | settle --stdin | simulate --stdin | confirm --stdin");
    process.exit(2);
  }
}

module.exports = { settledLog, settle, simulate, confirmReleases, confirmationProvider, normalizeLog, VAULT_ABI, VERIFIER_ERRORS };

if (require.main === module) {
  main().catch((e) => {
    console.error(e.message || e);
    process.exit(1);
  });
}
