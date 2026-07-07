/**
 * Wallet-facing façade (06 §W0) — the only surface Sphere calls.
 *
 * Boundary (decision #2): Sphere never recomputes a bridge hash or touches a
 * Tron-specific detail. It loads a {BridgeManifest}, asks this façade to derive
 * the bridge-in plan / build the bridge-back artifacts, and renders UI. Every
 * value here is either a pure derivation (conformance-tested against
 * `protocol/vectors`) or a thin wrapper over an already-built bridge-back function.
 */
import { NetworkId } from '@unicitylabs/state-transition-sdk/lib/api/NetworkId.js';
import { EncodedPredicate } from '@unicitylabs/state-transition-sdk/lib/predicate/EncodedPredicate.js';
import { SignaturePredicate } from '@unicitylabs/state-transition-sdk/lib/predicate/builtin/SignaturePredicate.js';
import { TokenId } from '@unicitylabs/state-transition-sdk/lib/transaction/TokenId.js';
import { TokenSalt } from '@unicitylabs/state-transition-sdk/lib/transaction/TokenSalt.js';
import type { Token } from '@unicitylabs/state-transition-sdk/lib/transaction/Token.js';

import {
  type BridgeBackReason,
  type BridgeConfig,
} from '../bridge-back/derivations.js';
import {
  type BridgeBackBurnReason,
  buildBridgeBackBurnReason,
  buildWitnessRequest,
  createBridgeBackBurnTransfer,
  previewReturn,
  type ReturnPreview,
  type WitnessRequest,
} from '../bridge-back/burn.js';
import { recipientCommitment } from '../identifiers.js';
import { fromHex, toHex } from '../hex.js';
import type { TronUsdtBridgePlugin } from '../index.js';

export * from './manifest.js';
// Pure bridge-back reason builder (reasonBytes + reasonHash) — the app feeds these
// to the engine's bridgeBurn, no SDK token needed (06 §A1.2).
export { buildBridgeBackBurnReason } from '../bridge-back/burn.js';
export type { BridgeBackBurnReason } from '../bridge-back/burn.js';
// Read-only bridge-back surface the UI renders directly.
export { previewReturn } from '../bridge-back/burn.js';
export { decodeBridgeBackReason } from '../bridge-back/derivations.js';
export type {
  BridgeBackReason,
  BridgeConfig,
  DecodedBridgeBackReason,
  ReturnLeaf,
} from '../bridge-back/derivations.js';

/** A typed Tron contract-call parameter (TronWeb `triggerSmartContract` form). */
export interface TronCallParam {
  readonly type: string;
  readonly value: string;
}

/**
 * A single unsigned Tron contract call the wallet's {TronSigner} signs +
 * broadcasts. Shaped for TronWeb `transactionBuilder.triggerSmartContract`
 * (functionSignature + typed parameters) so the signer needs no ABI knowledge.
 */
export interface TronCall {
  /** Target contract, Tron `41…` hex address. */
  readonly contractHex: string;
  /** Solidity function signature, e.g. `lock(uint256,bytes32,bytes32)`. */
  readonly functionSignature: string;
  /** Typed parameters in declaration order. */
  readonly parameters: readonly TronCallParam[];
}

/** Everything bridge-in needs after the user picks asset + amount (06 §A1.1). */
export interface BridgeInPlan {
  /** The Unicity TokenId this deposit funds (hex). The lock commits to it. */
  readonly tokenIdHex: string;
  /** Salt the wallet keeps to mint exactly this token (hex, 32 bytes). */
  readonly saltHex: string;
  /** `SHA256(ownerPredicateCbor)` committed on Tron (hex, 32 bytes). */
  readonly recipientCommitmentHex: string;
  /** Locked amount in the asset's smallest unit. */
  readonly amount: bigint;
  /** One-time max `approve` of the asset to the vault (skip if allowance already covers `amount`). */
  readonly approve: TronCall;
  /** `lock(amount, tokenId, recipientCommitment)` on the vault. */
  readonly lock: TronCall;
}

/** Inputs to {buildBridgeInPlan}. Provide exactly one of `recipientPubkey` / `ownerPredicateCbor`. */
export interface BridgeInPlanInput {
  readonly plugin: TronUsdtBridgePlugin;
  readonly amount: bigint;
  /** Network id of the Unicity network the token is minted on (e.g. testnet2 = 4). */
  readonly networkId: number;
  /**
   * The wallet's 33-byte compressed chain pubkey. The façade builds the same
   * `SignaturePredicate(pubkey)` the engine mints to, so the lock's
   * `recipientCommitment` binds the bridged token to this wallet. Preferred — the
   * app never needs the SDK.
   */
  readonly recipientPubkey?: Uint8Array;
  /** Alternative: a pre-encoded owner `EncodedPredicate` CBOR (commitment = SHA256 of it). */
  readonly ownerPredicateCbor?: Uint8Array;
  /**
   * `approve` amount. A one-time `MAX_UINT256` reduces repeat bridges to a single
   * prompt (06 §A1.3); default exact `amount`.
   */
  readonly approveAmount?: bigint;
}

