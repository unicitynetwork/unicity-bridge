import type { WalletIssuancePolicy } from '@unicitylabs/bridge-core';
import { SplitMintJustification } from '@unicitylabs/state-transition-sdk/lib/payment/SplitMintJustification.js';
import { CborDeserializer } from '@unicitylabs/state-transition-sdk/lib/serialization/cbor/CborDeserializer.js';
import type { CertifiedMintTransaction } from '@unicitylabs/state-transition-sdk/lib/transaction/CertifiedMintTransaction.js';
import type { TokenType } from '@unicitylabs/state-transition-sdk/lib/transaction/TokenType.js';
import { VerificationResult } from '@unicitylabs/state-transition-sdk/lib/verification/VerificationResult.js';
import { VerificationStatus } from '@unicitylabs/state-transition-sdk/lib/verification/VerificationStatus.js';

import { BRIDGE_LOCK_JUSTIFICATION_TAG } from './BridgeLockJustification.js';

const RULE = 'BridgedTokenIssuancePolicy';
const BACKING_TAGS: readonly bigint[] = [BRIDGE_LOCK_JUSTIFICATION_TAG, SplitMintJustification.CBOR_TAG];

export class BridgedTokenIssuancePolicy implements WalletIssuancePolicy {
  public constructor(
    public readonly tokenType: TokenType,
    public readonly coinIds: readonly string[],
    public readonly revision?: string,
  ) {}

  public verify(transaction: CertifiedMintTransaction): Promise<VerificationResult<VerificationStatus>> {
    const tag = reasonTag(transaction.justification);
    if (tag !== null && BACKING_TAGS.includes(tag)) return Promise.resolve(new VerificationResult(RULE, VerificationStatus.OK));
    return Promise.resolve(
      new VerificationResult(RULE, VerificationStatus.FAIL, 'A bridged token must be minted against a lock or split off a bridged token.'),
    );
  }
}

function reasonTag(justification: Uint8Array | null): bigint | null {
  if (!justification) return null;
  try {
    return BigInt(CborDeserializer.decodeTag(justification).tag);
  } catch {
    return null;
  }
}
