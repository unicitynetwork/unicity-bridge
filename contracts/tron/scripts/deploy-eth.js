const fs = require("fs");
const path = require("path");
const crypto = require("crypto");
const ethers = require("ethers");
const { loadEnv } = require("./env");

const LOCK_DOMAIN = "0x158b847f78b3910a5f5f42820de61abba1bf5ae1fbb29dabfba09118f393f932";
const NULLIFIER_DOMAIN = "0xd4530e4ea58fc8e38f84506e62b421476c3eeec70f4cbebefc32688a510e2d5d";
const REASON_TAG = 39048;

function sha256Hex(s) {
  return "0x" + crypto.createHash("sha256").update(Buffer.from(s, "utf8")).digest("hex");
}
const CHAIN_FAMILY = "eip155";
function deriveTokenType(chainIdStr, assetEvmHex) {
  return sha256Hex(`unicity-bridge:${CHAIN_FAMILY}:${chainIdStr}:${assetEvmHex}`);
}
function deriveCoinId(chainIdStr, assetEvmHex) {
  return sha256Hex(`unicity-bridge-coin:${CHAIN_FAMILY}:${chainIdStr}:${assetEvmHex}`);
}

function artifact(file) {
  const p = path.join(__dirname, "..", "artifacts", "contracts", file);
  const j = JSON.parse(fs.readFileSync(p, "utf8"));
  return { abi: j.abi, bytecode: j.bytecode };
}
const VAULT = artifact("UnicityBridgeVault.sol/UnicityBridgeVault.json");
const SP1_ERRORS = ["error InvalidProof()", "error WrongVerifierSelector(bytes4 received, bytes4 expected)", "error InvalidExitCode()", "error InvalidVkRoot()", "error ProofInvalid()", "error PublicInputNotInField()", "error RouteNotFound(bytes4 selector)", "error RouteIsFrozen(bytes4 selector)"];
const MOCK = artifact("test/MockTRC20.sol/MockTRC20.json");
const ERC20 = ["function approve(address spender, uint256 value) returns (bool)", "function balanceOf(address owner) view returns (uint256)", "function symbol() view returns (string)"];
const SP1_VERIFIER = artifact("verifier/v6.1.0/SP1VerifierGroth16.sol/SP1Verifier.json");

function context() {
  const env = loadEnv();
  const rpc = env.ETH_RPC_URL || "https://ethereum-sepolia-rpc.publicnode.com";
  const chainId = Number(env.ETH_CHAIN_ID || 11155111);
  if (!env.ETH_SK) throw new Error("ETH_SK not set in .env");
  const provider = new ethers.JsonRpcProvider(rpc, chainId, { staticNetwork: true });
  const wallet = new ethers.Wallet(env.ETH_SK, provider);
  const signer = new ethers.NonceManager(wallet);
  return { env, rpc, chainId, provider, signer, address: wallet.address };
}

function bridgeConfig(chainId, asset, vault) {
  const assetEvmHex = asset.slice(2).toLowerCase();
  return {
    sourceChainId: chainId,
    vault,
    asset,
    tokenType: deriveTokenType(String(chainId), assetEvmHex),
    coinId: deriveCoinId(String(chainId), assetEvmHex),
    reasonTag: REASON_TAG,
    lockDomain: LOCK_DOMAIN,
    nullifierDomain: NULLIFIER_DOMAIN,
  };
}

async function deployed(label, tx, receipt) {
  console.log(`  ${label} deployed at ${receipt.contractAddress} (tx ${tx.hash}, gas ${receipt.gasUsed}, block ${receipt.blockNumber})`);
  return receipt.contractAddress;
}

async function deployMockAsset({ signer }) {
  console.log("Deploying MockTRC20 ...");
  const factory = new ethers.ContractFactory(MOCK.abi, MOCK.bytecode, signer);
  const c = await factory.deploy();
  const tx = c.deploymentTransaction();
  return deployed("MockTRC20", tx, await tx.wait());
}

