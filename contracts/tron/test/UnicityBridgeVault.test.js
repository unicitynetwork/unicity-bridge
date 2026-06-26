// Behavior + property tests for the fresh ZK_BACK3 vault, against a MOCK proof
// verifier (M2): the proof is assumed valid, so these exercise the settlement
// logic — root transition, lock-ref binding, fee/deadline, value conservation,
// replay guard, reverts, reentrancy — exactly per 00 §7/§9 and 01.
const { expect } = require("chai");
const { ethers } = require("hardhat");

const abi = ethers.AbiCoder.defaultAbiCoder();
const VKEY = "0x" + "11".repeat(32);
const TOKEN_TYPE = "0x" + "aa".repeat(32);
const COIN_ID = "0x" + "bb".repeat(32);
const TRUST_BASE = ethers.id("trust-base-1");
const NEW_ROOT = ethers.id("spent-root-1");
const SRC_CHAIN = 728126428n;

// JS re-derivations of the keccak/ABI roots the vault recomputes (00 §7). Kept
// independent of the Solidity library so the two agree by construction.
function returnRoot(leaves) {
  let buf = "0x";
  for (const l of leaves) {
    buf = ethers.concat([
      buf,
      abi.encode(
        ["bytes32", "address", "uint256", "address", "uint256", "uint64"],
        [l[0], l[1], l[2], l[3], l[4], l[5]]
      ),
    ]);
  }
  return ethers.keccak256(buf);
}
function lockRefRoot(refs) {
  let buf = "0x";
  for (const r of refs) {
    buf = ethers.concat([buf, abi.encode(["uint256", "bytes32"], [r[0], r[1]])]);
  }
  return ethers.keccak256(buf);
}
function encodePV(pv) {
  return abi.encode(
    ["tuple(bytes32,bytes32,bytes32,bytes32,bytes32,bytes32,bytes32,uint32,uint256)"],
    [
      [
        pv.domainTag,
        pv.configHash,
        pv.trustBaseHash,
        pv.spentRootOld,
        pv.spentRootNew,
        pv.returnRoot,
        pv.lockRefRoot,
        pv.batchSize,
        pv.totalAmount,
      ],
    ]
  );
}

async function deployVault({ assetName = "MockTRC20", verifierName = "MockProofVerifier" } = {}) {
  const [deployer, admin, alice, bob, feeRcpt] = await ethers.getSigners();

  const Asset = await ethers.getContractFactory(assetName);
  const asset = await Asset.connect(deployer).deploy();
  await asset.waitForDeployment();

  const Verifier = await ethers.getContractFactory(verifierName);
  const verifier = await Verifier.connect(deployer).deploy();
  await verifier.waitForDeployment();

  // Predict the vault address so config.vault binds the live deployment.
  const nonce = await ethers.provider.getTransactionCount(deployer.address);
  const predicted = ethers.getCreateAddress({ from: deployer.address, nonce });

  const cfg = {
    sourceChainId: SRC_CHAIN,
    vault: predicted,
    asset: await asset.getAddress(),
    tokenType: TOKEN_TYPE,
    coinId: COIN_ID,
    reasonTag: 39050n,
    lockDomain: "0x" + "cc".repeat(32),
    nullifierDomain: "0x" + "dd".repeat(32),
  };

  const Vault = await ethers.getContractFactory("UnicityBridgeVault");
  const vault = await Vault.connect(deployer).deploy(
    cfg,
    await verifier.getAddress(),
    VKEY,
    admin.address
  );
  await vault.waitForDeployment();
  expect(await vault.getAddress()).to.equal(predicted);

  await vault.connect(admin).setTrustBaseAllowed(TRUST_BASE, true);
  return { deployer, admin, alice, bob, feeRcpt, asset, verifier, vault };
}

// Lock `amount` so the vault both holds custody (for payouts) and stores a
// lockDigest we can reference in a batch. Returns the new nonce.
async function lockDeposit(vault, asset, user, amount, tokenId) {
  await asset.mint(user.address, amount);
  await asset.connect(user).approve(await vault.getAddress(), amount);
  const nonce = await vault.nextNonce();
  await vault.connect(user).lock(amount, tokenId, ethers.id("recip-commit:" + tokenId));
  return nonce;
}

