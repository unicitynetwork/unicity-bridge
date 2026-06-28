// S4 relayer — observe the vault, verify accumulator sync, and settle batches.
// ZK_BACK3 §13 ("anyone can self-settle after the deadline"): this is a
// permissionless tool. It delegates accumulator math to the Rust host
// (`s2-rebuild`) and proving to the host (`sp1-groth16`); it owns event decoding,
// state verification, and tx submission.
//
//   node scripts/relayer.js scan                       # rebuild from events, verify vs on-chain spentRoot
//   node scripts/relayer.js settle <bundle> <settle>   # submit fulfillBatch for a prepared batch
//
// The full self-settle loop:
//   1. scan      — confirm the local view matches the chain (this tool)
//   2. host s2   — next_batch witnesses on top of the rebuilt accumulator
//   3. host sp1-groth16  — prove the batch
//   4. settle    — submit fulfillBatch (this tool)

"use strict";
const fs = require("fs");
const path = require("path");
const { execFileSync } = require("child_process");
const ethers = require("ethers");
const TronWebLib = require("tronweb");
const TronWeb = TronWebLib.TronWeb || TronWebLib.default || TronWebLib;
const { normalizeEvent, groupBatches, hex32 } = require("./relayer-lib.js");

function loadEnv() {
  const env = {};
  const p = path.join(__dirname, "..", "..", "..", ".env");
  for (const line of fs.readFileSync(p, "utf8").split("\n")) {
    const m = line.match(/^\s*([A-Za-z0-9_]+)\s*=\s*(.*)\s*$/);
    if (m && !line.trim().startsWith("#")) env[m[1]] = m[2];
  }
  return env;
}

const HOST_BIN =
  process.env.BRIDGE_HOST_BIN ||
  path.join(__dirname, "..", "..", "..", "prover", "target", "debug", "bridge-return-host");
const EVENTS_OUT = process.env.RELAYER_EVENTS_OUT || "/tmp/relayer-events.json";

// Fetch all events of a given name from TronGrid, following pagination.
async function fetchEvents(base, vaultBase58, eventName) {
  const out = [];
  let url =
    `${base}/v1/contracts/${vaultBase58}/events` +
    `?event_name=${eventName}&order_by=block_timestamp,asc&limit=200`;
  for (let page = 0; page < 100 && url; page++) {
    const res = await fetch(url);
    const json = await res.json();
    for (const ev of json.data || []) out.push(ev);
    url = json.meta && json.meta.links && json.meta.links.next;
  }
  return out;
}

async function vaultSpentRoot(tw, vaultHex) {
  const abi = JSON.parse(
    fs.readFileSync(
      path.join(__dirname, "..", "artifacts/contracts/UnicityBridgeVault.sol/UnicityBridgeVault.json"),
      "utf8"
    )
  ).abi;
  const v = tw.contract(abi, vaultHex);
  return hex32(await v.spentRoot().call());
}

async function scan() {
  const env = loadEnv();
  const base = (env.TRON_RPC_URL || "https://nile.trongrid.io").replace(/\/$/, "");
  const tw = new TronWeb({ fullHost: base, privateKey: env.TRON_SK });
  const vaultBase58 = env.TRON_VAULT;
  const vaultHex = tw.address.toHex(vaultBase58);
  console.log(`relayer scan: vault ${vaultBase58} on ${base}`);

  const raw = [
    ...(await fetchEvents(base, vaultBase58, "BatchFulfilled")),
    ...(await fetchEvents(base, vaultBase58, "Released")),
  ];
  const events = raw.map(normalizeEvent);
  const grouped = groupBatches(events);
  fs.writeFileSync(EVENTS_OUT, JSON.stringify(grouped, null, 2));
  console.log(
    `  decoded ${events.length} events -> ${grouped.batches.length} settled batch(es) -> ${EVENTS_OUT}`
  );

  // S2 rebuild + verify each transition against the chain. A non-zero exit means
  // the accumulator can't be reproduced from the log (incomplete/tampered, or a
  // root from a different accumulator version) — refuse to settle.
  let rebuilt;
  try {
    rebuilt = JSON.parse(
      execFileSync(HOST_BIN, ["s2-rebuild", EVENTS_OUT], {
        encoding: "utf8",
        stdio: ["ignore", "pipe", "pipe"],
      })
    );
  } catch (e) {
    const msg = (e.stderr || e.stdout || e.message || "").toString().trim();
    console.log(`  ❌ S2 could not reproduce the chain accumulator:\n     ${msg.split("\n")[0]}`);
    console.log("  Refusing to settle. (A vault settled by a different accumulator version will not match.)");
    process.exit(1);
  }
  const onchain = await vaultSpentRoot(tw, vaultHex);
  const synced = rebuilt.spent_root.toLowerCase() === onchain.toLowerCase();
  console.log(`  rebuilt spent_root: ${rebuilt.spent_root}`);
  console.log(`  on-chain spentRoot: ${onchain}`);
  console.log(`  spent nullifiers:   ${rebuilt.spent_count}`);
  console.log(
    synced
      ? "  ✅ SYNCED — local accumulator matches the chain; safe to build the next batch"
      : "  ❌ DIVERGED — event log incomplete/tampered; do NOT settle"
  );
  if (!synced) process.exit(1);
}

