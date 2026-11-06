const { expect } = require("chai");
const hre = require("hardhat");
const { hex32, normalizeEvent, groupBatches, releaseLogged } = require("../scripts/relayer-lib.js");
const { lockRejections, confirmReleases } = require("../scripts/relayer.js");

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

  it("names the transaction each batch was settled in, and refuses a Released from another", () => {
    const events = [
      { kind: "BatchFulfilled", blockNumber: 100, eventIndex: 0, txId: "aa", spentRootOld: R(0), spentRootNew: R(11), batchSize: 1 },
      { kind: "Released", blockNumber: 100, eventIndex: 1, txId: "aa", nullifier: R(0xa1) },
    ];
    expect(groupBatches(events).batches).to.deep.equal([
      { nullifiers: [R(0xa1)], spent_root_old: R(0), spent_root_new: R(11), txid: "aa" },
    ]);
    const crossed = [events[0], { ...events[1], txId: "bb" }];
    expect(() => groupBatches(crossed)).to.throw(/from transaction bb, not the batch's aa/);
  });

  it("throws when a batch's Released events are missing (truncated log)", () => {
    const events = [
      { kind: "BatchFulfilled", blockNumber: 100, eventIndex: 0, spentRootOld: R(0), spentRootNew: R(11), batchSize: 3 },
      { kind: "Released", blockNumber: 100, eventIndex: 1, nullifier: R(0xa1) },
    ];
    expect(() => groupBatches(events)).to.throw(/expected 3 Released, found 1/);
  });
});

describe("S4 relayer — simulate checks the lock refs a leaf draws on", () => {
  const ethers = hre.ethers;
  const AMOUNT = 1_000_000n;

  // The vault through the TronWeb contract shape (`method(...args).call()`).
  function tronShaped(vault) {
    return { lockDigest: (nonce) => ({ call: () => vault.lockDigest(nonce) }) };
  }

  async function deployVault() {
    const [deployer, recipient] = await ethers.getSigners();
    const verifier = await (await ethers.getContractFactory("MockProofVerifier")).deploy();
    const asset = await (await ethers.getContractFactory("MockTRC20")).deploy();
    const cfg = [31337, deployer.address, await asset.getAddress(), ethers.id("type"), ethers.id("coin"), 39048, ethers.id("lock"), ethers.id("nullifier")];
    const vault = await (await ethers.getContractFactory("UnicityBridgeVault")).deploy(cfg, await verifier.getAddress(), ethers.ZeroHash, deployer.address, false);
    await asset.mint(deployer.address, AMOUNT);
    await asset.approve(await vault.getAddress(), AMOUNT);
    const nonce = await vault.nextNonce();
    await vault.lock(AMOUNT, ethers.id("token"), ethers.id("recipient"));
    return { recipient, vault, lock: { nonce, digest: await vault.lockDigest(nonce) } };
  }

  function leaf(recipient, label, lockRefs) {
    return { nullifier: ethers.id(label), recipient: recipient.address, amount: AMOUNT.toString(), feeRecipient: ethers.ZeroAddress, feeAmount: "0", deadline: "0", lockRefs };
  }

  it("rejects, as a lock rejection, a leaf whose lock ref is not the digest the vault stored", async () => {
    const { recipient, vault, lock } = await deployVault();
    const fine = leaf(recipient, "t1", [{ nonce: "0x" + lock.nonce.toString(16), digest: lock.digest }]);
    const forged = leaf(recipient, "t2", [{ nonce: "0x3e7", digest: ethers.id("no such lock") }]);
    const wrongDigest = leaf(recipient, "t3", [{ nonce: "0x" + lock.nonce.toString(16), digest: ethers.id("other") }]);
    const rejected = await lockRejections(tronShaped(vault), [fine, forged, wrongDigest]);
    expect(rejected.map((r) => [r.nullifier, r.kind])).to.deep.equal([
      [forged.nullifier, "lock"],
      [wrongDigest.nullifier, "lock"],
    ]);
    expect(rejected[0].reason).to.include("lock 999");
  });

  it("matches a digest the vault returns without a 0x prefix", async () => {
    const { recipient, vault, lock } = await deployVault();
    const bare = { lockDigest: (nonce) => ({ call: async () => (await vault.lockDigest(nonce)).slice(2).toUpperCase() }) };
    const fine = leaf(recipient, "t4", [{ nonce: lock.nonce.toString(), digest: lock.digest }]);
    expect(await lockRejections(bare, [fine])).to.deep.equal([]);
  });
});

describe("S4 relayer — confirming a release on a second node", () => {
  const ethers = hre.ethers;
  const TOPIC = ethers.id("Released(bytes32,address,uint256,address,uint256,uint64)").slice(2);
  const VAULT = "41" + "a6".repeat(20);
  const N1 = R(0xa1);

  // A solidity node's `gettransactioninfobyid` for a successful settlement of `nullifier`.
  function info(nullifier, overrides = {}) {
    return {
      id: "cc".repeat(32),
      blockNumber: 7,
      receipt: { result: "SUCCESS" },
      log: [{ address: "a6".repeat(20), topics: [TOPIC, nullifier.slice(2), "00".repeat(32)], data: "" }],
      ...overrides,
    };
  }

  it("matches the vault's Released log in EVM and Tron shapes", () => {
    const evm = [{ address: "0x" + "A6".repeat(20), topics: ["0x" + TOPIC, N1] }];
    expect(releaseLogged(evm, { vault: VAULT, topic: "0x" + TOPIC, nullifier: N1 })).to.equal(true);
    expect(releaseLogged(info(N1).log, { vault: VAULT, topic: TOPIC, nullifier: N1 })).to.equal(true);
    expect(releaseLogged(info(N1).log, { vault: VAULT, topic: TOPIC, nullifier: R(0xa2) })).to.equal(false);
    expect(releaseLogged(info(N1).log, { vault: "41" + "b7".repeat(20), topic: TOPIC, nullifier: N1 })).to.equal(false);
    expect(releaseLogged(undefined, { vault: VAULT, topic: TOPIC, nullifier: N1 })).to.equal(false);
  });

  it("confirms only a solidified, successful transaction with the vault's Released log for the nullifier", async () => {
    const answers = {
      ["11".repeat(32)]: info(N1),
      ["22".repeat(32)]: {},
      ["33".repeat(32)]: info(N1, { receipt: { result: "REVERT" }, log: undefined }),
      ["44".repeat(32)]: info(R(0xa2)),
    };
    const asked = [];
    const transactionInfo = async (txid) => (asked.push(txid), answers[txid]);
    const releases = Object.keys(answers).map((txid, i) => ({ nullifier: N1, txid: i === 0 ? "0x" + txid : txid }));
    const { confirmed, unconfirmed } = await confirmReleases(transactionInfo, VAULT, releases);
    expect(confirmed).to.deep.equal([releases[0]]);
    expect(unconfirmed.map((u) => u.reason)).to.deep.equal([
      "the confirmation node has no solidified transaction by that id",
      "the transaction did not succeed (REVERT)",
      "the transaction emitted no Released event for the nullifier from the vault",
    ]);
    expect(asked[0]).to.equal("11".repeat(32));
  });
});
