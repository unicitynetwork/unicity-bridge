#!/usr/bin/env node
/**
 * Real end-to-end bridge demo: Tron Nile testnet  ->  Unicity testnet2.
 *
 *   1. deploy   — deploy MockTRC20 (USDT) + MockProofVerifier + UnicityBridgeVault
 *   2. lock     — approve + vault.lock() USDT (stores lockDigest -> bridge-back-ready),
 *                 committing to a Unicity tokenId+recipient
 *   3. wait     — wait for Tron source finality (K confirmations)
 *   4. mint     — mint the bridged Unicity token (verifies the lock vs. live Nile)
 *   5. transfer — transfer the token to a second Unicity owner
 *   6. verify   — receiver re-verifies the token (re-checks the Tron lock proof)
 *
 * Each step persists to demo/.demo-state.json so the flow can be driven one
 * command at a time from a shell. See ../DEMO.md.
 */
import { AggregatorClient } from '@unicitylabs/state-transition-sdk/lib/api/AggregatorClient.js';
import { RootTrustBase } from '@unicitylabs/state-transition-sdk/lib/api/bft/RootTrustBase.js';
import { CertificationData } from '@unicitylabs/state-transition-sdk/lib/api/CertificationData.js';
import { CertificationStatus } from '@unicitylabs/state-transition-sdk/lib/api/CertificationResponse.js';
import { NetworkId } from '@unicitylabs/state-transition-sdk/lib/api/NetworkId.js';
import { SigningService } from '@unicitylabs/state-transition-sdk/lib/crypto/secp256k1/SigningService.js';
import { SignaturePredicate } from '@unicitylabs/state-transition-sdk/lib/predicate/builtin/SignaturePredicate.js';
import { SignaturePredicateUnlockScript } from '@unicitylabs/state-transition-sdk/lib/predicate/builtin/SignaturePredicateUnlockScript.js';
import { EncodedPredicate } from '@unicitylabs/state-transition-sdk/lib/predicate/EncodedPredicate.js';
import { PredicateVerifierService } from '@unicitylabs/state-transition-sdk/lib/predicate/verification/PredicateVerifierService.js';
import { StateTransitionClient } from '@unicitylabs/state-transition-sdk/lib/StateTransitionClient.js';
import { MintTransaction } from '@unicitylabs/state-transition-sdk/lib/transaction/MintTransaction.js';
import { Token } from '@unicitylabs/state-transition-sdk/lib/transaction/Token.js';
import { TokenId } from '@unicitylabs/state-transition-sdk/lib/transaction/TokenId.js';
import { TokenSalt } from '@unicitylabs/state-transition-sdk/lib/transaction/TokenSalt.js';
import { TokenType } from '@unicitylabs/state-transition-sdk/lib/transaction/TokenType.js';
import { TransferTransaction } from '@unicitylabs/state-transition-sdk/lib/transaction/TransferTransaction.js';
import { MintJustificationVerifierService } from '@unicitylabs/state-transition-sdk/lib/transaction/verification/MintJustificationVerifierService.js';
import { waitInclusionProof } from '@unicitylabs/state-transition-sdk/lib/util/InclusionProofUtils.js';
import { VerificationStatus } from '@unicitylabs/state-transition-sdk/lib/verification/VerificationStatus.js';

import {
  createTronUsdtBridgePlugin,
  decodeBridgedValue,
  encodeBridgedValue,
  fromHex,
  LOCK_EVENT_TOPIC0,
  recipientCommitment,
  toHex,
  TronUsdtLockJustification,
  type TronUsdtBridgeConfig,
} from '../src/index.js';
import { readEnv, requireTronKey } from './env.js';
import { loadState, requireState, saveState, type DemoState } from './state.js';
import {
  deployContract,
  encodeVaultCtor,
  getNowBlock,
  makeTronWeb,
  MOCK_PROOF_VERIFIER,
  MOCK_TRC20,
  sendMethod,
  toEvmHex,
  UNICITY_BRIDGE_VAULT,
  waitForReceipt,
} from './tron.js';

