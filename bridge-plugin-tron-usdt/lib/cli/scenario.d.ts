import { NetworkId } from '@unicitylabs/state-transition-sdk/lib/api/NetworkId.js';
import type { CertifiedMintTransaction } from '@unicitylabs/state-transition-sdk/lib/transaction/CertifiedMintTransaction.js';
import { TronUsdtLockJustification, type TronLog, type TronRpc, type TronTxInfo, type TronUsdtBridgeConfig, type TronUsdtBridgePlugin } from '../index.js';
export declare class MockTronRpc implements TronRpc {
    txInfo: TronTxInfo | null;
    tip: bigint;
    constructor(txInfo: TronTxInfo | null, tip: bigint);
    getTransactionInfo(): Promise<TronTxInfo | null>;
    getNowBlockNumber(): Promise<bigint>;
}
export interface LockEventFields {
    nonce: bigint;
    fromEvmHex: string;
    amount: bigint;
    unicityTokenId: Uint8Array;
    recipientCommitment: Uint8Array;
}
export declare function makeLockLog(addressEvmHex: string, e: LockEventFields): TronLog;
export declare const DEMO_NETWORK: NetworkId;
export declare const DEMO_TXID: string;
export declare const DEMO_NONCE = 7n;
export interface DemoBuild {
    plugin: TronUsdtBridgePlugin;
    certifiedTx: CertifiedMintTransaction;
    rpc: MockTronRpc;
    tokenIdHex: string;
}
/** Builds a valid lock + matching bridged mint, then applies optional tampering. */
export declare function buildDemo(config: TronUsdtBridgeConfig, amount: bigint, confirmations: number, overrides?: {
    logs?: (def: TronLog) => TronLog[];
    justification?: (d: TronUsdtLockJustification['data']) => TronUsdtLockJustification['data'];
    tokenValueAmount?: bigint;
    tip?: bigint;
    blockNumber?: bigint;
}): Promise<DemoBuild>;
export declare function hexToBytes(hex: string): Uint8Array;