const MAX_UINT256 = (1n << 256n) - 1n;

/**
 * Derive the bridge-in target token + the unsigned Tron `approve`/`lock` calls.
 * Pure + offline (no RPC): the order is load-bearing — the lock commits to this
 * exact `tokenId` + `recipientCommitment`, and the wallet then mints *that* token.
 */
export async function buildBridgeInPlan(input: BridgeInPlanInput): Promise<BridgeInPlan> {
  const saltBytes = crypto.getRandomValues(new Uint8Array(32));
  const salt = TokenSalt.fromBytes(saltBytes);
  const tokenId = await TokenId.fromSalt(NetworkId.fromId(input.networkId), salt);

  const ownerCbor =
    input.ownerPredicateCbor ??
    (input.recipientPubkey
      ? EncodedPredicate.fromPredicate(SignaturePredicate.create(input.recipientPubkey)).toCBOR()
      : undefined);
  if (!ownerCbor) {
    throw new Error('buildBridgeInPlan: provide recipientPubkey or ownerPredicateCbor');
  }
  const commitment = recipientCommitment(ownerCbor);

  // Tron `41…` hex addresses (the form TronWeb's address-typed params accept).
  const vaultTron = '41' + input.plugin.resolvedConfig.lockContractHex;
  const assetTron = '41' + input.plugin.resolvedConfig.assetContractHex;
  const approveAmount = input.approveAmount ?? input.amount;

  return {
    tokenIdHex: toHex(tokenId.bytes),
    saltHex: toHex(saltBytes),
    recipientCommitmentHex: toHex(commitment),
    amount: input.amount,
    approve: {
      contractHex: assetTron,
      functionSignature: 'approve(address,uint256)',
      parameters: [
        { type: 'address', value: vaultTron },
        { type: 'uint256', value: approveAmount.toString() },
      ],
    },
    lock: {
      contractHex: vaultTron,
      functionSignature: 'lock(uint256,bytes32,bytes32)',
      parameters: [
        { type: 'uint256', value: input.amount.toString() },
        { type: 'bytes32', value: '0x' + toHex(tokenId.bytes) },
        { type: 'bytes32', value: '0x' + toHex(commitment) },
      ],
    },
  };
}

/** Convenience: the max-allowance approve amount (one-time approve UX). */
export { MAX_UINT256 };

/**
 * Build the terminal burn for a bridge-back (06 §A1.2 step 3). The caller
 * certifies the returned transfer through its normal SDK path, then calls
 * {finalizeBridgeBack} with the certified burn's stateId/txHash. For a partial
 * return, split first and pass the child whose value == `reason.amount`.
 */
export async function buildBridgeBackBurn(args: {
  readonly token: Token;
  readonly bridgeConfig: BridgeConfig;
  readonly reason: BridgeBackReason;
  readonly stateMask: Uint8Array;
}): Promise<{ transfer: Awaited<ReturnType<typeof createBridgeBackBurnTransfer>>['transfer']; reason: BridgeBackBurnReason }> {
  return createBridgeBackBurnTransfer(args.token, args.bridgeConfig, args.reason, args.stateMask);
}

/** The read-only return preview + the prover hand-off envelope, post-certification. */
export interface BridgeBackArtifacts {
  readonly preview: ReturnPreview;
  readonly witnessRequest: WitnessRequest;
}

/**
 * After the burn certifies, derive the pending nullifier + settlement leaf
 * ({previewReturn}) and assemble the prover hand-off envelope
 * ({buildWitnessRequest}). The wallet shows `preview` and POSTs `witnessRequest`
 * (plus the burned-token blob, recovery-critical) to the return service.
 * `reasonBytes` are the canonical bytes from {buildBridgeBackBurn}'s reason.
 */
export function finalizeBridgeBack(args: {
  readonly configHash: Uint8Array;
  readonly reason: BridgeBackReason;
  readonly reasonBytes: Uint8Array;
  readonly burnStateId: Uint8Array;
  readonly burnTxHash: Uint8Array;
  readonly burnedTokenCbor: Uint8Array;
}): BridgeBackArtifacts {
  const preview = previewReturn(args.configHash, args.reason, args.burnStateId, args.burnTxHash);
  const witnessRequest = buildWitnessRequest({
    tokenCbor: args.burnedTokenCbor,
    configHash: args.configHash,
    reasonBytes: args.reasonBytes,
  });
  return { preview, witnessRequest };
}

/** Hex helpers re-exported for read-only UI formatting. */
export { fromHex, toHex };
