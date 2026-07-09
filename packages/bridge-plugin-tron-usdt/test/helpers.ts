import { NetworkId } from '@unicitylabs/state-transition-sdk/lib/api/NetworkId.js';
import { SigningService } from '@unicitylabs/state-transition-sdk/lib/crypto/secp256k1/SigningService.js';
import { EncodedPredicate } from '@unicitylabs/state-transition-sdk/lib/predicate/EncodedPredicate.js';
import { SignaturePredicate } from '@unicitylabs/state-transition-sdk/lib/predicate/builtin/SignaturePredicate.js';
import type { CertifiedMintTransaction } from '@unicitylabs/state-transition-sdk/lib/transaction/CertifiedMintTransaction.js';
import { MintTransaction } from '@unicitylabs/state-transition-sdk/lib/transaction/MintTransaction.js';
import { TokenId } from '@unicitylabs/state-transition-sdk/lib/transaction/TokenId.js';
import { TokenSalt } from '@unicitylabs/state-transition-sdk/lib/transaction/TokenSalt.js';
import { TokenType } from '@unicitylabs/state-transition-sdk/lib/transaction/TokenType.js';

import {
  createTronUsdtBridgePlugin,
  LOCK_EVENT_TOPIC0,
  recipientCommitment,
  toEvmAddressHex,
  toHex,
  TronUsdtLockJustification,
  type TronLog,
  type TronRpc,
  type TronTxInfo,
  type TronUsdtBridgeConfig,
  TRON_NILE_CHAIN_ID,
  encodeBridgePaymentData,
} from '../src/index.js';

export const LOCK_CONTRACT = '410000000000000000000000000000000000000abc';
export const USDT_CONTRACT = '410000000000000000000000000000000000000def';
export const TXID = '11'.repeat(32);
export const AMOUNT = 1_000_000n;
export const NONCE = 7n;
export const CONFIRMATIONS = 20;

export const NETWORK = NetworkId.fromId(4);

export const CONFIG: TronUsdtBridgeConfig = {
  chainId: TRON_NILE_CHAIN_ID,
  lockContract: LOCK_CONTRACT,
  assetContract: USDT_CONTRACT,
  confirmations: CONFIRMATIONS,
  decimals: 6,
};

export class MockTronRpc implements TronRpc {
  public constructor(
    public txInfo: TronTxInfo | null,
    public tip: bigint,
  ) {}

  public async getTransactionInfo(): Promise<TronTxInfo | null> {
    return this.txInfo;
  }

  public async getNowBlockNumber(): Promise<bigint> {
    return this.tip;
  }
}

function wordFromBigInt(v: bigint): string {
  return v.toString(16).padStart(64, '0');
}

function wordFromBytes(b: Uint8Array): string {
  return toHex(b).padStart(64, '0');
}

export interface LockEventFields {
  nonce: bigint;
  fromEvmHex: string; // 20-byte hex
  amount: bigint;
  unicityTokenId: Uint8Array; // 32 bytes
  recipientCommitment: Uint8Array; // 32 bytes
}

export function makeLockLog(addressEvmHex: string, e: LockEventFields): TronLog {
  return {
    address: addressEvmHex.toLowerCase(),
    topics: [LOCK_EVENT_TOPIC0, wordFromBigInt(e.nonce), e.fromEvmHex.toLowerCase().padStart(64, '0')],
    data: wordFromBigInt(e.amount) + wordFromBytes(e.unicityTokenId) + wordFromBytes(e.recipientCommitment),
  };
}

export interface Scenario {
  plugin: ReturnType<typeof createTronUsdtBridgePlugin>;
  rpc: MockTronRpc;
  certifiedTx: CertifiedMintTransaction;
  tokenId: TokenId;
  recipientCommitmentBytes: Uint8Array;
}

/**
 * Build a fully valid bridged-token mint + matching mocked Tron lock. Overrides
 * let individual tests tamper with exactly one dimension.
 */
export async function buildScenario(
  overrides: {
    blockNumber?: bigint;
    tip?: bigint;
    success?: boolean;
    logs?: (defaultLog: TronLog) => TronLog[];
    justification?: (d: TronUsdtLockJustification['data']) => TronUsdtLockJustification['data'];
    tokenValueAmount?: bigint | null; // null => omit value envelope
    tokenType?: Uint8Array;
  } = {},
): Promise<Scenario> {
  const plugin = createTronUsdtBridgePlugin(CONFIG, { rpc: new MockTronRpc(null, 0n) });

  const recipientKey = SigningService.generate();
  const recipient = SignaturePredicate.create(recipientKey.publicKey);
  const recipientCbor = EncodedPredicate.fromPredicate(recipient).toCBOR();
  const recipientCommitmentBytes = recipientCommitment(recipientCbor);

  const salt = TokenSalt.generate();
  const tokenId = await TokenId.fromSalt(NETWORK, salt);

  const blockNumber = overrides.blockNumber ?? 100n;
  const tip = overrides.tip ?? blockNumber + BigInt(CONFIRMATIONS);

  const defaultLog = makeLockLog(plugin.resolvedConfig.lockContractHex, {
    nonce: NONCE,
    fromEvmHex: 'ab'.repeat(20),
    amount: AMOUNT,
    unicityTokenId: tokenId.bytes,
    recipientCommitment: recipientCommitmentBytes,
  });

  const logs = overrides.logs ? overrides.logs(defaultLog) : [defaultLog];
  const rpc = new MockTronRpc(
    { blockNumber, success: overrides.success ?? true, logs },
    tip,
  );
  // rebuild plugin with this rpc
  const wired = createTronUsdtBridgePlugin(CONFIG, { rpc });

  let jData: TronUsdtLockJustification['data'] = {
    chainId: CONFIG.chainId,
    lockContract: hexToBytes(plugin.resolvedConfig.lockContractHex),
    assetContract: hexToBytes(plugin.resolvedConfig.assetContractHex),
    txid: hexToBytes(TXID),
    logIndex: 0,
    amount: AMOUNT,
    nonce: NONCE,
  };
  if (overrides.justification) {
    jData = overrides.justification(jData);
  }
  const justification = new TronUsdtLockJustification(jData);

  const valueAmount = overrides.tokenValueAmount === undefined ? AMOUNT : overrides.tokenValueAmount;
  const valueData = valueAmount === null ? null : encodeBridgePaymentData(wired.resolvedConfig.coinId, valueAmount);

  const tokenType = new TokenType(overrides.tokenType ?? wired.resolvedConfig.tokenType);

  const mint = await MintTransaction.create(
    NETWORK,
    recipient,
    valueData,
    tokenType,
    salt,
    justification.toCBOR(),
  );

  return {
    plugin: wired,
    rpc,
    certifiedTx: mint as unknown as CertifiedMintTransaction,
    tokenId,
    recipientCommitmentBytes,
  };
}

export function hexToBytes(hex: string): Uint8Array {
  const h = hex.startsWith('0x') ? hex.slice(2) : hex;
  const out = new Uint8Array(h.length / 2);
  for (let i = 0; i < out.length; i++) {
    out[i] = parseInt(h.slice(i * 2, i * 2 + 2), 16);
  }
  return out;
}

export { toEvmAddressHex };