// Accept emit-settlement (single leaf/lock_ref) or emit-settlement-b2 (arrays).
function readBatch(settle) {
  const leaves = settle.leaves || (settle.leaf ? [settle.leaf] : []);
  const refs = settle.lock_refs || (settle.lock_ref ? [settle.lock_ref] : []);
  return {
    leaves: leaves.map((l) => [
      l.nullifier,
      l.recipient,
      BigInt(l.amount),
      l.fee_recipient,
      BigInt(l.fee_amount),
      BigInt(l.deadline),
    ]),
    lockRefs: refs.map((r) => [BigInt(r.nonce), r.digest]),
  };
}

async function settle(bundlePath, settlePath) {
  const env = loadEnv();
  const base = (env.TRON_RPC_URL || "https://nile.trongrid.io").replace(/\/$/, "");
  const tw = new TronWeb({ fullHost: base, privateKey: env.TRON_SK });
  const meHex = tw.address.toHex(tw.defaultAddress.base58);
  const vaultHex = tw.address.toHex(env.TRON_VAULT);
  const bundle = JSON.parse(fs.readFileSync(bundlePath, "utf8"));
  const { leaves, lockRefs } = readBatch(JSON.parse(fs.readFileSync(settlePath, "utf8")));
  console.log(`relayer settle: ${leaves.length} leaf(s) on vault ${env.TRON_VAULT}`);

  const coder = ethers.AbiCoder.defaultAbiCoder();
  const funcSig =
    "fulfillBatch(bytes,bytes,(bytes32,address,uint256,address,uint256,uint64)[],(uint256,bytes32)[])";
  const rawParameter = coder
    .encode(
      [
        "bytes",
        "bytes",
        "tuple(bytes32,address,uint256,address,uint256,uint64)[]",
        "tuple(uint256,bytes32)[]",
      ],
      [bundle.public_values, bundle.proof_bytes, leaves, lockRefs]
    )
    .slice(2);
  const built = await tw.transactionBuilder.triggerSmartContract(
    vaultHex,
    funcSig,
    { feeLimit: 1_000_000_000, rawParameter },
    [],
    meHex
  );
  const res = await tw.trx.sendRawTransaction(await tw.trx.sign(built.transaction));
  const id = res.txid || built.transaction.txID;
  let info;
  for (let i = 0; i < 20 && !(info && info.receipt); i++) {
    await new Promise((r) => setTimeout(r, 3000));
    info = await tw.trx.getTransactionInfo(id);
  }
  const ok = info && info.receipt && info.receipt.result === "SUCCESS";
  console.log(
    `  fulfillBatch tx ${id} -> ${info?.receipt?.result} (energy ${info?.receipt?.energy_usage_total})`
  );
  if (!ok) {
    console.log("  resMessage:", info?.resMessage ? Buffer.from(info.resMessage, "hex").toString() : "?");
    process.exit(1);
  }
  console.log(`  🎉 settled. https://nile.tronscan.org/#/transaction/${id}`);
}

async function main() {
  const cmd = process.argv[2];
  if (cmd === "scan") return scan();
  if (cmd === "settle") {
    const [, , , bundle, settlePath] = process.argv;
    if (!bundle || !settlePath) throw new Error("usage: relayer.js settle <bundle.json> <settle.json>");
    return settle(bundle, settlePath);
  }
  console.log("usage: relayer.js scan | settle <bundle.json> <settle.json>");
}

main().catch((e) => {
  console.error(e.message || e);
  process.exit(1);
});
