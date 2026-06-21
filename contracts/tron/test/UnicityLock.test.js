const { expect } = require("chai");
const { ethers } = require("hardhat");

const TOKEN_ID = "0x" + "11".repeat(32);
const RECIP = "0x" + "22".repeat(32);
const AMOUNT = 1_000_000n; // 1 USDT (6 decimals)

async function deploy() {
  const [admin, user, other, verifier] = await ethers.getSigners();
  const Token = await ethers.getContractFactory("MockTRC20");
  const usdt = await Token.deploy();
  const Lock = await ethers.getContractFactory("UnicityLock");
  const lock = await Lock.deploy(await usdt.getAddress(), admin.address);

  await usdt.mint(user.address, 10n * AMOUNT);
  await usdt.connect(user).approve(await lock.getAddress(), 10n * AMOUNT);
  return { admin, user, other, verifier, usdt, lock };
}

describe("UnicityLock", () => {
  it("locks USDT, emits Lock with the committed fields, and escrows the tokens", async () => {
    const { user, usdt, lock } = await deploy();
    const lockAddr = await lock.getAddress();

    await expect(lock.connect(user).lock(AMOUNT, TOKEN_ID, RECIP))
      .to.emit(lock, "Lock")
      .withArgs(0n, user.address, AMOUNT, TOKEN_ID, RECIP);

    expect(await usdt.balanceOf(lockAddr)).to.equal(AMOUNT);
    const rec = await lock.locks(0n);
    expect(rec.from).to.equal(user.address);
    expect(rec.amount).to.equal(AMOUNT);
    expect(rec.unicityTokenId).to.equal(TOKEN_ID);
    expect(rec.recipientCommitment).to.equal(RECIP);
    expect(rec.withdrawn).to.equal(false);
    expect(await lock.tokenIdUsed(TOKEN_ID)).to.equal(true);
  });

  it("assigns incrementing nonces", async () => {
    const { user, lock } = await deploy();
    const id2 = "0x" + "33".repeat(32);
    await lock.connect(user).lock(AMOUNT, TOKEN_ID, RECIP);
    await expect(lock.connect(user).lock(AMOUNT, id2, RECIP))
      .to.emit(lock, "Lock")
      .withArgs(1n, user.address, AMOUNT, id2, RECIP);
    expect(await lock.nextNonce()).to.equal(2n);
  });

  it("reverts a second lock for the same tokenId (defence-in-depth anti-replay)", async () => {
    const { user, lock } = await deploy();
    await lock.connect(user).lock(AMOUNT, TOKEN_ID, RECIP);
    await expect(
      lock.connect(user).lock(AMOUNT, TOKEN_ID, RECIP)
    ).to.be.revertedWith("UnicityLock: tokenId already locked");
  });

  it("rejects zero amount / tokenId / recipient", async () => {
    const { user, lock } = await deploy();
    await expect(lock.connect(user).lock(0, TOKEN_ID, RECIP)).to.be.revertedWith("UnicityLock: zero amount");
    await expect(lock.connect(user).lock(AMOUNT, ethers.ZeroHash, RECIP)).to.be.revertedWith("UnicityLock: zero tokenId");
    await expect(lock.connect(user).lock(AMOUNT, TOKEN_ID, ethers.ZeroHash)).to.be.revertedWith("UnicityLock: zero recipient");
  });

  it("reverts lock without allowance", async () => {
    const { other, lock, usdt } = await deploy();
    await usdt.mint(other.address, AMOUNT); // balance but no approval
    // The safe-transfer wrapper turns the failed token pull into its own revert.
    await expect(
      lock.connect(other).lock(AMOUNT, TOKEN_ID, RECIP)
    ).to.be.revertedWith("UnicityLock: transferFrom failed");
  });

  describe("bridge-back (unlock)", () => {
    it("is disabled until a verifier is set", async () => {
      const { user, verifier, lock } = await deploy();
      await lock.connect(user).lock(AMOUNT, TOKEN_ID, RECIP);
      await expect(
        lock.connect(verifier).unlock(0n, user.address, AMOUNT)
      ).to.be.revertedWith("UnicityLock: bridge-back disabled");
    });

    it("only admin can set the verifier", async () => {
      const { other, verifier, lock } = await deploy();
      await expect(
        lock.connect(other).setBurnProofVerifier(verifier.address)
      ).to.be.revertedWith("UnicityLock: not admin");
    });

    it("releases to the recipient once and only via the verifier", async () => {
      const { admin, user, other, verifier, usdt, lock } = await deploy();
      await lock.connect(user).lock(AMOUNT, TOKEN_ID, RECIP);
      await lock.connect(admin).setBurnProofVerifier(verifier.address);

      // non-verifier cannot settle
      await expect(
        lock.connect(other).unlock(0n, other.address, AMOUNT)
      ).to.be.revertedWith("UnicityLock: not verifier");

      await expect(lock.connect(verifier).unlock(0n, other.address, AMOUNT))
        .to.emit(lock, "Unlock")
        .withArgs(0n, other.address, AMOUNT);
      expect(await usdt.balanceOf(other.address)).to.equal(AMOUNT);

      // no double withdraw
      await expect(
        lock.connect(verifier).unlock(0n, other.address, AMOUNT)
      ).to.be.revertedWith("UnicityLock: already withdrawn");
    });

    it("rejects amount mismatch and unknown nonce", async () => {
      const { admin, user, other, verifier, lock } = await deploy();
      await lock.connect(user).lock(AMOUNT, TOKEN_ID, RECIP);
      await lock.connect(admin).setBurnProofVerifier(verifier.address);
      await expect(
        lock.connect(verifier).unlock(0n, other.address, AMOUNT + 1n)
      ).to.be.revertedWith("UnicityLock: amount mismatch");
      await expect(
        lock.connect(verifier).unlock(99n, other.address, AMOUNT)
      ).to.be.revertedWith("UnicityLock: unknown nonce");
    });
  });
});