const env = readEnv();

function log(msg = ''): void {
  process.stdout.write(msg + '\n');
}

function nileTx(txid: string): string {
  return `https://nile.tronscan.org/#/transaction/${txid}`;
}

/**
 * Build the bridge plugin against the live Nile node. `confirmations` is the
 * finality threshold this party enforces: the minter passes 0 (it trusts its
 * own lock — being in a block is enough), an independent verifier passes K.
 */
function bridgePlugin(state: DemoState, confirmations: number): ReturnType<typeof createTronUsdtBridgePlugin> {
  const tron = requireState(state, 'tron', 'deploy');
  const config: TronUsdtBridgeConfig = {
    chainId: tron.chainId,
    lockContract: tron.lockEvmHex,
    assetContract: tron.assetEvmHex,
    confirmations,
    decimals: 6,
    rpcUrl: env.tronRpc,
    apiKey: env.tronApiKey,
  };
  // Real HTTP Tron RPC (no mock); value read from the token's own data envelope.
  return createTronUsdtBridgePlugin(config, { extractAmount: decodeBridgedValue });
}

/** Depth-first search for a verification sub-result whose message matches. */
function findResult(
  result: { message: string; results: readonly unknown[] },
  match: (message: string) => boolean,
): { message: string } | null {
  if (result.message && match(result.message)) {
    return result;
  }
  for (const child of result.results) {
    const found = findResult(child as typeof result, match);
    if (found) {
      return found;
    }
  }
  return null;
}

async function trustBase(): Promise<RootTrustBase> {
  const res = await fetch(env.trustBaseUrl);
  if (!res.ok) {
    throw new Error(`Failed to fetch trust base: HTTP ${res.status} ${env.trustBaseUrl}`);
  }
  return RootTrustBase.fromJSON(await res.json());
}

function unicityClient(): StateTransitionClient {
  return new StateTransitionClient(new AggregatorClient(env.aggregatorUrl, env.aggregatorApiKey ?? null));
}

// ---------------------------------------------------------------------------
// 1. deploy
// ---------------------------------------------------------------------------
async function deploy(state: DemoState): Promise<void> {
  const key = requireTronKey(env);
  const tronWeb = makeTronWeb(env, key);
  const deployer = tronWeb.defaultAddress.base58 as string;
  log(`Deployer: ${deployer}  (chainId ${env.tronChainId})`);
  log(`RPC: ${env.tronRpc}\n`);

  log('Deploying MockTRC20 (USDT stand-in)...');
  const asset = await deployContract(tronWeb, MOCK_TRC20(), 'MockTRC20', []);
  log(`  asset (USDT) = ${asset.base58}  ${nileTx(asset.txid)}`);

  // Bridge-in routes through the current contract, UnicityBridgeVault, so the
  // deposit also stores lockDigest (making it bridge-back-able). lock() and the
  // Lock event are identical to the old UnicityLock. The mock verifier is unused
  // by lock() (only fulfillBatch verifies proofs).
  log('Deploying MockProofVerifier...');
  const verifier = await deployContract(tronWeb, MOCK_PROOF_VERIFIER(), 'MockProofVerifier', []);
  log(`  verifier     = ${verifier.base58}  ${nileTx(verifier.txid)}`);

  log('Deploying UnicityBridgeVault(cfg, verifier, vkey, admin, pull=false)...');
  const ctor = encodeVaultCtor({
    chainId: env.tronChainId,
    assetEvm: asset.evmHex,
    verifierEvm: verifier.evmHex,
    adminEvm: toEvmHex(tronWeb, deployer),
    pullPayments: false,
  });
  const lock = await deployContract(tronWeb, UNICITY_BRIDGE_VAULT(), 'UnicityBridgeVault', [], ctor);
  log(`  vault (lock) = ${lock.base58}  ${nileTx(lock.txid)}`);

  // Fund the deployer with USDT so it can lock.
  const fundAmount = env.amount * 10n;
  log(`Minting ${fundAmount} test-USDT to deployer...`);
  const mintTxid = await sendMethod(tronWeb, MOCK_TRC20().abi, asset.base58, 'mint', [deployer, fundAmount.toString()]);
  await waitForReceipt(tronWeb, mintTxid);
  log(`  mint tx      = ${nileTx(mintTxid)}\n`);

  state.tron = {
    chainId: env.tronChainId,
    rpcUrl: env.tronRpc,
    deployerBase58: deployer,
    deployerEvmHex: toEvmHex(tronWeb, deployer),
    assetBase58: asset.base58,
    assetEvmHex: asset.evmHex,
    lockBase58: lock.base58,
    lockEvmHex: lock.evmHex,
    deployTxids: { asset: asset.txid, lock: lock.txid, mint: mintTxid },
  };
  saveState(state);

  const plugin = bridgePlugin(state, env.confirmations);
  log('Bridged-asset identifiers (deterministic from chainId + asset):');
  log(`  TokenType = ${plugin.tokenTypeHex}`);
  log(`  coinId    = ${plugin.coinIdHex}`);
  log('\n✔ deploy complete.');
}