// Assemble a self-consistent single-leaf batch (the MOCK verifier accepts any
// proof, so we just need PV to match the vault's stored config/root).
async function buildBatch(vault, { leaves, lockRefs, spentRootOld, spentRootNew = NEW_ROOT }) {
  const total = leaves.reduce((s, l) => s + l[2], 0n);
  const pv = {
    domainTag: await vault.DOMAIN_TAG(),
    configHash: await vault.CONFIG_HASH(),
    trustBaseHash: TRUST_BASE,
    spentRootOld: spentRootOld ?? (await vault.spentRoot()),
    spentRootNew,
    returnRoot: returnRoot(leaves),
    lockRefRoot: lockRefRoot(lockRefs),
    batchSize: leaves.length,
    totalAmount: total,
  };
  return { encoded: encodePV(pv), pv };
}

describe("UnicityBridgeVault — constructor & lock (bridge-in)", () => {
  it("derives CONFIG_HASH/DOMAIN_TAG/ASSET and inits spentRoot to EMPTY_TREE_ROOT", async () => {
    const { vault, asset } = await deployVault();
    expect(await vault.ASSET()).to.equal(await asset.getAddress());
    expect(await vault.spentRoot()).to.equal(await vault.EMPTY_TREE_ROOT());
    expect(await vault.DOMAIN_TAG()).to.equal(
      ethers.keccak256(ethers.toUtf8Bytes("unicity-bridge-return:v1"))
    );
    expect(await vault.CONFIG_HASH()).to.not.equal(ethers.ZeroHash);
  });

  it("rejects a config whose vault != address(this)", async () => {
    const [deployer, admin] = await ethers.getSigners();
    const Asset = await ethers.getContractFactory("MockTRC20");
    const asset = await Asset.deploy();
    const Verifier = await ethers.getContractFactory("MockProofVerifier");
    const verifier = await Verifier.deploy();
    const Vault = await ethers.getContractFactory("UnicityBridgeVault");
    const badCfg = {
      sourceChainId: SRC_CHAIN,
      vault: deployer.address, // wrong
      asset: await asset.getAddress(),
      tokenType: TOKEN_TYPE,
      coinId: COIN_ID,
      reasonTag: 39050n,
      lockDomain: "0x" + "cc".repeat(32),
      nullifierDomain: "0x" + "dd".repeat(32),
    };
    await expect(
      Vault.deploy(badCfg, await verifier.getAddress(), VKEY, admin.address)
    ).to.be.revertedWith("vault: config vault mismatch");
  });

  it("lock() stores lockDigest, escrows the asset, guards tokenId, increments nonce", async () => {
    const { vault, asset, alice } = await deployVault();
    const amount = 1_000_000n;
    const tokenId = ethers.id("token-1");
    await asset.mint(alice.address, amount);
    await asset.connect(alice).approve(await vault.getAddress(), amount);
    const commit = ethers.id("commit-1");

    await expect(vault.connect(alice).lock(amount, tokenId, commit))
      .to.emit(vault, "Lock")
      .withArgs(0n, alice.address, amount, tokenId, commit);

    expect(await asset.balanceOf(await vault.getAddress())).to.equal(amount);
    expect(await vault.nextNonce()).to.equal(1n);
    expect(await vault.tokenIdUsed(tokenId)).to.equal(true);

    // lockDigest equals the library derivation (00 §3)
    const Harness = await ethers.getContractFactory("EncodingHarness");
    const h = await Harness.deploy();
    const expected = await h.lockDigest(
      SRC_CHAIN,
      await vault.getAddress(),
      0n,
      await asset.getAddress(),
      TOKEN_TYPE,
      COIN_ID,
      amount,
      tokenId,
      commit
    );
    expect(await vault.lockDigest(0n)).to.equal(expected);

    await expect(vault.connect(alice).lock(amount, tokenId, commit)).to.be.revertedWith(
      "vault: tokenId already locked"
    );
  });

  it("lock() rejects zero amount / tokenId / recipient", async () => {
    const { vault, asset, alice } = await deployVault();
    await asset.mint(alice.address, 10n);
    await asset.connect(alice).approve(await vault.getAddress(), 10n);
    await expect(vault.connect(alice).lock(0, ethers.id("t"), ethers.id("c"))).to.be.revertedWith(
      "vault: zero amount"
    );
    await expect(vault.connect(alice).lock(1, ethers.ZeroHash, ethers.id("c"))).to.be.revertedWith(
      "vault: zero tokenId"
    );
    await expect(vault.connect(alice).lock(1, ethers.id("t"), ethers.ZeroHash)).to.be.revertedWith(
      "vault: zero recipient"
    );
  });
});

