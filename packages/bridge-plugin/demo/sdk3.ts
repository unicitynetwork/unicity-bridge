import type { RootTrustBase } from '@unicitylabs/state-transition-sdk/lib/api/bft/RootTrustBase.js';
import { UnicitySealQuorumSignaturesVerificationRule } from '@unicitylabs/state-transition-sdk/lib/api/bft/verification/rule/UnicitySealQuorumSignaturesVerificationRule.js';
import { UnicityCertificateVerifier } from '@unicitylabs/state-transition-sdk/lib/api/bft/verification/UnicityCertificateVerifier.js';
import { VerifiedSealCache } from '@unicitylabs/state-transition-sdk/lib/api/bft/verification/VerifiedSealCache.js';
import { Secp256k1SignatureVerifier } from '@unicitylabs/state-transition-sdk/lib/crypto/secp256k1/Secp256k1SignatureVerifier.js';
import { PredicateVerifierService } from '@unicitylabs/state-transition-sdk/lib/predicate/verification/PredicateVerifierService.js';
import { MintJustificationVerifierService } from '@unicitylabs/state-transition-sdk/lib/transaction/verification/MintJustificationVerifierService.js';
import { TokenIssuanceVerifierService } from '@unicitylabs/state-transition-sdk/lib/transaction/verification/TokenIssuanceVerifierService.js';
import { VerificationContext } from '@unicitylabs/state-transition-sdk/lib/transaction/verification/VerificationContext.js';

export interface VerificationStack {
  readonly predicateVerifier: PredicateVerifierService;
  readonly unicityCertificateVerifier: UnicityCertificateVerifier;
  readonly mintJustificationVerifier: MintJustificationVerifierService;
  readonly context: VerificationContext;
}

export function verificationStack(
  trustBase: RootTrustBase,
  mintJustificationVerifier: MintJustificationVerifierService = new MintJustificationVerifierService(),
): VerificationStack {
  const predicateVerifier = PredicateVerifierService.create();
  const unicityCertificateVerifier = new UnicityCertificateVerifier(
    new UnicitySealQuorumSignaturesVerificationRule(new Secp256k1SignatureVerifier(), new VerifiedSealCache(256)),
  );
  const context = new VerificationContext(
    trustBase,
    predicateVerifier,
    unicityCertificateVerifier,
    mintJustificationVerifier,
    new TokenIssuanceVerifierService(false),
  );
  return { predicateVerifier, unicityCertificateVerifier, mintJustificationVerifier, context };
}