async function deployVerifier({ signer }) {
  console.log("Deploying SP1Verifier v6.1.0 ...");
  const factory = new ethers.ContractFactory(SP1_VERIFIER.abi, SP1_VERIFIER.bytecode, signer);
  const c = await factory.deploy();
  const tx = c.deploymentTransaction();
  return deployed("SP1Verifier", tx, await tx.wait());
}

async function deployVault({ env, chainId, signer, address: admin }, asset, vkey) {
  const verifier = env.ETH_SP1_GATEWAY;
  if (!verifier) throw new Error("ETH_SP1_GATEWAY not set in .env");
  const pullPayments = env.ETH_PULL_PAYMENTS === "1";
  const cfg = bridgeConfig(chainId, asset, admin);
  console.log(`Deploying UnicityBridgeVault (asset ${asset}, verifier ${verifier}, vkey ${vkey.slice(0, 12)}…, ${pullPayments ? "PULL" : "push"}-payment) ...`);
  const factory = new ethers.ContractFactory(VAULT.abi, VAULT.bytecode, signer);
  const c = await factory.deploy(cfg, verifier, vkey, admin, pullPayments);
  const tx = c.deploymentTransaction();
  const address = await deployed("UnicityBridgeVault", tx, await tx.wait());
  const vault = new ethers.Contract(address, VAULT.abi, signer);
  console.log(`  CONFIG_HASH ${await vault.CONFIG_HASH()}`);
  console.log(`  tokenType   ${cfg.tokenType}`);
  console.log(`  coinId      ${cfg.coinId}`);
  return address;
}

async function allowTrustBase({ signer }, vaultAddr, hash) {
  const vault = new ethers.Contract(vaultAddr, VAULT.abi, signer);
  const tx = await vault.setTrustBaseAllowed(hash, true);
  const r = await tx.wait();
  console.log(`  setTrustBaseAllowed(${hash}) tx ${tx.hash} gas ${r.gasUsed}; allowed=${await vault.trustBaseAllowed(hash)}`);
}

async function lockSmoke({ signer, address }, vaultAddr, assetAddr, amountArg) {
  const amount = BigInt(amountArg || 1_000_000);
  const asset = new ethers.Contract(assetAddr, ERC20, signer);
  const vault = new ethers.Contract(vaultAddr, VAULT.abi, signer);
  const tokenId = ethers.hexlify(ethers.randomBytes(32));
  const recipientCommitment = ethers.hexlify(ethers.randomBytes(32));
  const held = await asset.balanceOf(address);
  console.log(`  deployer holds ${held} ${await asset.symbol()}`);
  if (held < amount) throw new Error(`lock-smoke: need ${amount}, hold ${held}`);
  let tx = await asset.approve(vaultAddr, amount);
  console.log(`  approve tx ${tx.hash} gas ${(await tx.wait()).gasUsed}`);
  tx = await vault.lock(amount, tokenId, recipientCommitment);
  const r = await tx.wait();
  const log = r.logs.map((l) => { try { return vault.interface.parseLog(l); } catch { return null; } }).find((l) => l && l.name === "Lock");
  console.log(`  lock tx ${tx.hash} gas ${r.gasUsed}; Lock(nonce ${log.args.nonce}, amount ${log.args.amount})`);
  console.log(`  lockDigest[${log.args.nonce}] ${await vault.lockDigest(log.args.nonce)}`);
  console.log(`  vault balance ${await asset.balanceOf(vaultAddr)}`);
}

async function fulfillProbe({ signer, address }, vaultAddr, bundlePath) {
  const b = JSON.parse(fs.readFileSync(bundlePath || path.join(__dirname, "..", "..", "..", "protocol/vectors/proof/b1-groth16.json"), "utf8"));
  const vault = new ethers.Contract(vaultAddr, [...VAULT.abi, ...SP1_ERRORS], signer);
  const leaf = { nullifier: ethers.ZeroHash, recipient: address, amount: 1n, feeRecipient: ethers.ZeroAddress, feeAmount: 0n, deadline: 0n };
  try {
    await vault.fulfillBatch.staticCall(b.public_values, b.proof_bytes, [leaf], []);
    console.log("  fulfillBatch did not revert (unexpected for a foreign bundle)");
  } catch (e) {
    const data = e.data ?? e.info?.error?.data;
    console.log(`  fulfillBatch reverted: ${e.reason || (data && vault.interface.parseError(data)?.name) || e.shortMessage || e.message}`);
  }
}

