#!/usr/bin/env node
/**
 * Live bridge-BACK e2e (the M2 integration backstop, 02 §"Testing"):
 *
 *   1. mint   — mint a **bridge-lock-genesis** Unicity token to a fresh owner on
 *               the live testnet2 aggregator. The genesis carries a real
 *               `TronUsdtLockJustification` (tag 1330002), structurally verified
 *               at mint time against an in-process mock Tron RPC. This is the
 *               same 8-field CBOR the prover's Rust `BridgeLockJustification`
 *               decodes, so the burned blob is consumable by `bridge_lock_obligation`.
 *   2. burn   — burn it to a canonical `BridgeBackReason`: terminal recipient
 *               predicate `BurnPredicate(H(reasonBytes))`, `reasonBytes` in aux
 *               data (00 §4); certify on the aggregator.
 *   3. derive — from the REAL certified burn (stateId + tx hash) derive the
 *               nullifier + return leaf exactly as the prover will (00 §5/§7).
 *   4. persist— write the burned-token blob + the prover witness-request envelope
 *               + the bridge config the prover must use to validate it.
 *
 * Uses the root `.env` (UNICITY_GATEWAY / UNICITY_API_KEY / UNICITY_TRUSTBASE).
 * Only the Unicity aggregator is live; the Tron lock is mocked in-process so the
 * mint's bridge backing verifies without a real Nile deployment.
 */
import { readFileSync, writeFileSync } from 'node:fs';

import { AggregatorClient } from '@unicitylabs/state-transition-sdk/lib/api/AggregatorClient.js';
import { RootTrustBase } from '@unicitylabs/state-transition-sdk/lib/api/bft/RootTrustBase.js';
import { CertificationData } from '@unicitylabs/state-transition-sdk/lib/api/CertificationData.js';
import { CertificationStatus } from '@unicitylabs/state-transition-sdk/lib/api/CertificationResponse.js';
import { NetworkId } from '@unicitylabs/state-transition-sdk/lib/api/NetworkId.js';
import { StateId } from '@unicitylabs/state-transition-sdk/lib/api/StateId.js';
import { SigningService } from '@unicitylabs/state-transition-sdk/lib/crypto/secp256k1/SigningService.js';
import { SignaturePredicate } from '@unicitylabs/state-transition-sdk/lib/predicate/builtin/SignaturePredicate.js';
import { SignaturePredicateUnlockScript } from '@unicitylabs/state-transition-sdk/lib/predicate/builtin/SignaturePredicateUnlockScript.js';
import { EncodedPredicate } from '@unicitylabs/state-transition-sdk/lib/predicate/EncodedPredicate.js';
import { PredicateVerifierService } from '@unicitylabs/state-transition-sdk/lib/predicate/verification/PredicateVerifierService.js';
import { StateTransitionClient } from '@unicitylabs/state-transition-sdk/lib/StateTransitionClient.js';
import { Asset } from '@unicitylabs/state-transition-sdk/lib/payment/asset/Asset.js';
import { AssetId } from '@unicitylabs/state-transition-sdk/lib/payment/asset/AssetId.js';
import { PaymentAssetCollection } from '@unicitylabs/state-transition-sdk/lib/payment/asset/PaymentAssetCollection.js';
import { MintTransaction } from '@unicitylabs/state-transition-sdk/lib/transaction/MintTransaction.js';
import { Token } from '@unicitylabs/state-transition-sdk/lib/transaction/Token.js';
import { TokenId } from '@unicitylabs/state-transition-sdk/lib/transaction/TokenId.js';
import { TokenSalt } from '@unicitylabs/state-transition-sdk/lib/transaction/TokenSalt.js';
import { TokenType } from '@unicitylabs/state-transition-sdk/lib/transaction/TokenType.js';
import { MintJustificationVerifierService } from '@unicitylabs/state-transition-sdk/lib/transaction/verification/MintJustificationVerifierService.js';
import { waitInclusionProof } from '@unicitylabs/state-transition-sdk/lib/util/InclusionProofUtils.js';

import { sha256 } from '@noble/hashes/sha2.js';

import {
  type BridgeBackReason,
  type BridgeConfig,
  buildWitnessRequest,
  configHash as deriveConfigHash,
  createBridgeBackBurnTransfer,
  createTronUsdtBridgePlugin,
  fromHex,
  LOCK_EVENT_TOPIC0,
  lockDigest as deriveLockDigest,
  previewReturn,
  recipientCommitment,
  toHex,
  TronUsdtLockJustification,
  TRON_NILE_CHAIN_ID,
  type TronLog,
  type TronRpc,
  type TronTxInfo,
} from '../src/index.js';

