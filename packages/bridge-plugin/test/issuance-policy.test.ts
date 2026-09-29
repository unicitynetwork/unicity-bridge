import assert from 'node:assert/strict';
import { test } from 'node:test';

import { SplitMintJustification } from '@unicitylabs/state-transition-sdk/lib/payment/SplitMintJustification.js';
import { CborSerializer } from '@unicitylabs/state-transition-sdk/lib/serialization/cbor/CborSerializer.js';
import type { CertifiedMintTransaction } from '@unicitylabs/state-transition-sdk/lib/transaction/CertifiedMintTransaction.js';
import { TokenType } from '@unicitylabs/state-transition-sdk/lib/transaction/TokenType.js';
import { VerificationStatus } from '@unicitylabs/state-transition-sdk/lib/verification/VerificationStatus.js';

import { BridgedTokenIssuancePolicy } from '../src/index.js';
import { buildScenario } from './helpers.js';

const TYPE = new TokenType(new Uint8Array(32).fill(0x6f));
const COIN = 'ab'.repeat(32);

function mintWith(justification: Uint8Array | null): CertifiedMintTransaction {
  return { justification } as unknown as CertifiedMintTransaction;
}

function tagged(tag: bigint): Uint8Array {
  return CborSerializer.encodeTag(tag, CborSerializer.encodeUnsignedInteger(1n));
}

test('a bridged token type accepts a genesis minted against a lock', async () => {
  const s = await buildScenario();
  const policy = new BridgedTokenIssuancePolicy(TYPE, [COIN]);

  assert.equal((await policy.verify(s.certifiedTx)).status, VerificationStatus.OK);
});

test('a bridged token type accepts a genesis split off another token, whose own genesis the SDK checks in turn', async () => {
  const policy = new BridgedTokenIssuancePolicy(TYPE, [COIN]);

  assert.equal((await policy.verify(mintWith(tagged(SplitMintJustification.CBOR_TAG)))).status, VerificationStatus.OK);
});

test('a bridged token type refuses a genesis with no reason, another reason, or unreadable bytes', async () => {
  const policy = new BridgedTokenIssuancePolicy(TYPE, [COIN]);

  for (const justification of [null, tagged(39048n), new Uint8Array([0xff])]) {
    const result = await policy.verify(mintWith(justification));
    assert.equal(result.status, VerificationStatus.FAIL, `accepted ${String(justification)}`);
  }
});

test('the policy names its token type and the coin only that type issues', () => {
  const policy = new BridgedTokenIssuancePolicy(TYPE, [COIN]);

  assert.equal(policy.tokenType, TYPE);
  assert.deepEqual(policy.coinIds, [COIN]);
});
