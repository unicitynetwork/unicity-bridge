/**
 * Bridge-in **source adapter** (08 Phase 4 — the three-boundary abstraction).
 *
 * The orchestrator (Sphere) is chain-neutral: it runs an ordered list of opaque
 * {DepositStep}s, waits for receipts, guards account/network, persists recovery,
 * and mints. Everything chain-specific — how many signatures a deposit takes,
 * whether an ERC-20 `approve` is needed, how the commit (lock) event decodes, and
 * how the mint justification is built — lives behind {BridgeSourceAdapter}.
 *
 * One adapter serves every family: the deposit is the same two calls on the
 * same vault ABI, the lock event decodes the same way, and the signer the
 * wallet hands in encodes the calls for its chain.
 */
import type { BridgeSourceAdapter, CommitInfo, DepositRecovery, DepositStep } from '@unicitylabs/bridge-core';

import { BridgeLockJustification } from '../BridgeLockJustification.js';
import type { ContractCall } from '../contract-call.js';
import { fromHex } from '../hex.js';
import type { CreateBridgePluginDeps } from '../index.js';
import { decodeLockEvent } from '../lock-event.js';
import type { ConstantCaller, SourceTxInfo } from '../source-chain.js';
import { encodeBridgePaymentData } from '../value.js';
import { queryAllowance } from './allowance.js';
import { buildBridgeInPlan } from './facade.js';
import { chainName, type LoadedBridge } from './manifest.js';
import { selfMintVerifier } from './self-mint-verifier.js';

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

/** The wallet capability a deposit step needs. */
export interface DepositWallet {
  getAddress(): Promise<string>;
  sendCall(call: ContractCall): Promise<string>;
}

/** The node read the adapter needs (allowance). */
export type AllowanceReader = ConstantCaller;

/**
 * Build the {BridgeSourceAdapter} for a loaded bridge. Closes over the wallet
 * (for the deposit steps) and a node client (for the allowance read); the mint
 * verifier override is built with `deps`; by default it reads the wallet's value
 * payload, which is the production bridge token value format.
 */
export function createSourceAdapter(
  bridge: LoadedBridge,
  wallet: DepositWallet,
  rpc: AllowanceReader,
  deps: CreateBridgePluginDeps = {},
): BridgeSourceAdapter {
  const cfg = bridge.plugin.resolvedConfig;
  const vaultHex = cfg.lockContractHex;
  const where = `${bridge.manifest.symbol} on ${chainName(bridge)}`;

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
        label: `Lock ${where}…`,
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
        label: `Approve ${where}…`,
        awaitReceipt: true,
        send: () => wallet.sendCall(plan.approve),
      };
      return { recovery, steps: [approveStep, lockStep], commitIndex: 1 };
    },

    decodeCommit(rawReceipt) {
      const info = rawReceipt as SourceTxInfo | null;
      if (!info) return null;
      const logIndex = info.logs.findIndex((l) => l.address.toLowerCase() === vaultHex);
      const decoded = logIndex >= 0 ? decodeLockEvent(info.logs[logIndex]) : null;
      if (!decoded) return null;
      return { nonce: decoded.nonce, blockNumber: info.blockNumber, logIndex };
    },

    buildMintRequest({ saltHex, amount, commit, commitTxid }) {
      const genesisReason = new BridgeLockJustification({
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
        mintData: encodeBridgePaymentData(cfg.coinId, amount),
        tokenType: cfg.tokenType,
        salt: fromHex(saltHex),
        genesisReason,
        mintJustificationVerifiers: [selfMintVerifier(bridge, deps)],
      };
    },
  };
}
