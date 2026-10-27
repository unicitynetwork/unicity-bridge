const { expect } = require("chai");
const hre = require("hardhat");
const { settle, settledLog, simulate } = require("../scripts/relayer-eth.js");

const TRUST_BASE_HASH = "0x72a67260a9ce50ccbd88c889334042bda509115f85ec352a5e50d8bf90c358c0";
const LOCK_DOMAIN = "0x158b847f78b3910a5f5f42820de61abba1bf5ae1fbb29dabfba09118f393f932";
const NULLIFIER_DOMAIN = "0xd4530e4ea58fc8e38f84506e62b421476c3eeec70f4cbebefc32688a510e2d5d";
const AMOUNT = 1_000_000n;
const ethers = hre.ethers;

async function deployBridge(pullPayments, assetName = "MockTRC20") {
  const [deployer, recipient] = await ethers.getSigners();
  const verifier = await (await ethers.getContractFactory("MockProofVerifier")).deploy();
  const asset = await (await ethers.getContractFactory(assetName)).deploy();
  const cfg = [31337, deployer.address, await asset.getAddress(), ethers.id("type"), ethers.id("coin"), 39048, LOCK_DOMAIN, NULLIFIER_DOMAIN];
  const vault = await (await ethers.getContractFactory("UnicityBridgeVault")).deploy(cfg, await verifier.getAddress(), ethers.ZeroHash, deployer.address, pullPayments);
  await vault.setTrustBaseAllowed(TRUST_BASE_HASH, true);
  await asset.mint(deployer.address, AMOUNT * 10n);
  await asset.approve(await vault.getAddress(), AMOUNT * 10n);
  return { deployer, recipient, asset, vault };
}

async function lockOnce(vault, nonceLabel) {
  const nonce = await vault.nextNonce();
  await vault.lock(AMOUNT, ethers.id("token-" + nonceLabel), ethers.id("recipient"));
  return { nonce, digest: await vault.lockDigest(nonce) };
}

async function craftBundle(vault, recipient, lock, label) {
  const coder = ethers.AbiCoder.defaultAbiCoder();
  const leaf = { nullifier: ethers.id("nullifier-" + label), recipient: recipient.address, amount: AMOUNT.toString(), feeRecipient: ethers.ZeroAddress, feeAmount: "0", deadline: "0" };
  const lockRef = { nonce: lock.nonce.toString(), digest: lock.digest };
  const returnRoot = ethers.keccak256(coder.encode(["bytes32", "address", "uint256", "address", "uint256", "uint64"], [leaf.nullifier, leaf.recipient, AMOUNT, leaf.feeRecipient, 0n, 0n]));
  const lockRefRoot = ethers.keccak256(coder.encode(["uint256", "bytes32"], [lock.nonce, lock.digest]));
  const pv = [await vault.DOMAIN_TAG(), await vault.CONFIG_HASH(), TRUST_BASE_HASH, await vault.spentRoot(), ethers.id("root-" + label), returnRoot, lockRefRoot, 1, AMOUNT];
  return {
    batchId: "0x" + label,
    publicValues: coder.encode(["tuple(bytes32,bytes32,bytes32,bytes32,bytes32,bytes32,bytes32,uint32,uint256)"], [pv]),
    proofBytes: "0x00",
    leaves: [leaf],
    lockRefs: [lockRef],
    spentRootNew: pv[4],
  };
}

