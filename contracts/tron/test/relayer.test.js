const { expect } = require("chai");
const { hex32, normalizeEvent, groupBatches } = require("../scripts/relayer-lib.js");

const R = (n) => "0x" + n.toString(16).padStart(64, "0");

describe("S4 relayer — event grouping (relayer-lib)", () => {
  it("hex32 normalizes hex/decimal/number/bigint and strips prefixes", () => {
    expect(hex32("0x" + "ab".repeat(32))).to.equal("0x" + "ab".repeat(32));
    expect(hex32("ab".repeat(32))).to.equal("0x" + "ab".repeat(32));
    expect(hex32(255n)).to.equal(R(255));
    expect(hex32("255")).to.equal(R(255));
    // 41-prefixed (Tron) 21-byte value -> keep the trailing 32 (here just pads)
    expect(hex32("0x1")).to.equal(R(1));
  });

  it("normalizeEvent maps TronGrid named + positional results", () => {
    const named = normalizeEvent({
      event_name: "BatchFulfilled",
      block_number: 10,
      event_index: 0,
      result: { spentRootOld: R(0), spentRootNew: R(7), batchSize: "2" },
    });
    expect(named).to.include({ kind: "BatchFulfilled", spentRootOld: R(0), spentRootNew: R(7), batchSize: 2 });

    const positional = normalizeEvent({
      event_name: "Released",
      block_number: 10,
      event_index: 1,
      result: { 0: R(0xaa) },
    });
    expect(positional).to.include({ kind: "Released", nullifier: R(0xaa) });
  });

  it("groups BatchFulfilled + following Released into the S2 events shape", () => {
    // Contract emits BatchFulfilled then `batchSize` Released, per tx.
    const events = [
      { kind: "BatchFulfilled", blockNumber: 100, eventIndex: 0, spentRootOld: R(0), spentRootNew: R(11), batchSize: 2 },
      { kind: "Released", blockNumber: 100, eventIndex: 1, nullifier: R(0xa1) },
      { kind: "Released", blockNumber: 100, eventIndex: 2, nullifier: R(0xa2) },
      { kind: "BatchFulfilled", blockNumber: 200, eventIndex: 0, spentRootOld: R(11), spentRootNew: R(22), batchSize: 1 },
      { kind: "Released", blockNumber: 200, eventIndex: 1, nullifier: R(0xb1) },
    ];
    const { batches } = groupBatches(events);
    expect(batches).to.deep.equal([
      { nullifiers: [R(0xa1), R(0xa2)], spent_root_old: R(0), spent_root_new: R(11) },
      { nullifiers: [R(0xb1)], spent_root_old: R(11), spent_root_new: R(22) },
    ]);
  });

  it("sorts out-of-order events before grouping", () => {
    const events = [
      { kind: "Released", blockNumber: 100, eventIndex: 2, nullifier: R(0xa2) },
      { kind: "BatchFulfilled", blockNumber: 100, eventIndex: 0, spentRootOld: R(0), spentRootNew: R(11), batchSize: 2 },
      { kind: "Released", blockNumber: 100, eventIndex: 1, nullifier: R(0xa1) },
    ];
    const { batches } = groupBatches(events);
    expect(batches[0].nullifiers).to.deep.equal([R(0xa1), R(0xa2)]);
  });

  it("throws when a batch's Released events are missing (truncated log)", () => {
    const events = [
      { kind: "BatchFulfilled", blockNumber: 100, eventIndex: 0, spentRootOld: R(0), spentRootNew: R(11), batchSize: 3 },
      { kind: "Released", blockNumber: 100, eventIndex: 1, nullifier: R(0xa1) },
    ];
    expect(() => groupBatches(events)).to.throw(/expected 3 Released, found 1/);
  });
});
