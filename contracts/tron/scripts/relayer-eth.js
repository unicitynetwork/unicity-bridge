"use strict";
const fs = require("fs");
const path = require("path");
const { execFileSync } = require("child_process");
const ethers = require("ethers");
const { loadEnv } = require("./env.js");
const { groupBatches, hex32, releaseLogged, beforeDeadline, exitWith, commandDeadline, UNCONFIRMED_EXIT } = require("./relayer-lib.js");

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
const RECEIPT_POLL_MS = 4000;
// A second RPC can trail the one the log came from by a block or two, so a release it shows
// short of depth only is read again once after a block's time before it goes unconfirmed.
const CONFIRM_CATCH_UP_MS = 15_000;

function context() {
  const env = loadEnv();
  const rpc = env.ETH_RPC_URL || "https://ethereum-sepolia-rpc.publicnode.com";
  const chainId = Number(env.ETH_CHAIN_ID || 11155111);
  if (!env.ETH_VAULT) throw new Error("ETH_VAULT not set in .env");
  const provider = new ethers.JsonRpcProvider(rpc, chainId, { staticNetwork: true });
  // A Wallet, which signs a transaction before sending it, so the settlement's hash is known
  // whatever becomes of the send.
  const signer = env.ETH_SK ? new ethers.Wallet(env.ETH_SK, provider) : null;
  const vault = new ethers.Contract(env.ETH_VAULT, [...VAULT_ABI, ...VERIFIER_ERRORS], signer ?? provider);
  return {
    rpc,
    chainId,
    confirmRpc: env.ETH_CONFIRM_RPC_URL,
    provider,
    vault,
    fromBlock: Number(env.ETH_VAULT_DEPLOY_BLOCK || 0),
    settleConfirmations: Number(env.ETH_SETTLE_CONFIRMATIONS || 12),
    settleUntil: commandDeadline(),
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

// The settlement is signed before it is sent, so its txid is known whatever becomes of the
// send, and from then on it is reported by that txid whatever the reads say: `confirmed` is
// false when no receipt with `confirmations` was read by `until`. Only a revert, before the
// send or in the receipt, or a node refusing the transaction outright fails it.
async function settle(
  vault,
  bundle,
  confirmations = 1,
  { provider, sender, until = Infinity, pollMs = RECEIPT_POLL_MS, retry = RPC_RETRY } = {},
) {
  const leaves = (bundle.leaves || []).map(leafTuple);
  const lockRefs = (bundle.lockRefs || []).map((r) => [BigInt(r.nonce), r.digest]);
  const signer = vault.runner;
  let signed;
  try {
    const request = await vault.fulfillBatch.populateTransaction(bundle.publicValues, bundle.proofBytes, leaves, lockRefs);
    signed = await signer.signTransaction(await retried("settlement preparation", () => signer.populateTransaction(request), retry));
  } catch (e) {
    throw new Error(`fulfillBatch rejected: ${revertReason(vault, e)}`);
  }
  const txid = ethers.Transaction.from(signed).hash;
  await broadcast(sender ?? signer.provider, signed, retry);
  const receipt = await receiptOf(provider ?? signer.provider, txid, confirmations, { until, pollMs });
  if (!receipt) return { txid, confirmed: false };
  if (receipt.status !== 1) {
    throw new Error(`fulfillBatch tx ${txid} reverted on chain`);
  }
  return { txid, confirmed: true, gasUsed: receipt.gasUsed, blockNumber: receipt.blockNumber };
}

function rpcMessage(e) {
  return [e.shortMessage, e.message, e.info?.error?.message, e.error?.message].filter(Boolean).join(" ");
}

// How a node answers a signed transaction it already holds, or whose nonce is already spent.
function alreadySent(e) {
  return e.code === "NONCE_EXPIRED" || /already known|known transaction|already imported|nonce too low/i.test(rpcMessage(e));
}

// Sends the signed settlement with eth_sendRawTransaction alone. ethers' broadcastTransaction
// reads the block number and the network alongside the send and throws if either read fails,
// though the transaction has gone out, which reported a settlement in flight as failed. After
// a failure a repeat can clear, the same signed transaction is sent again: it has one hash, and
// a node that holds it already says so. A node refusing the transaction itself (funds, fees)
// fails the settlement, since it never went out. Resolves to whether a node acknowledged it;
// when no node answered, the transaction may still have gone out, and its receipt decides.
async function broadcast(sender, signed, retry = RPC_RETRY) {
  for (let attempt = 0; ; attempt++) {
    try {
      await sender.send("eth_sendRawTransaction", [signed]);
      return true;
    } catch (e) {
      if (alreadySent(e)) return true;
      if (!TRANSIENT_RPC_ERRORS.has(e.code)) throw new Error(`fulfillBatch refused by the node: ${e.shortMessage || e.message}`);
      if (attempt >= retry.retries) {
        console.error(`relayer-eth: the settlement was sent without an answer (${e.shortMessage || e.message}); its receipt decides whether it went out`);
        return false;
      }
      console.error(`relayer-eth: sending the settlement failed (${e.shortMessage || e.message}); sending it again, ${attempt + 1} of ${retry.retries}`);
      await new Promise((r) => setTimeout(r, retry.delayMs * 2 ** attempt));
    }
  }
}

// What each settlement transaction's receipt says through `provider`, the RPC the log comes
// from: "confirmed" once it succeeded `confirmations` deep, "reverted" once it failed that
// deep, "pending" while it is shallower or known to the node unmined, and "unknown" when the
// node has neither its receipt nor the transaction.
async function settlementStates(provider, txids, confirmations = 1, retry = RPC_RETRY) {
  if (!txids.length) return { settlements: [] };
  const head = await retried("block number", () => provider.getBlockNumber(), retry);
  const settlements = [];
  for (const txid of txids) {
    const receipt = await retried(`receipt of ${txid}`, () => provider.getTransactionReceipt(txid), retry);
    if (receipt) {
      const depth = Math.max(head - receipt.blockNumber + 1, 0);
      const state = depth < confirmations ? "pending" : receipt.status === 1 ? "confirmed" : "reverted";
      settlements.push({ txid, state, confirmations: depth });
      continue;
    }
    const tx = await retried(`transaction ${txid}`, () => provider.getTransaction(txid), retry);
    settlements.push({ txid, state: tx ? "pending" : "unknown", confirmations: 0 });
  }
  return { settlements };
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
// `confirmations` deep, with the vault's Released event for the nullifier in its logs. A
// release short of depth only is read again once after `catchUpMs`. No releases means
// nothing is read: the service asks that at startup to see that the command runs.
async function confirmReleases(provider, vaultAddress, releases, confirmations = 1, retry = RPC_RETRY, catchUpMs = CONFIRM_CATCH_UP_MS) {
  if (!releases.length) return { confirmed: [], unconfirmed: [] };
  const first = await checkReleases(provider, vaultAddress, releases, confirmations, retry);
  const shallow = first.unconfirmed.filter((u) => u.shallow);
  if (!shallow.length || !(catchUpMs > 0)) return withoutDepthMarks(first);
  await new Promise((r) => setTimeout(r, catchUpMs));
  const again = await checkReleases(provider, vaultAddress, shallow, confirmations, retry);
  return withoutDepthMarks({
    confirmed: [...first.confirmed, ...again.confirmed],
    unconfirmed: [...first.unconfirmed.filter((u) => !u.shallow), ...again.unconfirmed],
  });
}

function withoutDepthMarks({ confirmed, unconfirmed }) {
  return { confirmed, unconfirmed: unconfirmed.map(({ nullifier, txid, reason }) => ({ nullifier, txid, reason })) };
}

async function checkReleases(provider, vaultAddress, releases, confirmations, retry) {
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
    if (!reason) confirmed.push({ nullifier, txid });
    else unconfirmed.push({ nullifier, txid, reason, shallow: !!receipt && reason.endsWith(" confirmations") });
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
        `broadcast ${result.txid}, but no receipt with ${settleConfirmations} confirmation(s) could be read; the service waits on it`,
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
  } else if (cmd === "receipt" && stdin) {
    const { txids = [] } = JSON.parse(await readStdin());
    const out = await settlementStates(ctx.provider, txids, settleConfirmations);
    console.error(`relayer-eth receipt: ${out.settlements.map((s) => `${s.txid} ${s.state}`).join(", ") || "nothing asked"}`);
    process.stdout.write(JSON.stringify(out));
  } else {
    console.error("usage: relayer-eth.js events | scan | settle --stdin | simulate --stdin | confirm --stdin | receipt --stdin");
    process.exit(2);
  }
}

module.exports = {
  settledLog,
  settle,
  simulate,
  confirmReleases,
  confirmationProvider,
  settlementStates,
  normalizeLog,
  VAULT_ABI,
  VERIFIER_ERRORS,
};

if (require.main === module) {
  main().catch((e) => {
    console.error(e.message || e);
    process.exit(1);
  });
}