describe("relayer-eth against the vault", () => {
  it("settles a batch, reproduces it from the log, and refuses a stale root", async () => {
    const { recipient, asset, vault } = await deployBridge(false);
    const first = await craftBundle(vault, recipient, await lockOnce(vault, "a"), "a");

    const { txid } = await settle(vault, first);
    expect(txid).to.match(/^0x[0-9a-f]{64}$/);
    expect(await asset.balanceOf(recipient.address)).to.equal(AMOUNT);
    expect(await vault.spentRoot()).to.equal(first.spentRootNew);

    const log = await settledLog(vault, 0);
    expect(log.spent_root).to.equal(first.spentRootNew);
    expect(log.batches).to.deep.equal([{ nullifiers: [first.leaves[0].nullifier], spent_root_old: ethers.ZeroHash, spent_root_new: first.spentRootNew }]);

    const second = await craftBundle(vault, recipient, await lockOnce(vault, "b"), "b");
    await settle(vault, second);
    expect((await settledLog(vault, 0)).batches.map((b) => b.spent_root_new)).to.deep.equal([first.spentRootNew, second.spentRootNew]);

    let err;
    try {
      await settle(vault, second);
    } catch (e) {
      err = e;
    }
    expect(err.message).to.include("vault: stale root");
  });

  it("reports a settlement only once it has the confirmations asked for", async () => {
    const { recipient, vault } = await deployBridge(false);
    const bundle = await craftBundle(vault, recipient, await lockOnce(vault, "d"), "d");
    let reported = false;
    const settling = settle(vault, bundle, 3).then((r) => ((reported = true), r));
    await new Promise((r) => setTimeout(r, 500));
    expect(reported).to.equal(false);
    await hre.network.provider.send("hardhat_mine", ["0x2"]);
    const { txid } = await settling;
    expect(txid).to.match(/^0x[0-9a-f]{64}$/);
  });

  it("reads the settled log and root only up to the height the confirmations make final", async () => {
    const { recipient, vault } = await deployBridge(false);
    const bundle = await craftBundle(vault, recipient, await lockOnce(vault, "e"), "e");
    await settle(vault, bundle);
    const early = await settledLog(vault, 0, 3);
    expect(early.batches).to.deep.equal([]);
    expect(early.spent_root).to.equal(ethers.ZeroHash);
    await hre.network.provider.send("hardhat_mine", ["0x2"]);
    const final = await settledLog(vault, 0, 3);
    expect(final.batches.map((b) => b.spent_root_new)).to.deep.equal([bundle.spentRootNew]);
    expect(final.spent_root).to.equal(bundle.spentRootNew);
  });

  // Stands in for the vault, failing the first `failures` calls of each read settledLog makes.
  function flaky(vault, failures, code = "TIMEOUT") {
    const calls = { head: 0, logs: 0, root: 0 };
    const read = (key, call) => async (...args) => {
      calls[key] += 1;
      if (calls[key] <= failures) throw Object.assign(new Error(`request ${code}`), { code, shortMessage: `request ${code}` });
      return call(...args);
    };
    const provider = vault.runner.provider ?? vault.runner;
    return {
      calls,
      runner: { provider: { getBlockNumber: read("head", () => provider.getBlockNumber()) } },
      filters: vault.filters,
      queryFilter: read("logs", (...args) => vault.queryFilter(...args)),
      spentRoot: read("root", (...args) => vault.spentRoot(...args)),
    };
  }
  const NO_WAIT = { retries: 2, delayMs: 0 };

  it("reads the settled log through requests that time out once", async () => {
    const { recipient, vault } = await deployBridge(false);
    await settle(vault, await craftBundle(vault, recipient, await lockOnce(vault, "j"), "j"));
    const unreliable = flaky(vault, 1);
    expect(await settledLog(unreliable, 0, 1, NO_WAIT)).to.deep.equal(await settledLog(vault, 0));
    expect(unreliable.calls).to.deep.equal({ head: 2, logs: 3, root: 2 });
  });

  it("gives up on a read that keeps timing out, and at once on an error a repeat cannot clear", async () => {
    const { vault } = await deployBridge(false);
    const down = flaky(vault, Infinity);
    let err = await settledLog(down, 0, 1, NO_WAIT).catch((e) => e);
    expect(err.code).to.equal("TIMEOUT");
    expect(down.calls.head).to.equal(3);

    const broken = flaky(vault, Infinity, "CALL_EXCEPTION");
    err = await settledLog(broken, 0, 1, NO_WAIT).catch((e) => e);
    expect(err.code).to.equal("CALL_EXCEPTION");
    expect(broken.calls.head).to.equal(1);
  });

  it("names the leaves whose payout would revert", async () => {
    const { recipient, vault } = await deployBridge(false);
    const lock = await lockOnce(vault, "c");
    const fine = { nullifier: ethers.id("n1"), recipient: recipient.address, amount: AMOUNT.toString(), feeRecipient: ethers.ZeroAddress, feeAmount: "0", deadline: "0" };
    const tooMuch = { ...fine, nullifier: ethers.id("n2"), amount: (AMOUNT * 5n).toString() };
    const { rejected } = await simulate(vault, [fine, tooMuch]);
    expect(rejected.map((r) => r.nullifier)).to.deep.equal([tooMuch.nullifier]);
    expect(rejected[0].reason).to.include("insufficient balance");
    void lock;
  });

  it("rejects nothing for a pull-payment vault, whose settlement only credits what it owes", async () => {
    const { recipient, vault } = await deployBridge(true);
    const tooMuch = { nullifier: ethers.id("n3"), recipient: recipient.address, amount: (AMOUNT * 5n).toString(), feeRecipient: ethers.ZeroAddress, feeAmount: "0", deadline: "0" };
    const { rejected } = await simulate(vault, [tooMuch]);
    expect(rejected).to.deep.equal([]);
  });
});

