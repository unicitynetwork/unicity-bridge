#!/usr/bin/env node
/**
 * Live bridge-BACK e2e (the M2 integration backstop, 02 §"Testing"):
 *
 *   1. mint   — mint a bridged-asset Unicity token to a fresh owner on the live
 *               testnet2 aggregator (bridge tokenType + coinId value envelope).
 *   2. burn   — burn it to a canonical `BridgeBackReason`: the terminal transfer
 *               carries `reasonBytes` in its aux data and its recipient predicate
 *               is `BurnPredicate(H(reasonBytes))` (00 §4); certify on the aggregator.
 *   3. derive — from the REAL certified burn (stateId + tx hash) derive the
 *               nullifier + return leaf exactly as the prover will (00 §5/§7).
 *   4. persist— write the burned-token blob (release-authorizing recovery
 *               material) + the prover witness-request envelope to disk.
 *
 * Uses the root `.env` (UNICITY_GATEWAY / UNICITY_API_KEY / UNICITY_TRUSTBASE).
 * Reads/writes nothing on Tron: the genesis backing reason (E3) is exercised by
 * the prover's own token fixture; here we validate the Unicity-side burn path and
 * that the wallet-derived nullifier/leaf are computed from real certificates.
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
import { PredicateVerifierService } from '@unicitylabs/state-transition-sdk/lib/predicate/verification/PredicateVerifierService.js';
import { StateTransitionClient } from '@unicitylabs/state-transition-sdk/lib/StateTransitionClient.js';
import { MintTransaction } from '@unicitylabs/state-transition-sdk/lib/transaction/MintTransaction.js';
import { Token } from '@unicitylabs/state-transition-sdk/lib/transaction/Token.js';
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
  decodeBridgedValue,
  deriveCoinId,
  deriveTokenType,
  encodeBridgedValue,
  fromHex,
  previewReturn,
  toHex,
  TRON_NILE_CHAIN_ID,
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
    if ((v.startsWith('"') && v.endsWith('"')) || (v.startsWith("'") && v.endsWith("'"))) {
      v = v.slice(1, -1);
    }
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

function log(s = ''): void {
  process.stdout.write(s + '\n');
}

// ---- the deployment config the reason/nullifier bind to (00 §2) ------------
// Demo addresses for the source-chain vault/asset; the tokenType/coinId are the
// real deterministic derivations the plugin uses.
const ASSET_EVM = 'a614f803b6fd780986a42c78ec9c7f77e6ded13c';
const VAULT_EVM = '00000000000000000000000000000000000000a1';
const TRON_CHAIN = TRON_NILE_CHAIN_ID;

function bridgeConfig(): BridgeConfig {
  return {
    sourceChainId: BigInt(TRON_CHAIN),
    vault: fromHex(VAULT_EVM),
    asset: fromHex(ASSET_EVM),
    tokenType: deriveTokenType(TRON_CHAIN, ASSET_EVM),
    coinId: deriveCoinId(TRON_CHAIN, ASSET_EVM),
    reasonTag: 39050n,
    lockDomain: sha256(new TextEncoder().encode('UNICITY_BR_LOCK')),
    nullifierDomain: sha256(new TextEncoder().encode('UNICITY_BR_NUL')),
  };
}

function trustBase(): RootTrustBase {
  const path = new URL(TRUSTBASE, repoRoot).pathname;
  return RootTrustBase.fromJSON(JSON.parse(readFileSync(path, 'utf8')));
}

function client(): StateTransitionClient {
  return new StateTransitionClient(new AggregatorClient(GATEWAY, API_KEY));
}

async function main(): Promise<void> {
  const cfg = bridgeConfig();
  const cfgHash = deriveConfigHash(cfg);
  const tb = trustBase();
  const predicateVerifier = PredicateVerifierService.create();
  const c = client();

  log(`Aggregator : ${GATEWAY}`);
  log(`Network    : ${NETWORK_ID}`);
  log(`tokenType  : ${toHex(cfg.tokenType)}`);
  log(`coinId     : ${toHex(cfg.coinId)}`);
  log(`configHash : ${toHex(cfgHash)}\n`);

  // 1. mint a bridge-typed token to a fresh owner ----------------------------
  const ownerPriv = SigningService.generatePrivateKey();
  const ownerSigning = new SigningService(ownerPriv);
  const ownerPredicate = SignaturePredicate.fromSigningService(ownerSigning);
  const salt = TokenSalt.fromBytes(crypto.getRandomValues(new Uint8Array(32)));
  const networkId = NetworkId.fromId(NETWORK_ID);
  const tokenType = new TokenType(cfg.tokenType);
  const valueData = encodeBridgedValue(cfg.coinId, AMOUNT);

  log('1. minting bridged token (null justification — Unicity-side demo)...');
  const mintTx = await MintTransaction.create(networkId, ownerPredicate, valueData, tokenType, salt, null);
  await c.submitCertificationRequest(await CertificationData.fromMintTransaction(mintTx));
  const certifiedMint = await mintTx.toCertifiedTransaction(
    tb,
    predicateVerifier,
    await waitInclusionProof(c, tb, predicateVerifier, mintTx),
  );
  const token = await Token.mint(tb, predicateVerifier, new MintJustificationVerifierService(), certifiedMint);
  log(`   minted: ${toHex(token.toCBOR()).length / 2} bytes, value ${decodeBridgedValue(valueData, cfg.coinId)}\n`);

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
  const stateMask = crypto.getRandomValues(new Uint8Array(32));
  const { transfer: burnTx, reason: burnReason } = await createBridgeBackBurnTransfer(token, cfg, reason, stateMask);
  const unlock = await SignaturePredicateUnlockScript.create(burnTx, ownerSigning);
  const resp = await c.submitCertificationRequest(await CertificationData.fromTransaction(burnTx, unlock));
  if (resp.status !== String(CertificationStatus.SUCCESS)) {
    throw new Error(`burn certification failed: ${resp.status}`);
  }
  const burnedToken = await token.transfer(
    tb,
    predicateVerifier,
    await burnTx.toCertifiedTransaction(
      tb,
      predicateVerifier,
      await waitInclusionProof(c, tb, predicateVerifier, burnTx),
    ),
  );
  log(`   burned: ${toHex(burnedToken.toCBOR()).length / 2} bytes`);
  log(`   reasonHash: ${toHex(burnReason.reasonHash)}\n`);

  // 3. derive nullifier + leaf from the REAL certified burn (00 §5/§7) --------
  const burnStateId = (await StateId.fromTransaction(burnTx)).data;
  const burnTxHash = (await burnTx.calculateTransactionHash()).data;
  const preview = previewReturn(cfgHash, reason, burnStateId, burnTxHash);

  log('3. derived (read-only) return values:');
  log(`   burnStateId      : ${toHex(burnStateId)}`);
  log(`   burnTxHash       : ${toHex(burnTxHash)}`);
  log(`   burnTransitionId : ${toHex(preview.burnTransitionId)}`);
  log(`   nullifier        : ${toHex(preview.nullifier)}`);
  log(`   returnLeaf.amount: ${preview.returnLeaf.amount}\n`);

  // 4. persist the burned blob + prover witness request ----------------------
  const witness = buildWitnessRequest({
    tokenCbor: burnedToken.toCBOR(),
    configHash: cfgHash,
    reasonBytes: burnReason.reasonBytes,
  });
  const outPath = new URL('.bridge-back-state.json', import.meta.url).pathname;
  writeFileSync(
    outPath,
    JSON.stringify(
      {
        aggregator: GATEWAY,
        networkId: NETWORK_ID,
        configHash: toHex(cfgHash),
        tokenType: toHex(cfg.tokenType),
        coinId: toHex(cfg.coinId),
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
        witnessRequest: { tokenCbor: toHex(witness.tokenCbor), configHash: toHex(witness.configHash) },
        burnedTokenCbor: toHex(burnedToken.toCBOR()),
      },
      null,
      2,
    ),
  );
  log(`4. wrote ${outPath}`);
  log('\n✔ bridge-back e2e complete — a real burned token + its nullifier/leaf are persisted.');
}

void main().catch((e) => {
  console.error('\n✘ Error:', e instanceof Error ? (e.stack ?? e.message) : e);
  process.exit(1);
});
