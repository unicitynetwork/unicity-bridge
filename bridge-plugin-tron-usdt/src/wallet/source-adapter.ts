/**
 * Bridge-in **source adapter** (08 Phase 4 — the three-boundary abstraction).
 *
 * The orchestrator (Sphere) is chain-neutral: it runs an ordered list of opaque
 * {DepositStep}s, waits for receipts, guards account/network, persists recovery,
 * and mints. Everything chain-specific — how many signatures a deposit takes,
 * whether an ERC-20 `approve` is needed, how the commit (lock) event decodes, and
 * how the mint justification is built — lives behind {BridgeSourceAdapter}.
 *
 * This is the Tron/USDT implementation. A second chain (EVM, or an EIP-3009
 * single-signature deposit) implements the same interface and flows through the
 * unchanged Sphere orchestration — that is the exit criterion the interface
 * exists to satisfy.
 */
import type { CreateTronUsdtBridgePluginDeps } from '../index.js';
import { fromHex } from '../hex.js';
import { decodeLockEvent } from '../lock-event.js';
import type { TronTxInfo } from '../TronRpcClient.js';
import { TronUsdtLockJustification } from '../TronUsdtLockJustification.js';
import { queryAllowance } from './allowance.js';
import { buildBridgeInPlan, type TronCall } from './facade.js';
import type { LoadedBridge } from './manifest.js';
import { buildSelfMintVerifierService } from './self-mint-verifier.js';

// The chain-neutral adapter contract + its DTOs now live in @unicitylabs/bridge-core
// (08 Phase 4 item 1). Re-exported here so importers of the wallet surface keep
// resolving them; `createTronSourceAdapter` implements the core `BridgeSourceAdapter`.
import type {
  BridgeSourceAdapter,
  CommitInfo,
  DepositRecovery,
  DepositStep,
} from '@unicitylabs/bridge-core';
export type {
  BridgeSourceAdapter,
  CommitInfo,
  DepositParams,
  DepositRecovery,
  DepositStep,
  MintRequest,
  MintRequestArgs,
  PreparedDeposit,
} from '@unicitylabs/bridge-core';

/** The wallet capability a Tron deposit step needs. */
export interface DepositWallet {
  getAddress(): Promise<string>;
  sendCall(call: TronCall): Promise<string>;
}

/** The node read the Tron adapter needs (allowance). */
export interface AllowanceReader {
  triggerConstantContract(input: {
    ownerHex: string;
    contractHex: string;
    functionSelector: string;
    parameterHex?: string;
  }): Promise<string>;
}

/**
 * Build the Tron/USDT {BridgeSourceAdapter}. Closes over the wallet (for the
 * deposit steps) and a node client (for the allowance read); the mint verifier
 * override is built with `deps` (Sphere injects the SpherePaymentData extractor).
 */
export function createTronSourceAdapter(
  bridge: LoadedBridge,
  wallet: DepositWallet,
  rpc: AllowanceReader,
  deps: CreateTronUsdtBridgePluginDeps = {},
): BridgeSourceAdapter {
  const cfg = bridge.plugin.resolvedConfig;
  const vaultHex = cfg.lockContractHex;

  return {
    async prepareDeposit(params) {
      const plan = await buildBridgeInPlan({
        plugin: bridge.plugin,
        amount: params.amount,
        networkId: params.networkId,
        recipientPubkey: params.recipientPubkey,
        ownerPredicateCbor: params.ownerPredicateCbor,
        approveAmount: params.approveAmount,
      });
      const recovery: DepositRecovery = {
        tokenIdHex: plan.tokenIdHex,
        saltHex: plan.saltHex,
        recipientCommitmentHex: plan.recipientCommitmentHex,
        coinIdHex: bridge.plugin.coinIdHex,
        tokenTypeHex: bridge.plugin.tokenTypeHex,
        chainId: bridge.manifest.chainId,
      };
      const lockStep: DepositStep = {
        label: 'Lock USDT on Tron…',
        awaitReceipt: false, // the orchestrator waits for the commit receipt to decode it
        send: () => wallet.sendCall(plan.lock),
      };

      // Skip the approval when the vault's allowance already covers the amount
      // (08 §1.1). A read failure is treated as "approve" — a redundant approval
      // is safe, a skipped-but-needed one is not.
      let needApprove = true;
      try {
        const owner = await wallet.getAddress();
        const allowance = await queryAllowance(rpc, {
          assetAddress: cfg.assetContractHex,
          owner,
          spender: cfg.lockContractHex,
        });
        needApprove = allowance < params.amount;
      } catch {
        needApprove = true;
      }
      if (!needApprove) {
        return { recovery, steps: [lockStep], commitIndex: 0 };
      }
      const approveStep: DepositStep = {
        label: 'Approve USDT on Tron…',
        awaitReceipt: true,
        send: () => wallet.sendCall(plan.approve),
      };
      return { recovery, steps: [approveStep, lockStep], commitIndex: 1 };
    },

    decodeCommit(rawReceipt) {
      const info = rawReceipt as TronTxInfo | null;
      if (!info) return null;
      const logIndex = info.logs.findIndex((l) => l.address.toLowerCase() === vaultHex);
      const decoded = logIndex >= 0 ? decodeLockEvent(info.logs[logIndex]) : null;
      if (!decoded) return null;
      return { nonce: decoded.nonce, blockNumber: info.blockNumber, logIndex };
    },

    buildMintRequest({ saltHex, amount, commit, commitTxid }) {
      const genesisReason = new TronUsdtLockJustification({
        chainId: bridge.manifest.chainId,
        lockContract: fromHex(cfg.lockContractHex),
        assetContract: fromHex(cfg.assetContractHex),
        txid: fromHex(commitTxid),
        logIndex: commit.logIndex,
        amount,
        nonce: commit.nonce,
      }).toCBOR();
      return {
        coinIdHex: bridge.plugin.coinIdHex,
        amount,
        tokenType: cfg.tokenType,
        salt: fromHex(saltHex),
        genesisReason,
        mintJustificationVerifierOverride: buildSelfMintVerifierService(bridge, deps),
      };
    },
  };
}
