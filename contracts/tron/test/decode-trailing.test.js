// The SP1 guest commits publicValues = public_values_abi(288) || digest(32) = 320
// bytes, and the vault does abi.decode(publicValues, (PublicValues)). This checks
// abi.decode reads the 288-byte struct and tolerates the trailing 32 bytes — the
// precondition for fulfillBatch to accept a real SP1-committed publicValues blob.
const { expect } = require("chai");
const { ethers } = require("hardhat");

describe("PublicValues abi.decode tolerates trailing digest", () => {
  it("decodes a 320-byte (struct||digest) publicValues", async () => {
    const H = await ethers.getContractFactory("EncodingHarness");
    const h = await H.deploy();
    await h.waitForDeployment();
    const coder = ethers.AbiCoder.defaultAbiCoder();
    const pv = [
      "0x" + "11".repeat(32), "0x" + "22".repeat(32), "0x" + "33".repeat(32),
      "0x" + "44".repeat(32), "0x" + "55".repeat(32), "0x" + "66".repeat(32),
      "0x" + "77".repeat(32), 1, 1000000n,
    ];
    const abi288 = coder.encode(
      ["tuple(bytes32,bytes32,bytes32,bytes32,bytes32,bytes32,bytes32,uint32,uint256)"],
      [pv],
    );
    expect(ethers.dataLength(abi288)).to.equal(288);
    const withDigest = ethers.concat([abi288, "0x" + "ab".repeat(32)]); // 320 bytes
    expect(ethers.dataLength(withDigest)).to.equal(320);

    const d = await h.decodePublicValues(withDigest);
    expect(d.domainTag).to.equal("0x" + "11".repeat(32));
    expect(d.totalAmount).to.equal(1000000n);
    expect(d.batchSize).to.equal(1n);
  });
});
