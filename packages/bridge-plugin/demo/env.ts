import { TRON_NILE_CHAIN_ID } from '../src/index.js';

/** Reads demo configuration from the environment, with testnet defaults. */
export interface DemoEnv {
  // Tron (Nile testnet)
  tronRpc: string;
  tronApiKey?: string;
  tronPrivateKey?: string;
  tronChainId: number;
  /** Confirmations an independent verifier/receiver requires for source finality. */
  confirmations: number;
  /** Confirmations the minter requires — it trusts its own lock, so 0 (in a block). */
  mintConfirmations: number;
  /** How long the receiver keeps retrying while awaiting source finality (ms). */
  verifyTimeoutMs: number;
  /** Delay between receiver finality retries (ms). */
  verifyRetryMs: number;
  // Bridge amount (USDT, 6 decimals)
  amount: bigint;
  // Unicity (testnet2)
  aggregatorUrl: string;
  aggregatorApiKey?: string;
  networkId: number;
  trustBaseUrl: string;
}

export function readEnv(): DemoEnv {
  return {
    tronRpc: process.env.TRON_RPC ?? 'https://nile.trongrid.io',
    tronApiKey: process.env.TRON_API_KEY || undefined,
    tronPrivateKey: process.env.TRON_PRIVATE_KEY || undefined,
    tronChainId: Number(process.env.TRON_CHAIN_ID ?? TRON_NILE_CHAIN_ID),
    confirmations: Number(process.env.CONFIRMATIONS ?? 20),
    mintConfirmations: Number(process.env.MINT_CONFIRMATIONS ?? 0),
    verifyTimeoutMs: Number(process.env.VERIFY_TIMEOUT_MS ?? 300_000),
    verifyRetryMs: Number(process.env.VERIFY_RETRY_MS ?? 6_000),
    amount: BigInt(process.env.AMOUNT ?? '1000000'), // 1.000000 USDT
    aggregatorUrl: process.env.UNICITY_AGGREGATOR ?? 'https://gateway.testnet2.unicity.network',
    aggregatorApiKey: process.env.UNICITY_API_KEY || undefined,
    networkId: Number(process.env.UNICITY_NETWORK_ID ?? '4'),
    trustBaseUrl:
      process.env.UNICITY_TRUSTBASE_URL ??
      'https://raw.githubusercontent.com/unicitynetwork/unicity-ids/refs/heads/main/bft-trustbase.testnet2.json',
  };
}

export function requireTronKey(env: DemoEnv): string {
  if (!env.tronPrivateKey) {
    throw new Error('TRON_PRIVATE_KEY is required for this step (a Nile-testnet account funded with test TRX).');
  }
  return env.tronPrivateKey;
}