describe("UnicityBridgeVault — fulfillBatch (bridge-back, mock proof)", () => {
  it("settles a single leaf: pays principal + fee within deadline, advances root, emits", async () => {
    const { vault, asset, alice, bob, feeRcpt } = await deployVault();
    const locked = 1_000_000n;
    const nonce = await lockDeposit(vault, asset, alice, locked, ethers.id("dep-1"));
    const digest = await vault.lockDigest(nonce);

    const amount = 400_000n;
    const fee = 1_000n;
    const future = 1_900_000_000n; // > block.timestamp
    const leaf = [ethers.id("nul-1"), bob.address, amount, feeRcpt.address, fee, future];
    const { encoded } = await buildBatch(vault, { leaves: [leaf], lockRefs: [[nonce, digest]] });

    await expect(vault.fulfillBatch(encoded, "0x", [leaf], [[nonce, digest]]))
      .to.emit(vault, "Released")
      .withArgs(ethers.id("nul-1"), bob.address, amount, feeRcpt.address, fee, future)
      .and.to.emit(vault, "BatchFulfilled")
      .withArgs(ethers.ZeroHash, NEW_ROOT, 1, amount);

    expect(await asset.balanceOf(bob.address)).to.equal(amount - fee);
    expect(await asset.balanceOf(feeRcpt.address)).to.equal(fee);
    expect(await asset.balanceOf(await vault.getAddress())).to.equal(locked - amount);
    expect(await vault.spentRoot()).to.equal(NEW_ROOT);
  });

  it("deadline gates the fee only: past deadline pays full principal, no fee", async () => {
    const { vault, asset, alice, bob, feeRcpt } = await deployVault();
    const nonce = await lockDeposit(vault, asset, alice, 1_000_000n, ethers.id("dep-1"));
    const digest = await vault.lockDigest(nonce);

    const amount = 400_000n;
    const leaf = [ethers.id("nul-1"), bob.address, amount, feeRcpt.address, 1_000n, 1n]; // deadline in the past
    const { encoded } = await buildBatch(vault, { leaves: [leaf], lockRefs: [[nonce, digest]] });

    await expect(vault.fulfillBatch(encoded, "0x", [leaf], [[nonce, digest]]))
      .to.emit(vault, "Released")
      .withArgs(ethers.id("nul-1"), bob.address, amount, feeRcpt.address, 0n, 1n);
    expect(await asset.balanceOf(bob.address)).to.equal(amount); // full principal
    expect(await asset.balanceOf(feeRcpt.address)).to.equal(0n);
  });

  it("replay guard: re-submitting the settled batch reverts on stale root", async () => {
    const { vault, asset, alice, bob } = await deployVault();
    const nonce = await lockDeposit(vault, asset, alice, 1_000_000n, ethers.id("dep-1"));
    const digest = await vault.lockDigest(nonce);
    const leaf = [ethers.id("nul-1"), bob.address, 400_000n, ethers.ZeroAddress, 0n, 0n];
    const { encoded } = await buildBatch(vault, { leaves: [leaf], lockRefs: [[nonce, digest]] });

    await vault.fulfillBatch(encoded, "0x", [leaf], [[nonce, digest]]);
    await expect(vault.fulfillBatch(encoded, "0x", [leaf], [[nonce, digest]])).to.be.revertedWith(
      "vault: stale root"
    );
  });

  it("works with a no-return TRC20 (USDT-style)", async () => {
    const { vault, asset, alice, bob } = await deployVault({ assetName: "MockNoReturnTRC20" });
    const nonce = await lockDeposit(vault, asset, alice, 1_000_000n, ethers.id("dep-1"));
    const digest = await vault.lockDigest(nonce);
    const leaf = [ethers.id("nul-1"), bob.address, 400_000n, ethers.ZeroAddress, 0n, 0n];
    const { encoded } = await buildBatch(vault, { leaves: [leaf], lockRefs: [[nonce, digest]] });
    await vault.fulfillBatch(encoded, "0x", [leaf], [[nonce, digest]]);
    expect(await asset.balanceOf(bob.address)).to.equal(400_000n);
  });

  it("reentrancy guard trips when the asset reenters a guarded function", async () => {
    const { vault, asset, alice, bob } = await deployVault({ assetName: "MockReentrantTRC20" });
    await asset.setVault(await vault.getAddress());
    const nonce = await lockDeposit(vault, asset, alice, 1_000_000n, ethers.id("dep-1"));
    const digest = await vault.lockDigest(nonce);
    const leaf = [ethers.id("nul-1"), bob.address, 400_000n, ethers.ZeroAddress, 0n, 0n];
    const { encoded } = await buildBatch(vault, { leaves: [leaf], lockRefs: [[nonce, digest]] });

    await vault.fulfillBatch(encoded, "0x", [leaf], [[nonce, digest]]);
    expect(await asset.reentryBlocked()).to.equal(true);
  });
});

