import type { CertifiedMintTransaction } from '@unicitylabs/state-transition-sdk/lib/transaction/CertifiedMintTransaction.js';
import type { IMintJustificationVerifier } from '@unicitylabs/state-transition-sdk/lib/transaction/verification/IMintJustificationVerifier.js';
import type { MintJustificationVerifierService } from '@unicitylabs/state-transition-sdk/lib/transaction/verification/MintJustificationVerifierService.js';
import { VerificationResult } from '@unicitylabs/state-transition-sdk/lib/verification/VerificationResult.js';
import { VerificationStatus } from '@unicitylabs/state-transition-sdk/lib/verification/VerificationStatus.js';

import { bytesEqual, toHex } from './hex.js';
import { recipientCommitment } from './identifiers.js';
import { decodeLockEvent } from './lock-event.js';
import { TronUsdtLockJustification, TRON_USDT_LOCK_JUSTIFICATION_TAG } from './TronUsdtLockJustification.js';
import type { TronRpc } from './TronRpcClient.js';
import { decodeBridgePaymentData, type BridgedAmountExtractor } from './value.js';

const RULE = 'TronUsdtMintJustificationVerifier';

/** Trust anchors + identifiers a verifier instance is bound to. */
export interface ResolvedTronUsdtConfig {
  readonly chainId: number;
  /** 20-byte EVM-form address, lowercase hex. */
  readonly lockContractHex: string;
  /** 20-byte EVM-form address, lowercase hex. */
  readonly assetContractHex: string;
  readonly confirmations: number;
  /** 32-byte Unicity TokenType for this asset. */
  readonly tokenType: Uint8Array;
  /** 32-byte Sphere coinId for this asset. */
  readonly coinId: Uint8Array;
}

export interface TronUsdtVerifierDeps {
  readonly rpc: TronRpc;
  /** Defaults to the simple CLI value envelope decoder. */
  readonly extractAmount?: BridgedAmountExtractor;
}

function fail(message: string): VerificationResult<VerificationStatus> {
  return new VerificationResult(RULE, VerificationStatus.FAIL, message);
}

/**
 * Validates a bridged USDT-on-Tron token by re-checking its mint reason against
 * a Tron node: the lock exists, is final, has the right amount, and commits to
 * exactly this token's id + recipient. See docs/spec/MINT_REASON.md.
 */
export class TronUsdtMintJustificationVerifier implements IMintJustificationVerifier {
  private readonly rpc: TronRpc;
  private readonly extractAmount: BridgedAmountExtractor;

  public constructor(
    private readonly config: ResolvedTronUsdtConfig,
    deps: TronUsdtVerifierDeps,
  ) {
    this.rpc = deps.rpc;
    this.extractAmount = deps.extractAmount ?? decodeBridgePaymentData;
  }

  public get tag(): bigint {
    return TRON_USDT_LOCK_JUSTIFICATION_TAG;
  }

  public async verify(
    transaction: CertifiedMintTransaction,
    _service: MintJustificationVerifierService,
  ): Promise<VerificationResult<VerificationStatus>> {
    const bytes = transaction.justification;
    if (!bytes) {
      return fail('Transaction has no justification.');
    }

    let justification: TronUsdtLockJustification;
    try {
      justification = TronUsdtLockJustification.fromCBOR(bytes);
    } catch (e) {
      return fail(`Malformed justification: ${(e as Error).message}`);
    }
    const j = justification.data;

    // 1. Trust anchors: which Tron chain/contract/asset is authoritative.
    if (j.chainId !== this.config.chainId) {
      return fail(`Chain id mismatch: proof ${j.chainId}, expected ${this.config.chainId}.`);
    }
    if (toHex(j.lockContract).toLowerCase() !== this.config.lockContractHex) {
      return fail('Lock contract is not the canonical bridge contract.');
    }
    if (toHex(j.assetContract).toLowerCase() !== this.config.assetContractHex) {
      return fail('Asset contract is not the canonical bridged asset.');
    }

    // 2. Token type must be this asset's type.
    if (!bytesEqual(transaction.tokenType.bytes, this.config.tokenType)) {
      return fail('Token type does not match this bridged asset.');
    }

    // 3-4. Fetch the lock tx and require success + finality.
    const txInfo = await this.rpc.getTransactionInfo(toHex(j.txid));
    if (!txInfo) {
      return fail(`Lock transaction not found: ${toHex(j.txid)}.`);
    }
    if (!txInfo.success) {
      return fail('Lock transaction did not succeed.');
    }
    const tip = await this.rpc.getNowBlockNumber();
    const confirmations = tip - txInfo.blockNumber;
    if (confirmations < BigInt(this.config.confirmations)) {
      return fail(
        `Insufficient confirmations: ${confirmations} < ${this.config.confirmations} (awaiting source finality).`,
      );
    }

    // 5. Locate the Lock event and confirm it came from the canonical contract.
    const log = txInfo.logs[j.logIndex];
    if (!log) {
      return fail(`No log at index ${j.logIndex}.`);
    }
    if (log.address.toLowerCase() !== this.config.lockContractHex) {
      return fail('Log was not emitted by the canonical lock contract.');
    }
    const event = decodeLockEvent(log);
    if (!event) {
      return fail(`Log at index ${j.logIndex} is not a Lock event.`);
    }

    // 6. Amount: event == justification == token's declared value.
    if (event.amount !== j.amount) {
      return fail(`Amount mismatch: event ${event.amount}, justification ${j.amount}.`);
    }
    const declared = this.extractAmount(transaction.data, this.config.coinId);
    if (declared == null) {
      return fail('Token declares no bridged-asset value.');
    }
    if (declared !== event.amount) {
      return fail(`Token value ${declared} does not match locked amount ${event.amount}.`);
    }

    // 7. Binding: the lock commits to exactly this token id, recipient and nonce.
    if (event.nonce !== j.nonce) {
      return fail(`Nonce mismatch: event ${event.nonce}, justification ${j.nonce}.`);
    }
    if (!bytesEqual(event.unicityTokenId, transaction.tokenId.bytes)) {
      return fail('Lock is bound to a different token id (replay/forgery).');
    }
    const expectedRecipient = recipientCommitment(transaction.recipient.toCBOR());
    if (!bytesEqual(event.recipientCommitment, expectedRecipient)) {
      return fail('Lock is bound to a different recipient (theft/front-run).');
    }

    return new VerificationResult(RULE, VerificationStatus.OK);
  }
}