// ---- env -------------------------------------------------------------------
function parseDotenv(path: string): Record<string, string> {
  const out: Record<string, string> = {};
  for (const line of readFileSync(path, 'utf8').split('\n')) {
    const t = line.trim();
    if (!t || t.startsWith('#')) continue;
    const eq = t.indexOf('=');
    if (eq < 0) continue;
    let v = t.slice(eq + 1).trim();
    if ((v.startsWith('"') && v.endsWith('"')) || (v.startsWith("'") && v.endsWith("'"))) v = v.slice(1, -1);
    out[t.slice(0, eq).trim()] = v;
  }
  return out;
}

const repoRoot = new URL('../../', import.meta.url);
const env = { ...parseDotenv(new URL('.env', repoRoot).pathname), ...process.env } as Record<string, string>;

const GATEWAY = env.UNICITY_GATEWAY;
const API_KEY = env.UNICITY_API_KEY || null;
const TRUSTBASE = env.UNICITY_TRUSTBASE ?? 'bft-trustbase.testnet2.json';
if (!GATEWAY) throw new Error('UNICITY_GATEWAY missing from .env');

const NETWORK_ID = Number(env.UNICITY_NETWORK_ID ?? '4');
const AMOUNT = BigInt(env.AMOUNT ?? '1000000'); // 1.000000 (6 decimals)
const NONCE = 7n;

// Demo source-chain addresses (Tron hex, 0x41-prefixed). The mock Tron RPC below
// stands in for a real Nile lock so the mint's bridge backing verifies in-process.
const ASSET_TRON = '410000000000000000000000000000000000000def';
const LOCK_TRON = '410000000000000000000000000000000000000abc';

function log(s = ''): void {
  process.stdout.write(s + '\n');
}

// ---- in-process mock Tron RPC (the only mocked dependency) ------------------
class MockTronRpc implements TronRpc {
  public constructor(
    private readonly info: TronTxInfo,
    private readonly tip: bigint,
  ) {}
  public async getTransactionInfo(): Promise<TronTxInfo | null> {
    return this.info;
  }
  public async getNowBlockNumber(): Promise<bigint> {
    return this.tip;
  }
}

function word(hex: string): string {
  return hex.toLowerCase().padStart(64, '0');
}
function makeLockLog(
  addressEvmHex: string,
  e: { nonce: bigint; fromEvmHex: string; amount: bigint; unicityTokenId: Uint8Array; recipientCommitment: Uint8Array },
): TronLog {
  return {
    address: addressEvmHex.toLowerCase(),
    topics: [LOCK_EVENT_TOPIC0, word(e.nonce.toString(16)), word(e.fromEvmHex)],
    data: word(e.amount.toString(16)) + word(toHex(e.unicityTokenId)) + word(toHex(e.recipientCommitment)),
  };
}

// Read the bridged-coin value using the SDK PaymentAssetCollection format — the
// SAME format the Rust prover decodes (00: token value/coinId). The bespoke
// `encodeBridgedValue` envelope in src/value.ts is CLI-only and is NOT
// cross-stack compatible; the production wallet (sphere) likewise uses the SDK
// payment data. This is why the demo encodes/extracts via the SDK collection.
function extractSdkAmount(data: Uint8Array | null, coinId: Uint8Array): bigint | null {
  if (!data) return null;
  try {
    return PaymentAssetCollection.fromCBOR(data).get(new AssetId(coinId))?.value ?? null;
  } catch {
    return null;
  }
}

function trustBase(): RootTrustBase {
  return RootTrustBase.fromJSON(JSON.parse(readFileSync(new URL(TRUSTBASE, repoRoot).pathname, 'utf8')));
}
function client(): StateTransitionClient {
  return new StateTransitionClient(new AggregatorClient(GATEWAY, API_KEY));
}

