/**
 * Bridge-back burn construction (02-ts-sdk-and-wallet.md §2a–2c).
 *
 * The wallet burns the source token so the terminal transfer carries the
 * canonical `reasonBytes` in its **auxiliary data** and its recipient predicate
 * is `BurnPredicate(H(reasonBytes))` — *not* `BurnPredicate(reasonBytes)` (00 §4).
 * The reason is then fully self-contained in the burned blob: the circuit reads
 * `reasonBytes` from the certified aux data, recomputes `reasonHash`, requires
 * the terminal predicate to equal `BurnPredicate(reasonHash)`, and decodes the
 * fields. This module produces exactly those bytes plus the read-only return
 * preview and the prover hand-off envelope. It holds no authority and never
 * proves or settles.
 */
import { BurnPredicate } from '@unicitylabs/state-transition-sdk/lib/predicate/builtin/BurnPredicate.js';
import type { IPredicate } from '@unicitylabs/state-transition-sdk/lib/predicate/IPredicate.js';
import type { Token } from '@unicitylabs/state-transition-sdk/lib/transaction/Token.js';
import { TransferTransaction } from '@unicitylabs/state-transition-sdk/lib/transaction/TransferTransaction.js';

import {
  type BridgeBackReason,
  type BridgeConfig,
  burnTransitionId,
  encodeBridgeBackReason,
  nullifier as deriveNullifier,
  reasonHash as deriveReasonHash,
  type ReturnLeaf,
} from './derivations.js';

/** The reason bytes + the predicate that binds them, ready to attach to a burn. */
export interface BridgeBackBurnReason {
  /** Canonical `BridgeBackReason` CBOR; rides in the burn transfer's aux data. */
  readonly reasonBytes: Uint8Array;
  /** `H(reasonBytes)` — the payload of the binding {BurnPredicate}. */
  readonly reasonHash: Uint8Array;
  /** `BurnPredicate(H(reasonBytes))` — the terminal recipient predicate. */
  readonly burnPredicate: BurnPredicate;
}

/**
 * Build the canonical reason + its binding {BurnPredicate}. Pure (no SDK token
 * needed) so a wallet can preview the exact bytes before committing the burn.
 */
export function buildBridgeBackBurnReason(c: BridgeConfig, r: BridgeBackReason): BridgeBackBurnReason {
  const reasonBytes = encodeBridgeBackReason(c, r);
  const reasonHash = deriveReasonHash(reasonBytes);
  return {
    reasonBytes,
    reasonHash,
    burnPredicate: BurnPredicate.create(reasonHash),
  };
}

/**
 * Construct the terminal burn transfer for `token`: recipient is
 * `BurnPredicate(H(reasonBytes))` and the aux data is the canonical
 * `reasonBytes`. For a partial return, split first (existing split flow) and
 * pass the child whose value equals `amount`. The caller certifies/submits this
 * transfer through its normal SDK path; the resulting burned blob is the
 * release-authorizing recovery material (ZK_BACK3 §13).
 *
 * @param token     The (whole) token to burn.
 * @param c         Deployment config (binds the reason's config fields).
 * @param r         The return parameters (recipient/amount/fee/deadline).
 * @param stateMask State mask mixed into the new state (per the SDK transfer API).
 */
export async function createBridgeBackBurnTransfer(
  token: Token,
  c: BridgeConfig,
  r: BridgeBackReason,
  stateMask: Uint8Array,
): Promise<{ transfer: TransferTransaction; reason: BridgeBackBurnReason }> {
  const reason = buildBridgeBackBurnReason(c, r);
  const transfer = await TransferTransaction.create(
    token,
    reason.burnPredicate as unknown as IPredicate,
    stateMask,
    reason.reasonBytes,
  );
  return { transfer, reason };
}

/** The read-only return preview the wallet shows the user (02 §2b). */
export interface ReturnPreview {
  readonly burnTransitionId: Uint8Array;
  readonly nullifier: Uint8Array;
  readonly returnLeaf: ReturnLeaf;
}

/**
 * Derive the pending nullifier + settlement leaf for a burn, given the certified
 * burn state id / tx hash (available once the burn is certified). Read-only:
 * the prover computes the identical values, and a TS relayer reuses this (02 §2b).
 */
export function previewReturn(
  configHash: Uint8Array,
  r: BridgeBackReason,
  burnStateId: Uint8Array,
  burnTxHash: Uint8Array,
): ReturnPreview {
  const btId = burnTransitionId(burnStateId, burnTxHash);
  const nullifier = deriveNullifier(configHash, btId);
  return {
    burnTransitionId: btId,
    nullifier,
    returnLeaf: {
      nullifier,
      recipient: r.recipient,
      amount: r.amount,
      feeRecipient: r.feeRecipient,
      feeAmount: r.feeAmount,
      deadline: r.deadline,
    },
  };
}

/**
 * The witness-request envelope the wallet/relayer posts to a prover (02 §2c).
 * The wallet supplies only what it owns; the prover fetches anchor + inclusion
 * proofs itself. `reasonBytes` are included for convenience, but they are also
 * recoverable from `tokenCbor` (the burn's aux data) — the blob is self-contained.
 */
export interface WitnessRequest {
  readonly tokenCbor: Uint8Array;
  readonly configHash: Uint8Array;
  readonly reasonBytes: Uint8Array;
  readonly anchorHint?: bigint;
}

/** Assemble the prover hand-off envelope. */
export function buildWitnessRequest(args: {
  tokenCbor: Uint8Array;
  configHash: Uint8Array;
  reasonBytes: Uint8Array;
  anchorHint?: bigint;
}): WitnessRequest {
  const req: WitnessRequest = {
    tokenCbor: args.tokenCbor,
    configHash: args.configHash,
    reasonBytes: args.reasonBytes,
  };
  return args.anchorHint === undefined ? req : { ...req, anchorHint: args.anchorHint };
}
