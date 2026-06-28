// S4 relayer — pure helpers (no network), unit-tested in test/relayer.test.js.
//
// The relayer reconstructs the vault's settlement history from its events so it
// can (a) confirm it is in sync with the chain and (b) build the next batch. The
// accumulator math itself is delegated to the Rust host (`s2-rebuild`), so these
// helpers only turn the event log into the `events.json` that S2 consumes.
//
// fulfillBatch emits, in one tx and in this order: BatchFulfilled{spentRootOld,
// spentRootNew, batchSize, totalAmount} then `batchSize` × Released{nullifier,…}.
// Grouping therefore walks events in (blockNumber, eventIndex) order and, for
// each BatchFulfilled, claims the next `batchSize` Released as its leaves.

"use strict";

/// Normalize anything (hex with/without 0x, decimal string, number, bigint) to a
/// 0x-prefixed 32-byte hex string.
function hex32(x) {
  if (typeof x === "bigint" || typeof x === "number") {
    return "0x" + BigInt(x).toString(16).padStart(64, "0");
  }
  let s = String(x).trim();
  if (s.startsWith("0x") || s.startsWith("0X")) s = s.slice(2);
  if (/^[0-9]+$/.test(s) && s.length < 64) s = BigInt(s).toString(16);
  if (s.length > 64 && s.length % 2 === 0) s = s.slice(-64); // tolerate 41-prefixed etc.
  return "0x" + s.toLowerCase().padStart(64, "0");
}

/// Map a TronGrid decoded event (`{ event_name, result, block_number,
/// event_index }`) to the relayer's clean shape. Field access is tolerant of
/// TronGrid returning either named keys or positional ("0","1",…) results.
function normalizeEvent(raw) {
  const r = raw.result || {};
  const pick = (name, idx) => (r[name] !== undefined ? r[name] : r[String(idx)]);
  const base = {
    blockNumber: Number(raw.block_number ?? raw.blockNumber ?? 0),
    eventIndex: Number(raw.event_index ?? raw.eventIndex ?? 0),
    txId: raw.transaction_id || raw.txId,
  };
  if (raw.event_name === "BatchFulfilled") {
    return {
      ...base,
      kind: "BatchFulfilled",
      spentRootOld: hex32(pick("spentRootOld", 0)),
      spentRootNew: hex32(pick("spentRootNew", 1)),
      batchSize: Number(pick("batchSize", 2)),
    };
  }
  if (raw.event_name === "Released") {
    return { ...base, kind: "Released", nullifier: hex32(pick("nullifier", 0)) };
  }
  return { ...base, kind: raw.event_name };
}

/// Group ordered events into the S2 `events.json` shape:
/// `{ batches: [{ nullifiers, spent_root_old, spent_root_new }] }`.
function groupBatches(events) {
  const sorted = [...events].sort(
    (a, b) => a.blockNumber - b.blockNumber || a.eventIndex - b.eventIndex
  );
  const batches = [];
  for (let i = 0; i < sorted.length; i++) {
    const e = sorted[i];
    if (e.kind !== "BatchFulfilled") continue;
    const nullifiers = [];
    for (let j = i + 1; j < sorted.length && nullifiers.length < e.batchSize; j++) {
      if (sorted[j].kind === "BatchFulfilled") break; // next batch began; log truncated
      if (sorted[j].kind === "Released") nullifiers.push(sorted[j].nullifier);
    }
    if (nullifiers.length !== e.batchSize) {
      throw new Error(
        `batch at block ${e.blockNumber}: expected ${e.batchSize} Released, found ${nullifiers.length}`
      );
    }
    batches.push({
      nullifiers,
      spent_root_old: e.spentRootOld,
      spent_root_new: e.spentRootNew,
    });
  }
  return { batches };
}

module.exports = { hex32, normalizeEvent, groupBatches };