async function main(): Promise<void> {
  const tb = trustBase();
  const predicateVerifier = PredicateVerifierService.create();
  const c = client();

  // Owner of the freshly minted token.
  const ownerSigning = new SigningService(SigningService.generatePrivateKey());
  const ownerPredicate = SignaturePredicate.fromSigningService(ownerSigning);
  const recipientCommitmentBytes = recipientCommitment(EncodedPredicate.fromPredicate(ownerPredicate).toCBOR());

  const salt = TokenSalt.fromBytes(crypto.getRandomValues(new Uint8Array(32)));
  const networkId = NetworkId.fromId(NETWORK_ID);
  const tokenId = await TokenId.fromSalt(networkId, salt);

  // Bridge plugin (minter self-trusts its lock: confirmations 0) + mock Nile RPC
  // returning a Lock event that matches this exact token + recipient + amount.
  const lockLog = makeLockLog('', {
    nonce: NONCE,
    fromEvmHex: 'ab'.repeat(20),
    amount: AMOUNT,
    unicityTokenId: tokenId.bytes,
    recipientCommitment: recipientCommitmentBytes,
  });
  const plugin = createTronUsdtBridgePlugin(
    { chainId: TRON_NILE_CHAIN_ID, lockContract: LOCK_TRON, assetContract: ASSET_TRON, confirmations: 0, decimals: 6 },
    { rpc: new MockTronRpc({ blockNumber: 100n, success: true, logs: [lockLog] }, 100n), extractAmount: extractSdkAmount },
  );
  // Fix the mock log address to the plugin's normalized EVM-form lock address.
  lockLog.address = plugin.resolvedConfig.lockContractHex;

  // The bridge config the reason/nullifier bind to (and the prover must use).
  const cfg: BridgeConfig = {
    sourceChainId: BigInt(plugin.resolvedConfig.chainId),
    vault: fromHex(plugin.resolvedConfig.lockContractHex),
    asset: fromHex(plugin.resolvedConfig.assetContractHex),
    tokenType: plugin.resolvedConfig.tokenType,
    coinId: plugin.resolvedConfig.coinId,
    reasonTag: 39048n,
    lockDomain: sha256(new TextEncoder().encode('UNICITY_BR_LOCK')),
    nullifierDomain: sha256(new TextEncoder().encode('UNICITY_BR_NUL')),
  };
  const cfgHash = deriveConfigHash(cfg);

  log(`Aggregator : ${GATEWAY}`);
  log(`tokenType  : ${plugin.tokenTypeHex}`);
  log(`coinId     : ${plugin.coinIdHex}`);
  log(`configHash : ${toHex(cfgHash)}\n`);

  // 1. mint a bridge-lock-genesis token --------------------------------------
  log('1. minting bridge-lock-genesis token (backing verified vs mock Nile)...');
  const justification = new TronUsdtLockJustification({
    chainId: plugin.resolvedConfig.chainId,
    lockContract: fromHex(plugin.resolvedConfig.lockContractHex),
    assetContract: fromHex(plugin.resolvedConfig.assetContractHex),
    txid: crypto.getRandomValues(new Uint8Array(32)),
    logIndex: 0,
    amount: AMOUNT,
    nonce: NONCE,
  }).toCBOR();
  const valueData = PaymentAssetCollection.create(new Asset(new AssetId(cfg.coinId), AMOUNT)).toCBOR();
  const mintTx = await MintTransaction.create(
    networkId,
    ownerPredicate,
    valueData,
    new TokenType(cfg.tokenType),
    salt,
    justification,
  );
  await c.submitCertificationRequest(await CertificationData.fromMintTransaction(mintTx));
  const mintJustVerifier = new MintJustificationVerifierService();
  mintJustVerifier.register(plugin.verifier);
  const certifiedMint = await mintTx.toCertifiedTransaction(
    tb,
    predicateVerifier,
    await waitInclusionProof(c, tb, predicateVerifier, mintTx),
  );
  const token = await Token.mint(tb, predicateVerifier, mintJustVerifier, certifiedMint);
  log(`   minted: ${toHex(token.toCBOR()).length / 2} bytes, value ${extractSdkAmount(valueData, cfg.coinId)}, backing OK\n`);

  // 2. burn it to a BridgeBackReason -----------------------------------------
  const reason: BridgeBackReason = {
    version: 1n,
    recipient: fromHex('00000000000000000000000000000000000000b2'),
    amount: AMOUNT,
    feeRecipient: fromHex('0000000000000000000000000000000000000000'),
    feeAmount: 0n,
    deadline: 1_900_000_000n,
  };
  log('2. burning to BurnPredicate(H(reasonBytes)) with reasonBytes in aux...');
  const { transfer: burnTx, reason: burnReason } = await createBridgeBackBurnTransfer(
    token,
    cfg,
    reason,
    crypto.getRandomValues(new Uint8Array(32)),
  );
  const unlock = await SignaturePredicateUnlockScript.create(burnTx, ownerSigning);
  const resp = await c.submitCertificationRequest(await CertificationData.fromTransaction(burnTx, unlock));
  if (resp.status !== String(CertificationStatus.SUCCESS)) throw new Error(`burn certification failed: ${resp.status}`);
  const burnedToken = await token.transfer(
    tb,
    predicateVerifier,
    await burnTx.toCertifiedTransaction(tb, predicateVerifier, await waitInclusionProof(c, tb, predicateVerifier, burnTx)),
  );
  log(`   burned: ${toHex(burnedToken.toCBOR()).length / 2} bytes, reasonHash ${toHex(burnReason.reasonHash)}\n`);

  // 3. derive nullifier + leaf from the REAL certified burn (00 §5/§7) --------
  const burnStateId = (await StateId.fromTransaction(burnTx)).data;
  const burnTxHash = (await burnTx.calculateTransactionHash()).data;
  const preview = previewReturn(cfgHash, reason, burnStateId, burnTxHash);
  log('3. derived (read-only) return values:');
  log(`   burnStateId      : ${toHex(burnStateId)}`);
  log(`   burnTxHash       : ${toHex(burnTxHash)}`);
  log(`   burnTransitionId : ${toHex(preview.burnTransitionId)}`);
  log(`   nullifier        : ${toHex(preview.nullifier)}\n`);

  // 4. persist for the prover ------------------------------------------------
  const witness = buildWitnessRequest({ tokenCbor: burnedToken.toCBOR(), configHash: cfgHash, reasonBytes: burnReason.reasonBytes });
  const outPath = new URL('.bridge-back-state.json', import.meta.url).pathname;
  writeFileSync(
    outPath,
    JSON.stringify(
      {
        aggregator: GATEWAY,
        networkId: NETWORK_ID,
        // The bridge config the prover must validate this token against:
        proverConfig: {
          sourceChainId: cfg.sourceChainId.toString(),
          vault: toHex(cfg.vault),
          asset: toHex(cfg.asset),
          tokenType: toHex(cfg.tokenType),
          coinId: toHex(cfg.coinId),
          reasonTag: cfg.reasonTag.toString(),
          lockDomain: toHex(cfg.lockDomain),
          nullifierDomain: toHex(cfg.nullifierDomain),
          justificationTag: TronUsdtLockJustification.CBOR_TAG.toString(),
        },
        lock: {
          nonce: NONCE.toString(),
          amount: AMOUNT.toString(),
          tokenId: toHex(tokenId.bytes),
          recipientCommitment: toHex(recipientCommitmentBytes),
          // TS-derived lockDigest the prover's bridge_lock_obligation must reproduce (00 §3)
          lockDigest: toHex(
            deriveLockDigest({
              sourceChainId: cfg.sourceChainId,
              vault: cfg.vault,
              nonce: NONCE,
              asset: cfg.asset,
              tokenType: cfg.tokenType,
              coinId: cfg.coinId,
              amount: AMOUNT,
              unicityTokenId: tokenId.bytes,
              recipientCommitment: recipientCommitmentBytes,
            }),
          ),
        },
        configHash: toHex(cfgHash),
        reasonBytes: toHex(burnReason.reasonBytes),
        reasonHash: toHex(burnReason.reasonHash),
        burnStateId: toHex(burnStateId),
        burnTxHash: toHex(burnTxHash),
        burnTransitionId: toHex(preview.burnTransitionId),
        nullifier: toHex(preview.nullifier),
        returnLeaf: {
          nullifier: toHex(preview.returnLeaf.nullifier),
          recipient: toHex(preview.returnLeaf.recipient),
          amount: preview.returnLeaf.amount.toString(),
          feeRecipient: toHex(preview.returnLeaf.feeRecipient),
          feeAmount: preview.returnLeaf.feeAmount.toString(),
          deadline: preview.returnLeaf.deadline.toString(),
        },
        witnessRequest: { configHash: toHex(witness.configHash), reasonBytes: toHex(witness.reasonBytes) },
        burnedTokenCbor: toHex(burnedToken.toCBOR()),
      },
      null,
      2,
    ),
  );
  log(`4. wrote ${outPath}`);
  log('\n✔ bridge-back e2e complete — a real bridge-lock-genesis burned token + its');
  log('  nullifier/leaf are persisted, consumable by the prover relation.');
}

void main().catch((e) => {
  console.error('\n✘ Error:', e instanceof Error ? (e.stack ?? e.message) : e);
  process.exit(1);
});
