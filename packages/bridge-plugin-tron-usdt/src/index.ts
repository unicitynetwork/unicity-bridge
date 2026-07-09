import {
  DEFAULT_CONFIRMATIONS,
  DEFAULT_DECIMALS,
  type TronUsdtBridgeConfig,
} from './config.js';
import { toHex } from './hex.js';
import { deriveCoinId, deriveTokenType } from './identifiers.js';
import { toEvmAddressHex } from './tron-address.js';
import { TronHttpRpcClient, type TronRpc } from './TronRpcClient.js';
import {
  type ResolvedTronUsdtConfig,
  TronUsdtMintJustificationVerifier,
} from './TronUsdtMintJustificationVerifier.js';
import { TRON_USDT_LOCK_JUSTIFICATION_TAG } from './TronUsdtLockJustification.js';
import type { BridgedAmountExtractor } from './value.js';

export * from './bridge-back/index.js';
export * from './config.js';
export * from './hex.js';
export * from './identifiers.js';
export * from './lock-event.js';
export * from './tron-address.js';
export * from './TronRpcClient.js';
export * from './TronUsdtLockJustification.js';
export * from './TronUsdtMintJustificationVerifier.js';
export * from './value.js';

export interface CreateTronUsdtBridgePluginDeps {
  /** Inject a TronRpc (e.g. a mock). If omitted, an HTTP client is built from config.rpcUrl. */
  readonly rpc?: TronRpc;
  /** Override the token-value extractor. Defaults to bare SDK PaymentAssetCollection CBOR. */
  readonly extractAmount?: BridgedAmountExtractor;
}

/** A ready-to-register bridge plugin for one Tron-bridged asset. */
export interface TronUsdtBridgePlugin {
  readonly cborTag: bigint;
  /** 32-byte Unicity TokenType (hex) for this asset. */
  readonly tokenTypeHex: string;
  /** 32-byte Sphere coinId (hex) for this asset. */
  readonly coinIdHex: string;
  readonly decimals: number;
  readonly resolvedConfig: ResolvedTronUsdtConfig;
  /** Register this into a MintJustificationVerifierService. */
  readonly verifier: TronUsdtMintJustificationVerifier;
}

/**
 * Build a Tron-USDT bridge plugin from canonical config. Derives the asset's
 * TokenType/coinId, normalizes trust-anchor addresses, and wires the verifier.
 */
export function createTronUsdtBridgePlugin(
  config: TronUsdtBridgeConfig,
  deps: CreateTronUsdtBridgePluginDeps = {},
): TronUsdtBridgePlugin {
  const tokenType = deriveTokenType(config.chainId, config.assetContract);
  const coinId = deriveCoinId(config.chainId, config.assetContract);

  const resolvedConfig: ResolvedTronUsdtConfig = {
    chainId: config.chainId,
    lockContractHex: toEvmAddressHex(config.lockContract),
    assetContractHex: toEvmAddressHex(config.assetContract),
    confirmations: config.confirmations ?? DEFAULT_CONFIRMATIONS,
    tokenType,
    coinId,
  };

  const rpc =
    deps.rpc ??
    new TronHttpRpcClient({
      baseUrl: config.rpcUrl ?? 'https://api.trongrid.io',
      apiKey: config.apiKey,
    });

  const verifier = new TronUsdtMintJustificationVerifier(resolvedConfig, {
    rpc,
    extractAmount: deps.extractAmount,
  });

  return {
    cborTag: TRON_USDT_LOCK_JUSTIFICATION_TAG,
    tokenTypeHex: toHex(tokenType),
    coinIdHex: toHex(coinId),
    decimals: config.decimals ?? DEFAULT_DECIMALS,
    resolvedConfig,
    verifier,
  };
}