async function withdraw({ signer, address }, vaultAddr) {
  const vault = new ethers.Contract(vaultAddr, VAULT.abi, signer);
  const owed = await vault.owed(address);
  console.log(`  owed to ${address}: ${owed}`);
  if (owed === 0n) return;
  const tx = await vault.withdraw();
  const r = await tx.wait();
  console.log(`  withdraw tx ${tx.hash} gas ${r.gasUsed}; owed now ${await vault.owed(address)}`);
}

async function freeze({ env, chainId, provider }, vaultAddr) {
  const vault = new ethers.Contract(vaultAddr, VAULT.abi, provider);
  const asset = await vault.ASSET();
  const cfg = bridgeConfig(chainId, asset.toLowerCase(), vaultAddr.toLowerCase());
  const record = {
    network: env.ETH_NETWORK || `eip155-${chainId}`,
    chain_family: CHAIN_FAMILY,
    chain_id_str: String(chainId),
    asset_evm_hex: asset.slice(2).toLowerCase(),
    config: {
      asset: asset.toLowerCase(),
      coin_id: cfg.coinId,
      config_hash: await vault.CONFIG_HASH(),
      domain_tag: await vault.DOMAIN_TAG(),
      lock_domain: LOCK_DOMAIN,
      nullifier_domain: NULLIFIER_DOMAIN,
      reason_tag: REASON_TAG,
      source_chain_id: chainId,
      token_type: cfg.tokenType,
      vault: vaultAddr.toLowerCase(),
    },
    deployment: {
      vault: vaultAddr,
      verifier_sp1: await vault.verifier(),
      verifier_sp1_impl: env.ETH_SP1_VERIFIER_V610 || null,
      asset,
      vkey: await vault.VKEY(),
      pull_payments: await vault.PULL_PAYMENTS(),
      admin: await vault.admin(),
    },
  };
  console.log(JSON.stringify(record, null, 2));
}

async function main() {
  const ctx = context();
  const cmd = process.argv[2] || "balance";
  const bal = await ctx.provider.getBalance(ctx.address);
  console.log(`Deployer ${ctx.address}: ${ethers.formatEther(bal)} ETH on ${ctx.rpc} (chain ${ctx.chainId})`);
  const a = process.argv.slice(3);
  switch (cmd) {
    case "balance": return;
    case "mock-asset": return deployMockAsset(ctx);
    case "verifier": return deployVerifier(ctx);
    case "vault": {
      if (a.length < 2) throw new Error("usage: vault <asset> <vkey>");
      return deployVault(ctx, ethers.getAddress(a[0]), a[1]);
    }
    case "allow-trust-base": {
      if (a.length < 2) throw new Error("usage: allow-trust-base <vault> <hash>");
      return allowTrustBase(ctx, a[0], a[1]);
    }
    case "lock-smoke": {
      if (a.length < 2) throw new Error("usage: lock-smoke <vault> <asset> [amount]");
      return lockSmoke(ctx, a[0], a[1], a[2]);
    }
    case "fulfill-probe": {
      if (a.length < 1) throw new Error("usage: fulfill-probe <vault> [bundle]");
      return fulfillProbe(ctx, a[0], a[1]);
    }
    case "freeze": {
      if (a.length < 1) throw new Error("usage: freeze <vault>");
      return freeze(ctx, a[0]);
    }
    case "withdraw": {
      if (a.length < 1) throw new Error("usage: withdraw <vault>");
      return withdraw(ctx, a[0]);
    }
    default:
      throw new Error(`unknown command: ${cmd}`);
  }
}

main().catch((e) => {
  console.error(e.message || e);
  process.exit(1);
});