describe("relayer-eth simulate checks the lock refs a leaf draws on", () => {
  function leaf(recipient, label, lockRefs) {
    return { nullifier: ethers.id(label), recipient: recipient.address, amount: AMOUNT.toString(), feeRecipient: ethers.ZeroAddress, feeAmount: "0", deadline: "0", lockRefs };
  }

  it("rejects a leaf whose lock ref is not the digest the vault stored", async () => {
    const { recipient, vault } = await deployBridge(false);
    const lock = await lockOnce(vault, "d");
    const fine = leaf(recipient, "n4", [{ nonce: "0x" + lock.nonce.toString(16), digest: lock.digest }]);
    const forged = leaf(recipient, "n5", [{ nonce: "0x3e7", digest: ethers.id("no such lock") }]);
    const { rejected } = await simulate(vault, [fine, forged]);
    expect(rejected.map((r) => r.nullifier)).to.deep.equal([forged.nullifier]);
    expect(rejected[0].reason).to.include("lock");
    expect(rejected[0].kind).to.equal("lock");
  });

  it("checks lock refs for a pull-payment vault too", async () => {
    const { recipient, vault } = await deployBridge(true);
    const forged = leaf(recipient, "n6", [{ nonce: "0x0", digest: ethers.id("no such lock") }]);
    const { rejected } = await simulate(vault, [forged]);
    expect(rejected.map((r) => r.nullifier)).to.deep.equal([forged.nullifier]);
    expect(rejected[0].kind).to.equal("lock");
  });

  it("tags a payout that would revert as a recipient rejection", async () => {
    const { recipient, vault } = await deployBridge(false);
    const lock = await lockOnce(vault, "f");
    const tooMuch = { ...leaf(recipient, "n7", [{ nonce: "0x" + lock.nonce.toString(16), digest: lock.digest }]), amount: (AMOUNT * 50n).toString() };
    const { rejected } = await simulate(vault, [tooMuch]);
    expect(rejected.map((r) => [r.nullifier, r.kind])).to.deep.equal([[tooMuch.nullifier, "recipient"]]);
  });

  it("reads lock digests at the confirmed height and tags a lock not yet that deep as unconfirmed", async () => {
    const { recipient, vault } = await deployBridge(true);
    const lock = await lockOnce(vault, "g");
    const fresh = leaf(recipient, "n8", [{ nonce: "0x" + lock.nonce.toString(16), digest: lock.digest }]);
    const early = await simulate(vault, [fresh], 3);
    expect(early.rejected.map((r) => [r.nullifier, r.kind])).to.deep.equal([[fresh.nullifier, "unconfirmed"]]);
    await hre.network.provider.send("hardhat_mine", ["0x2"]);
    expect((await simulate(vault, [fresh], 3)).rejected).to.deep.equal([]);
  });
});

describe("relayer-eth simulate mirrors what the vault will pay", () => {
  const FAR = (2n ** 40n).toString();
  function leaf(label, recipient, amount, feeRecipient = ethers.ZeroAddress, feeAmount = "0", deadline = "0") {
    return { nullifier: ethers.id(label), recipient, amount: amount.toString(), feeRecipient, feeAmount, deadline };
  }

  it("rejects a transfer that returns false without reverting", async () => {
    const { recipient, vault } = await deployBridge(false, "MockFalseReturnTRC20");
    await lockOnce(vault, "h");
    const { rejected } = await simulate(vault, [leaf("n10", recipient.address, AMOUNT)]);
    expect(rejected.map((r) => [r.reason, r.kind])).to.deep.equal([["transfer returned false", "recipient"]]);
  });

  it("simulates the fee transfer to the fee recipient", async () => {
    const { recipient, asset, vault } = await deployBridge(false, "MockBlocklistTRC20");
    await lockOnce(vault, "i");
    const feeRecipient = (await ethers.getSigners())[2];
    await asset.setBlocked(feeRecipient.address, true);
    const paid = leaf("n11", recipient.address, AMOUNT, feeRecipient.address, "1000", FAR);
    const { rejected } = await simulate(vault, [paid]);
    expect(rejected.map((r) => r.nullifier)).to.deep.equal([paid.nullifier]);
    expect(rejected[0].reason).to.include("blocked");
  });

  it("does not simulate a payout to the vault itself, which the vault treats as a no-op", async () => {
    const { asset, vault } = await deployBridge(false, "MockBlocklistTRC20");
    const vaultAddress = await vault.getAddress();
    await asset.setBlocked(vaultAddress, true);
    const { rejected } = await simulate(vault, [leaf("n12", vaultAddress, AMOUNT)]);
    expect(rejected).to.deep.equal([]);
  });
});