describe("UnicityBridgeVault — fulfillBatch reverts", () => {
  let ctx;
  let nonce, digest, leaf, lockRefs;

  beforeEach(async () => {
    ctx = await deployVault();
    nonce = await lockDeposit(ctx.vault, ctx.asset, ctx.alice, 1_000_000n, ethers.id("dep-1"));
    digest = await ctx.vault.lockDigest(nonce);
    leaf = [ethers.id("nul-1"), ctx.bob.address, 400_000n, ctx.feeRcpt.address, 1_000n, 1_900_000_000n];
    lockRefs = [[nonce, digest]];
  });

  async function fulfillWith(pvMutator, leavesArg = [leaf], refsArg = lockRefs) {
    const base = await buildBatch(ctx.vault, { leaves: [leaf], lockRefs });
    // start from a consistent PV, then optionally corrupt one field
    const pv = pvMutator ? pvMutator({ ...basePV(base) }) : basePV(base);
    return ctx.vault.fulfillBatch(encodePV(pv), "0x", leavesArg, refsArg);
  }
  function basePV(base) {
    return base.pv;
  }

  it("rejects an invalid proof (verifier reverts)", async () => {
    const bad = await deployVault({ verifierName: "RevertingProofVerifier" });
    const n = await lockDeposit(bad.vault, bad.asset, bad.alice, 1_000_000n, ethers.id("d"));
    const d = await bad.vault.lockDigest(n);
    const lf = [ethers.id("n"), bad.bob.address, 1n, ethers.ZeroAddress, 0n, 0n];
    const { encoded } = await buildBatch(bad.vault, { leaves: [lf], lockRefs: [[n, d]] });
    await expect(bad.vault.fulfillBatch(encoded, "0x", [lf], [[n, d]])).to.be.revertedWith(
      "mock: proof rejected"
    );
  });

  it("rejects a wrong domainTag", async () => {
    await expect(fulfillWith((pv) => ((pv.domainTag = ethers.id("x")), pv))).to.be.revertedWith(
      "vault: bad domain"
    );
  });

  it("rejects a wrong configHash", async () => {
    await expect(fulfillWith((pv) => ((pv.configHash = ethers.id("x")), pv))).to.be.revertedWith(
      "vault: bad config"
    );
  });

  it("rejects a non-allow-listed trust base", async () => {
    await expect(fulfillWith((pv) => ((pv.trustBaseHash = ethers.id("x")), pv))).to.be.revertedWith(
      "vault: trust base not allowed"
    );
  });

  it("rejects a stale spentRootOld", async () => {
    await expect(fulfillWith((pv) => ((pv.spentRootOld = ethers.id("x")), pv))).to.be.revertedWith(
      "vault: stale root"
    );
  });

  it("rejects a batchSize that disagrees with leaves.length", async () => {
    await expect(fulfillWith((pv) => ((pv.batchSize = 2), pv))).to.be.revertedWith(
      "vault: batch size mismatch"
    );
  });

  it("rejects a returnRoot that doesn't match the leaves", async () => {
    await expect(fulfillWith((pv) => ((pv.returnRoot = ethers.id("x")), pv))).to.be.revertedWith(
      "vault: return root mismatch"
    );
  });

  it("rejects a lockRefRoot that doesn't match the refs", async () => {
    await expect(fulfillWith((pv) => ((pv.lockRefRoot = ethers.id("x")), pv))).to.be.revertedWith(
      "vault: lock ref root mismatch"
    );
  });

  it("rejects a total that disagrees with the leaf sum", async () => {
    await expect(fulfillWith((pv) => ((pv.totalAmount = 1n), pv))).to.be.revertedWith(
      "vault: total amount mismatch"
    );
  });

  it("rejects a lock ref whose digest isn't the stored one", async () => {
    const refs = [[nonce, ethers.id("wrong-digest")]];
    const { encoded } = await buildBatch(ctx.vault, { leaves: [leaf], lockRefs: refs });
    await expect(ctx.vault.fulfillBatch(encoded, "0x", [leaf], refs)).to.be.revertedWith(
      "vault: lock digest mismatch"
    );
  });

  it("rejects unsorted / duplicate lock refs", async () => {
    const n2 = await lockDeposit(ctx.vault, ctx.asset, ctx.alice, 1_000n, ethers.id("dep-2"));
    const d2 = await ctx.vault.lockDigest(n2);
    const unsorted = [
      [n2, d2],
      [nonce, digest],
    ]; // n2 (1) before nonce (0) => not strictly increasing
    const { encoded } = await buildBatch(ctx.vault, { leaves: [leaf], lockRefs: unsorted });
    await expect(ctx.vault.fulfillBatch(encoded, "0x", [leaf], unsorted)).to.be.revertedWith(
      "vault: lock refs unsorted"
    );

    const dup = [
      [nonce, digest],
      [nonce, digest],
    ];
    const b2 = await buildBatch(ctx.vault, { leaves: [leaf], lockRefs: dup });
    await expect(ctx.vault.fulfillBatch(b2.encoded, "0x", [leaf], dup)).to.be.revertedWith(
      "vault: lock refs unsorted"
    );
  });

  it("rejects a leaf whose fee exceeds its amount", async () => {
    const badLeaf = [ethers.id("nul-1"), ctx.bob.address, 100n, ctx.feeRcpt.address, 101n, 1_900_000_000n];
    const { encoded } = await buildBatch(ctx.vault, { leaves: [badLeaf], lockRefs });
    await expect(ctx.vault.fulfillBatch(encoded, "0x", [badLeaf], lockRefs)).to.be.revertedWith(
      "vault: fee exceeds amount"
    );
  });

  it("rejects an empty batch", async () => {
    const pv = {
      domainTag: await ctx.vault.DOMAIN_TAG(),
      configHash: await ctx.vault.CONFIG_HASH(),
      trustBaseHash: TRUST_BASE,
      spentRootOld: await ctx.vault.spentRoot(),
      spentRootNew: NEW_ROOT,
      returnRoot: returnRoot([]),
      lockRefRoot: lockRefRoot(lockRefs),
      batchSize: 0,
      totalAmount: 0n,
    };
    await expect(ctx.vault.fulfillBatch(encodePV(pv), "0x", [], lockRefs)).to.be.revertedWith(
      "vault: empty batch"
    );
  });
});

describe("UnicityBridgeVault — admin", () => {
  it("only admin manages the trust-base allow-list", async () => {
    const { vault, admin, alice } = await deployVault();
    await expect(vault.connect(alice).setTrustBaseAllowed(ethers.id("x"), true)).to.be.revertedWith(
      "vault: not admin"
    );
    await expect(vault.connect(admin).setTrustBaseAllowed(ethers.id("x"), true))
      .to.emit(vault, "TrustBaseAllowedUpdated")
      .withArgs(ethers.id("x"), true);
    expect(await vault.trustBaseAllowed(ethers.id("x"))).to.equal(true);
  });

  it("transfers admin", async () => {
    const { vault, admin, alice } = await deployVault();
    await expect(vault.connect(admin).transferAdmin(alice.address))
      .to.emit(vault, "AdminTransferred")
      .withArgs(admin.address, alice.address);
    expect(await vault.admin()).to.equal(alice.address);
  });
});
