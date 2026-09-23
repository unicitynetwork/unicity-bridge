import { DEFAULT_DECIMALS, type BridgeAssetConfig } from './config.js';
import { BRIDGE_LOCK_JUSTIFICATION_TAG } from './BridgeLockJustification.js';
import { chainFamily } from './families.js';
import { toHex } from './hex.js';
import { deriveCoinId, deriveTokenType } from './identifiers.js';
import { LockMintJustificationVerifier, type ResolvedBridgeConfig } from './LockMintJustificationVerifier.js';
import type { SourceChainRpc } from './source-chain.js';
import type { BridgedAmountExtractor } from './value.js';

export * from './address.js';
export * from './bridge-back/index.js';
export * from './BridgeLockJustification.js';
export * from './BridgeMintJustificationVerifier.js';
export * from './config.js';
export * from './contract-call.js';
export * from './evm/config.js';
export * from './evm/EvmRpcClient.js';
export * from './evm/family.js';
export * from './families.js';
export * from './hex.js';
export * from './identifiers.js';
export * from './lock-event.js';
export * from './LockMintJustificationVerifier.js';
export * from './source-chain.js';
export * from './tron/config.js';
export * from './tron/family.js';
export * from './tron/TronRpcClient.js';
export * from './value.js';

export interface CreateBridgePluginDeps {
  /** Inject a node client (e.g. a mock). If omitted, the family builds one from `config.rpcUrl`. */
  readonly rpc?: SourceChainRpc;
  /** Override the token-value extractor. Defaults to the wallet value format (src/value.ts). */
  readonly extractAmount?: BridgedAmountExtractor;
}

/** A ready-to-register bridge plugin for one bridged asset. */
export interface BridgePlugin {
  readonly cborTag: bigint;
  /** 32-byte Unicity TokenType (hex) for this asset. */
  readonly tokenTypeHex: string;
  /** 32-byte Sphere coinId (hex) for this asset. */
  readonly coinIdHex: string;
  readonly decimals: number;
  readonly resolvedConfig: ResolvedBridgeConfig;
  /** Register this into a MintJustificationVerifierService. */
  readonly verifier: LockMintJustificationVerifier;
  readonly rpc: SourceChainRpc;
}

/**
 * Build a bridge plugin from canonical config. Derives the asset's
 * TokenType/coinId, normalizes trust-anchor addresses through the chain
 * family, and wires the verifier to the family's node client.
 */
export function createBridgePlugin(config: BridgeAssetConfig, deps: CreateBridgePluginDeps = {}): BridgePlugin {
  const family = chainFamily(config.family);
  const tokenType = deriveTokenType(config.family, config.chainId, config.assetContract);
  const coinId = deriveCoinId(config.family, config.chainId, config.assetContract);

  const resolvedConfig: ResolvedBridgeConfig = {
    family: config.family,
    chainId: config.chainId,
    lockContractHex: family.normalizeAddress(config.lockContract),
    assetContractHex: family.normalizeAddress(config.assetContract),
    confirmations: config.confirmations ?? family.defaultConfirmations,
    tokenType,
    coinId,
  };

  const rpc = deps.rpc ?? family.createRpc({ rpcUrl: config.rpcUrl, apiKey: config.apiKey });
  const verifier = new LockMintJustificationVerifier(resolvedConfig, { rpc, extractAmount: deps.extractAmount });

  return {
    cborTag: BRIDGE_LOCK_JUSTIFICATION_TAG,
    tokenTypeHex: toHex(tokenType),
    coinIdHex: toHex(coinId),
    decimals: config.decimals ?? DEFAULT_DECIMALS,
    resolvedConfig,
    verifier,
    rpc,
  };
}
