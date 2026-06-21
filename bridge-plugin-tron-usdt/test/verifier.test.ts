import assert from 'node:assert/strict';
import { test } from 'node:test';

import { VerificationStatus } from '@unicitylabs/state-transition-sdk/lib/verification/VerificationStatus.js';
import { MintJustificationVerifierService } from '@unicitylabs/state-transition-sdk/lib/transaction/verification/MintJustificationVerifierService.js';

import { AMOUNT, buildScenario, CONFIRMATIONS, hexToBytes, makeLockLog, NONCE } from './helpers.js';

async function verify(scenario: Awaited<ReturnType<typeof buildScenario>>) {
  return scenario.plugin.verifier.verify(
    scenario.certifiedTx,
    undefined as unknown as MintJustificationVerifierService,
  );
}

test('valid bridged mint verifies OK', async () => {
  const s = await buildScenario();
  const result = await verify(s);
  assert.equal(result.status, VerificationStatus.OK, result.message);
});

test('it registers cleanly into the SDK MintJustificationVerifierService by tag', async () => {
  const s = await buildScenario();
  const service = new MintJustificationVerifierService();
  service.register(s.plugin.verifier);
  // duplicate registration for the same tag must throw (1:1 tag→verifier).
  assert.throws(() => service.register(s.plugin.verifier), /Duplicate/);
});

test('FAIL: tampered amount in justification (does not match event)', async () => {
  const s = await buildScenario({ justification: (d) => ({ ...d, amount: AMOUNT + 1n }) });
  const result = await verify(s);
  assert.equal(result.status, VerificationStatus.FAIL);
  assert.match(result.message, /Amount mismatch/);
});

test('FAIL: token declares more value than was locked', async () => {
  const s = await buildScenario({ tokenValueAmount: AMOUNT * 1000n });
  const result = await verify(s);
  assert.equal(result.status, VerificationStatus.FAIL);
  assert.match(result.message, /does not match locked amount/);
});

test('FAIL: replay — lock bound to a different token id', async () => {
  // Event commits to a different tokenId than the minted token.
  const s = await buildScenario({
    logs: (def) => [makeLockLogWith(def, { unicityTokenId: hexToBytes('99'.repeat(32)) })],
  });
  const result = await verify(s);
  assert.equal(result.status, VerificationStatus.FAIL);
  assert.match(result.message, /different token id/);
});

test('FAIL: theft — lock bound to a different recipient', async () => {
  const s = await buildScenario({
    logs: (def) => [makeLockLogWith(def, { recipientCommitment: hexToBytes('88'.repeat(32)) })],
  });
  const result = await verify(s);
  assert.equal(result.status, VerificationStatus.FAIL);
  assert.match(result.message, /different recipient/);
});

test('FAIL: wrong/forged lock contract emitted the event', async () => {
  const s = await buildScenario({
    logs: (def) => [{ ...def, address: 'de'.repeat(20) }],
  });
  const result = await verify(s);
  assert.equal(result.status, VerificationStatus.FAIL);
  assert.match(result.message, /canonical lock contract/);
});

test('FAIL: insufficient confirmations (not yet final)', async () => {
  const s = await buildScenario({ blockNumber: 100n, tip: 100n + BigInt(CONFIRMATIONS) - 1n });
  const result = await verify(s);
  assert.equal(result.status, VerificationStatus.FAIL);
  assert.match(result.message, /Insufficient confirmations/);
});

test('FAIL: lock transaction not found on the node', async () => {
  const s = await buildScenario();
  s.rpc.txInfo = null;
  const result = await verify(s);
  assert.equal(result.status, VerificationStatus.FAIL);
  assert.match(result.message, /not found/);
});

test('FAIL: lock transaction reverted on chain', async () => {
  const s = await buildScenario({ success: false });
  const result = await verify(s);
  assert.equal(result.status, VerificationStatus.FAIL);
  assert.match(result.message, /did not succeed/);
});

test('FAIL: nonce mismatch between event and justification', async () => {
  const s = await buildScenario({ justification: (d) => ({ ...d, nonce: NONCE + 1n }) });
  const result = await verify(s);
  assert.equal(result.status, VerificationStatus.FAIL);
  assert.match(result.message, /Nonce mismatch/);
});

test('FAIL: wrong chain id in justification', async () => {
  const s = await buildScenario({ justification: (d) => ({ ...d, chainId: 1 }) });
  const result = await verify(s);
  assert.equal(result.status, VerificationStatus.FAIL);
  assert.match(result.message, /Chain id mismatch/);
});

// helper that overrides specific Lock-event fields while keeping the rest
function makeLockLogWith(
  def: ReturnType<typeof makeLockLog>,
  fields: { unicityTokenId?: Uint8Array; recipientCommitment?: Uint8Array },
): ReturnType<typeof makeLockLog> {
  // Rebuild data: amount(32) ‖ tokenId(32) ‖ recipientCommitment(32)
  const amountWord = def.data.slice(0, 64);
  const tokenIdWord = fields.unicityTokenId
    ? toWord(fields.unicityTokenId)
    : def.data.slice(64, 128);
  const recipWord = fields.recipientCommitment
    ? toWord(fields.recipientCommitment)
    : def.data.slice(128, 192);
  return { ...def, data: amountWord + tokenIdWord + recipWord };
}

function toWord(b: Uint8Array): string {
  let h = '';
  for (const x of b) h += x.toString(16).padStart(2, '0');
  return h.padStart(64, '0');
}
