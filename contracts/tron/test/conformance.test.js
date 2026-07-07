// Conformance tests: the vault's on-chain keccak/ABI recomputations must equal
// the cross-stack `protocol/vectors` fixtures (interop §10). The reference generator is
// Rust (protocol/vectors/gen); here the Solidity side proves it reproduces the
// same bytes for the groups the vault recomputes: config, lock, public.
const { expect } = require("chai");
const { ethers } = require("hardhat");
const fs = require("fs");
const path = require("path");

const VECTORS = path.join(__dirname, "..", "..", "..", "protocol", "vectors");

function loadVector(group, file) {
  return JSON.parse(fs.readFileSync(path.join(VECTORS, group, file), "utf8"));
}

describe("protocol/vectors conformance (config/lock/public)", () => {
  let harness;

  before(async () => {
    const Harness = await ethers.getContractFactory("EncodingHarness");
    harness = await Harness.deploy();
    await harness.waitForDeployment();
  });

  it("BRIDGE_PROTO_VERSION matches the pinned vectors version", () => {
    const version = fs.readFileSync(path.join(VECTORS, "VERSION"), "utf8").trim();
    expect(version).to.equal("1");
  });

  it("config -> configHash (00 §2)", async () => {
    const v = loadVector("config", "config-00.json");
    const out = loadVector("config", "config-00.json").out;
    const cfg = {
      sourceChainId: BigInt(v.in.source_chain_id),
      vault: v.in.vault,
      asset: v.in.asset,
      tokenType: out.token_type,
      coinId: out.coin_id,
      reasonTag: BigInt(v.in.reason_tag),
      lockDomain: v.in.lock_domain,
      nullifierDomain: v.in.nullifier_domain,
    };
    expect(await harness.configHash(cfg)).to.equal(out.config_hash);
  });

  it("lock -> lockDigest (00 §3)", async () => {
    const cfg = loadVector("config", "config-00.json");
    const lk = loadVector("lock", "lock-00.json");
    const digest = await harness.lockDigest(
      BigInt(cfg.in.source_chain_id),
      cfg.in.vault,
      BigInt(lk.in.nonce),
      cfg.in.asset,
      cfg.out.token_type,
      cfg.out.coin_id,
      BigInt(lk.in.amount),
      lk.in.unicity_token_id,
      lk.out.recipient_commitment
    );
    expect(digest).to.equal(lk.out.lock_digest);
  });

  it("public -> domainTag / returnRoot / lockRefRoot / PublicValues ABI (00 §7)", async () => {
    const p = loadVector("public", "public-00.json");

    expect(await harness.domainTag()).to.equal(p.out.domain_tag);

    const leaves = p.in.leaves.map((l) => [
      l.nullifier,
      l.recipient,
      BigInt(l.amount),
      l.fee_recipient,
      BigInt(l.fee_amount),
      BigInt(l.deadline),
    ]);
    expect(await harness.returnRoot(leaves)).to.equal(p.out.return_root);

    // The vault recomputes lockRefRoot over refs sorted ascending by nonce; the
    // fixture lists them unsorted to document that the prover sorts (00 §7).
    const refs = [...p.in.lock_refs]
      .sort((a, b) => (BigInt(a.nonce) < BigInt(b.nonce) ? -1 : 1))
      .map((r) => [BigInt(r.nonce), r.digest]);
    expect(await harness.lockRefRoot(refs)).to.equal(p.out.lock_ref_root);

    const pv = [
      p.out.domain_tag,
      p.out.config_hash,
      p.in.trust_base_hash,
      p.in.spent_root_old,
      p.in.spent_root_new,
      p.out.return_root,
      p.out.lock_ref_root,
      Number(p.in.batch_size),
      BigInt(p.in.total_amount),
    ];
    const encoded = await harness.encodePublicValues(pv);
    expect(encoded).to.equal(p.out.public_values_abi);

    // round-trip: the vault recovers every field with one abi.decode
    const decoded = await harness.decodePublicValues(encoded);
    expect(decoded.domainTag).to.equal(p.out.domain_tag);
    expect(decoded.configHash).to.equal(p.out.config_hash);
    expect(decoded.returnRoot).to.equal(p.out.return_root);
    expect(decoded.lockRefRoot).to.equal(p.out.lock_ref_root);
    expect(decoded.batchSize).to.equal(BigInt(p.in.batch_size));
    expect(decoded.totalAmount).to.equal(BigInt(p.in.total_amount));
  });
});
