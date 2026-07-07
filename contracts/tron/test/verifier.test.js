// Stage A verifier smoke (04-deployment.md): the real SP1 v6.1.0 Groth16 proof
// bundle the prover published (protocol/vectors/proof/b1-groth16.json) must verify
// against the *same* SP1 Groth16 verifier bytecode the vault will call on-chain.
// This is the prover<->source-chain join for M3, run locally before any deploy.
const { expect } = require("chai");
const { ethers } = require("hardhat");
const fs = require("fs");
const path = require("path");

const BUNDLE = path.join(__dirname, "..", "..", "..", "protocol", "vectors", "proof", "b1-groth16.json");

// Flip the last byte of a 0x-hex string (smallest tamper that stays well-formed).
function flipLastByte(hex) {
  const b = Buffer.from(hex.slice(2), "hex");
  b[b.length - 1] ^= 0x01;
  return "0x" + b.toString("hex");
}

describe("SP1 v6.1.0 Groth16 verifier (real proof bundle)", () => {
  let verifier;
  let bundle;

  before(async () => {
    bundle = JSON.parse(fs.readFileSync(BUNDLE, "utf8"));
    const Factory = await ethers.getContractFactory("SP1Verifier");
    verifier = await Factory.deploy();
    await verifier.waitForDeployment();
  });

  it("vendored verifier is the version the bundle targets", async () => {
    expect(await verifier.VERSION()).to.equal(bundle.circuit_version); // v6.1.0
    // The bundle's proof selector is the first 4 bytes of the verifier hash.
    const selector = bundle.proof_bytes.slice(0, 10); // "0x" + 4 bytes
    expect((await verifier.VERIFIER_HASH()).slice(0, 10)).to.equal(selector);
  });

  it("verifies the published B=1 proof", async () => {
    await expect(verifier["verifyProof(bytes32,bytes,bytes)"](bundle.vkey, bundle.public_values, bundle.proof_bytes)).to
      .not.be.reverted;
  });

  it("rejects a tampered proof", async () => {
    await expect(
      verifier["verifyProof(bytes32,bytes,bytes)"](bundle.vkey, bundle.public_values, flipLastByte(bundle.proof_bytes)),
    ).to.be.reverted;
  });

  it("rejects tampered public values", async () => {
    await expect(
      verifier["verifyProof(bytes32,bytes,bytes)"](bundle.vkey, flipLastByte(bundle.public_values), bundle.proof_bytes),
    ).to.be.reverted;
  });

  it("rejects a wrong program vkey", async () => {
    await expect(
      verifier["verifyProof(bytes32,bytes,bytes)"](flipLastByte(bundle.vkey), bundle.public_values, bundle.proof_bytes),
    ).to.be.reverted;
  });
});
