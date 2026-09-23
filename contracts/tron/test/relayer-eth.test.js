const { expect } = require("chai");
const hre = require("hardhat");
const { settle, settledLog, simulate } = require("../scripts/relayer-eth.js");

const TRUST_BASE_HASH = "0x72a67260a9ce50ccbd88c889334042bda509115f85ec352a5e50d8bf90c358c0";
const LOCK_DOMAIN = "0x158b847f78b3910a5f5f42820de61abba1bf5ae1fbb29dabfba09118f393f932";
const NULLIFIER_DOMAIN = "0xd4530e4ea58fc8e38f84506e62b421476c3eeec70f4cbebefc32688a510e2d5d";
const AMOUNT = 1_000_000n;
const ethers = hre.ethers;

async function deployBridge(pullPayments) {
  const [deployer, recipient] = await ethers.getSigners();
  const verifier = await (await ethers.getContractFactory("MockProofVerifier")).deploy();
  const asset = await (await ethers.getContractFactory("MockTRC20")).deploy();
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
});