// ---------------------------------------------------------------------------
// 2. lock
// ---------------------------------------------------------------------------
async function lock(state: DemoState): Promise<void> {
  const key = requireTronKey(env);
  const tron = requireState(state, 'tron', 'deploy');
  const tronWeb = makeTronWeb(env, key);

  // Pick the Unicity token this deposit will fund, and its recipient.
  const recipientPriv = SigningService.generatePrivateKey();
  const recipientSigning = new SigningService(recipientPriv);
  const recipientPredicate = SignaturePredicate.fromSigningService(recipientSigning);
  const recipientCommitmentBytes = recipientCommitment(EncodedPredicate.fromPredicate(recipientPredicate).toCBOR());

  const saltBytes = crypto.getRandomValues(new Uint8Array(32));
  const salt = TokenSalt.fromBytes(saltBytes);
  const networkId = NetworkId.fromId(env.networkId);
  const tokenId = await TokenId.fromSalt(networkId, salt);

  log('Bridge intent:');
  log(`  Unicity tokenId        = ${toHex(tokenId.bytes)}`);
  log(`  recipientCommitment    = ${toHex(recipientCommitmentBytes)}`);
  log(`  amount                 = ${env.amount} (6 decimals)\n`);

  log(`Approving ${env.amount} USDT to lock contract...`);
  const approveTxid = await sendMethod(tronWeb, MOCK_TRC20().abi, tron.assetBase58, 'approve', [
    tron.lockBase58,
    env.amount.toString(),
  ]);
  await waitForReceipt(tronWeb, approveTxid);
  log(`  approve tx = ${nileTx(approveTxid)}`);

  log('Calling lock(amount, tokenId, recipientCommitment)...');
  const lockTxid = await sendMethod(tronWeb, UNICITY_BRIDGE_VAULT().abi, tron.lockBase58, 'lock', [
    env.amount.toString(),
    '0x' + toHex(tokenId.bytes),
    '0x' + toHex(recipientCommitmentBytes),
  ]);
  const info = await waitForReceipt(tronWeb, lockTxid);
  log(`  lock tx    = ${nileTx(lockTxid)}`);

  // Locate the Lock event in the tx logs (topic0 == keccak256 of the signature).
  const logIndex = info.log.findIndex(
    (l) => l.address.toLowerCase() === tron.lockEvmHex && (l.topics[0] ?? '').toLowerCase() === LOCK_EVENT_TOPIC0,
  );
  if (logIndex < 0) {
    throw new Error('Lock event not found in transaction logs.');
  }
  const nonce = BigInt('0x' + info.log[logIndex].topics[1]); // indexed nonce
  log(`  block      = ${info.blockNumber}, logIndex = ${logIndex}, nonce = ${nonce}`);

  state.intent = {
    tokenIdHex: toHex(tokenId.bytes),
    saltHex: toHex(saltBytes),
    recipientCommitmentHex: toHex(recipientCommitmentBytes),
    recipientPrivKeyHex: toHex(recipientPriv),
    amount: env.amount.toString(),
  };
  state.lock = { txid: lockTxid, blockNumber: info.blockNumber, logIndex, nonce: Number(nonce) };
  saveState(state);
  log('\n✔ lock complete. The USDT is locked and bound to this exact token + recipient.');
}

