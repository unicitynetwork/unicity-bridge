import type { CertifiedMintTransaction } from '@unicitylabs/state-transition-sdk/lib/transaction/CertifiedMintTransaction.js';
import type { Token } from '@unicitylabs/state-transition-sdk/lib/transaction/Token.js';
import type { IMintJustificationVerifier } from '@unicitylabs/state-transition-sdk/lib/transaction/verification/IMintJustificationVerifier.js';
import { VerificationResult } from '@unicitylabs/state-transition-sdk/lib/verification/VerificationResult.js';
import { VerificationStatus } from '@unicitylabs/state-transition-sdk/lib/verification/VerificationStatus.js';

import { BridgeLockJustification, BRIDGE_LOCK_JUSTIFICATION_TAG } from './BridgeLockJustification.js';
import { toHex } from './hex.js';
import type { LockMintJustificationVerifier } from './LockMintJustificationVerifier.js';

const RULE = 'BridgeMintJustificationVerifier';

export class BridgeMintJustificationVerifier implements IMintJustificationVerifier {
  public constructor(private readonly verifiers: readonly LockMintJustificationVerifier[]) {}

  public get tag(): bigint {
    return BRIDGE_LOCK_JUSTIFICATION_TAG;
  }

  public verify(
    transaction: CertifiedMintTransaction,
    nestedTokenCollector: (token: Token) => void,
  ): Promise<VerificationResult<VerificationStatus>> {
    const bytes = transaction.justification;
    if (!bytes) {
      return fail('Transaction has no justification.');
    }
    let j: BridgeLockJustification;
    try {
      j = BridgeLockJustification.fromCBOR(bytes);
    } catch (e) {
      return fail(`Malformed justification: ${(e as Error).message}`);
    }
    const owner = this.verifiers.find((v) => v.accepts(j.data));
    if (!owner) {
      return fail(`No bridge verifies chain ${j.data.chainId} vault 0x${toHex(j.data.lockContract)}.`);
    }
    return owner.verify(transaction, nestedTokenCollector);
  }
}

function fail(message: string): Promise<VerificationResult<VerificationStatus>> {
  return Promise.resolve(new VerificationResult(RULE, VerificationStatus.FAIL, message));
}