// ---------------------------------------------------------------------------
// wait (OPTIONAL): watch source finality accrue. Not on the critical path —
// the minter does not wait, and `verify` already retries until finality. This
// is a read-only observation helper (no Tron key needed).
// ---------------------------------------------------------------------------
async function wait(state: DemoState): Promise<void> {
  const lk = requireState(state, 'lock', 'lock');
  const tronWeb = makeTronWeb(env);
  log(`Watching confirmations on Tron (lock in block ${lk.blockNumber}; target ${env.confirmations})...`);
  for (;;) {
    const tip = await getNowBlock(tronWeb);
    const confs = tip - lk.blockNumber;
    log(`  tip ${tip}  (${confs}/${env.confirmations} confirmations)`);
    if (confs >= env.confirmations) {
      break;
    }
    await new Promise((r) => setTimeout(r, 6000));
  }
  log('\n✔ source final. Independent verifiers will now accept the token.');
}

// ---------------------------------------------------------------------------
// 4. mint (Unicity)
// ---------------------------------------------------------------------------
async function mint(state: DemoState): Promise<void> {
  const tron = requireState(state, 'tron', 'deploy');
  const intent = requireState(state, 'intent', 'lock');
  const lk = requireState(state, 'lock', 'lock');

  // The minter trusts its own lock: it requires only that the lock tx is in a
  // block (MINT_CONFIRMATIONS, default 0), not full source finality. Independent
  // receivers enforce the real K threshold later (see `verify`).
  const plugin = bridgePlugin(state, env.mintConfirmations);
  const mintJustificationVerifier = new MintJustificationVerifierService();
  mintJustificationVerifier.register(plugin.verifier); // dispatched by CBOR tag 1330002
  const predicateVerifier = PredicateVerifierService.create();

  const networkId = NetworkId.fromId(env.networkId);
  const salt = TokenSalt.fromBytes(fromHex(intent.saltHex));
  const recipientSigning = new SigningService(fromHex(intent.recipientPrivKeyHex));
  const recipientPredicate = SignaturePredicate.fromSigningService(recipientSigning);
  const amount = BigInt(intent.amount);

  // Token value envelope (the value the verifier cross-checks against the lock).
  const valueData = encodeBridgedValue(plugin.resolvedConfig.coinId, amount);
  const tokenType = new TokenType(plugin.resolvedConfig.tokenType);

  // Mint reason = self-contained reference to the real Nile lock.
  const justification = new TronUsdtLockJustification({
    chainId: tron.chainId,
    lockContract: fromHex(tron.lockEvmHex),
    assetContract: fromHex(tron.assetEvmHex),
    txid: fromHex(lk.txid),
    logIndex: lk.logIndex,
    amount,
    nonce: BigInt(lk.nonce),
  }).toCBOR();

  const mintTransaction = await MintTransaction.create(
    networkId,
    recipientPredicate,
    valueData,
    tokenType,
    salt,
    justification,
  );

  log(`Submitting mint commitment to ${env.aggregatorUrl} ...`);
  const client = unicityClient();
  const certificationData = await CertificationData.fromMintTransaction(mintTransaction);
  await client.submitCertificationRequest(certificationData);

  const tb = await trustBase();
  log('Waiting for inclusion proof + checking the lock is in a block on Nile...');
  const certified = await mintTransaction.toCertifiedTransaction(
    tb,
    predicateVerifier,
    await waitInclusionProof(client, tb, predicateVerifier, mintTransaction),
  );
  // Token.mint runs token.verify(...) which invokes the bridge plugin. At
  // MINT_CONFIRMATIONS=0 it confirms the lock exists, succeeded, and binds this
  // exact token+recipient — without waiting for finality (the minter self-trusts).
  const token = await Token.mint(tb, predicateVerifier, mintJustificationVerifier, certified);

  state.mint = {
    networkId: env.networkId,
    aggregatorUrl: env.aggregatorUrl,
    tokenTypeHex: plugin.tokenTypeHex,
    coinIdHex: plugin.coinIdHex,
    ownerPrivKeyHex: intent.recipientPrivKeyHex,
    tokenCborHex: toHex(token.toCBOR()),
  };
  saveState(state);

  log('\n✔ mint complete. A real Unicity token now encapsulates the bridged USDT.');
  log(`  tokenId  = ${intent.tokenIdHex}`);
  log(`  value    = ${decodeBridgedValue(valueData, plugin.resolvedConfig.coinId)} (coinId ${plugin.coinIdHex})`);
  log(`  token    = ${toHex(token.toCBOR()).slice(0, 64)}... (${toHex(token.toCBOR()).length / 2} bytes)`);
}

// ---------------------------------------------------------------------------
// 5. transfer (Unicity, owner -> new owner)
// ---------------------------------------------------------------------------
async function transfer(state: DemoState): Promise<void> {
  const m = requireState(state, 'mint', 'mint');
  // token.transfer only checks the transfer proof — it does not re-run the mint
  // justification — so the self-trusting owner needs no Tron finality here.
  const predicateVerifier = PredicateVerifierService.create();
  const tb = await trustBase();
  const client = unicityClient();

  const token = await Token.fromCBOR(fromHex(m.tokenCborHex));
  const ownerSigning = new SigningService(fromHex(m.ownerPrivKeyHex));

  // New recipient.
  const newPriv = SigningService.generatePrivateKey();
  const newPredicate = SignaturePredicate.fromSigningService(new SigningService(newPriv));
  const recipient = EncodedPredicate.fromPredicate(newPredicate);

  log('Building transfer to a new Unicity owner...');
  const transferTransaction = await TransferTransaction.create(
    token,
    recipient,
    crypto.getRandomValues(new Uint8Array(32)),
  );
  const certificationData = await CertificationData.fromTransaction(
    transferTransaction,
    await SignaturePredicateUnlockScript.create(transferTransaction, ownerSigning),
  );
  const response = await client.submitCertificationRequest(certificationData);
  if (response.status !== String(CertificationStatus.SUCCESS)) {
    throw new Error(`Transfer certification failed: ${response.status}`);
  }

  log('Waiting for inclusion proof...');
  const transferred = await token.transfer(
    tb,
    predicateVerifier,
    await transferTransaction.toCertifiedTransaction(
      tb,
      predicateVerifier,
      await waitInclusionProof(client, tb, predicateVerifier, transferTransaction),
    ),
  );

  state.transfer = { recipientPrivKeyHex: toHex(newPriv), tokenCborHex: toHex(transferred.toCBOR()) };
  saveState(state);
  log('\n✔ transfer complete. Token handed to a new owner over the aggregator.');
  log(`  new token = ${toHex(transferred.toCBOR()).slice(0, 64)}... (${toHex(transferred.toCBOR()).length / 2} bytes)`);
}

// ---------------------------------------------------------------------------
// 6. verify (receiver re-checks everything, incl. the Tron lock)
// ---------------------------------------------------------------------------
async function verify(state: DemoState): Promise<number> {
  const tr = requireState(state, 'transfer', 'transfer');
  // An independent receiver enforces the real finality threshold (K = CONFIRMATIONS).
  const plugin = bridgePlugin(state, env.confirmations);
  const mintJustificationVerifier = new MintJustificationVerifierService();
  mintJustificationVerifier.register(plugin.verifier);
  const predicateVerifier = PredicateVerifierService.create();
  const tb = await trustBase();
  const token = await Token.fromCBOR(fromHex(tr.tokenCborHex));

  log(`Receiver verifies from scratch (requires ${env.confirmations} confirmations on Tron)...`);
  const deadline = Date.now() + env.verifyTimeoutMs;
  for (let attempt = 1; ; attempt++) {
    // Re-running token.verify re-queries the live Nile node, so confirmations
    // grow between attempts until source finality is reached.
    const result = await token.verify(tb, predicateVerifier, mintJustificationVerifier);
    if (result.status === VerificationStatus.OK) {
      const value = decodeBridgedValue(token.genesis?.data ?? null, plugin.resolvedConfig.coinId);
      log('\n  ownership + history (Unicity)   : OK');
      log('  mint reason (re-checked vs Nile): OK');
      log('\n✔ RECEIVED TOKEN VERIFIED.');
      log(`  The token is genuinely backed by ${value} locked USDT on Tron Nile,`);
      log('  bound to this token id, and now owned by the receiver — no trusted bridge operator.');
      return 0;
    }

    // Distinguish "not final yet" (transient — retry) from a real rejection (fatal).
    const pending = findResult(result, (m) => /awaiting source finality|insufficient confirmations/i.test(m));
    if (!pending) {
      log(`\n✘ verification FAILED: ${result.status}`);
      const reason = findResult(result, (m) => m.length > 0);
      if (reason) {
        log(`  reason: ${reason.message}`);
      }
      log('\n  This is a genuine rejection (not a finality delay) — the token does');
      log('  not back its claimed lock. Full trace:\n');
      log(result.toString());
      return 1;
    }

    if (Date.now() >= deadline) {
      log(`\n✘ Timed out after ${Math.round(env.verifyTimeoutMs / 1000)}s still awaiting source finality.`);
      log(`  ${pending.message}`);
      log('  The lock is valid but not yet final. Re-run `npm run e2e verify` later,');
      log('  or lower the threshold with CONFIRMATIONS=<n> (a receiver-side trust choice).');
      return 2;
    }
    log(`  [attempt ${attempt}] ${pending.message}`);
    log(`            not final yet — retrying in ${Math.round(env.verifyRetryMs / 1000)}s ...`);
    await new Promise((r) => setTimeout(r, env.verifyRetryMs));
  }
}

// ---------------------------------------------------------------------------
// dispatch
// ---------------------------------------------------------------------------
function summary(state: DemoState): void {
  log('Demo state:');
  log(`  tron deployed : ${state.tron ? 'yes (' + state.tron.lockBase58 + ')' : 'no'}`);
  log(`  locked        : ${state.lock ? 'yes (' + state.lock.txid + ')' : 'no'}`);
  log(`  minted        : ${state.mint ? 'yes' : 'no'}`);
  log(`  transferred   : ${state.transfer ? 'yes' : 'no'}`);
}

async function main(): Promise<void> {
  const cmd = process.argv[2] ?? 'status';
  const state = loadState();
  let code = 0;
  switch (cmd) {
    case 'deploy':
      await deploy(state);
      break;
    case 'lock':
      await lock(state);
      break;
    case 'wait':
      await wait(state);
      break;
    case 'mint':
      await mint(state);
      break;
    case 'transfer':
      await transfer(state);
      break;
    case 'verify':
      code = await verify(state);
      break;
    case 'status':
      summary(state);
      break;
    default:
      log(`Unknown command: ${cmd}`);
      log('Commands: deploy | lock | mint | transfer | verify | status   (wait = optional finality watcher)');
      code = 2;
  }
  process.exit(code);
}

void main().catch((e) => {
  console.error('\n✘ Error:', e instanceof Error ? e.message : e);
  process.exit(1);
});
