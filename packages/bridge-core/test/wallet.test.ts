import assert from 'node:assert/strict';
import { test } from 'node:test';

import {
  burnForReturn,
  mintBridgedToken,
  recoverPendingBurns,
  type BridgePayments,
  type MintRequest,
  type WalletPendingBurn,
} from '../src/index.js';

function payments(overrides: Partial<BridgePayments> = {}): BridgePayments & { log: string[] } {
  const log: string[] = [];
  return {
    log,
    mintCustom: async () => {
      log.push('mintCustom');
      return { success: true, tokenId: 'ab'.repeat(32) };
    },
    burn: async ({ tokenId }) => {
      log.push(`burn:${tokenId}`);
      return { success: true, burnId: 'burn-1', tokenId, burnedToken: new Uint8Array([9, 9, 9]) };
    },
    tokenJustification: async () => null,
    pendingBurns: async () => [],
    acknowledgeBurn: async (id) => {
      log.push(`ack:${id}`);
    },
    ...overrides,
  };
}

const verifier = { tag: 1330002n, verify: async () => ({}) as never };
const request: MintRequest = {
  coinIdHex: 'cc'.repeat(32),
  amount: 5n,
  mintData: new Uint8Array([1]),
  tokenType: new Uint8Array(32).fill(2),
  salt: new Uint8Array(32).fill(3),
  genesisReason: new Uint8Array([4]),
  mintJustificationVerifiers: [verifier],
};

test('mintBridgedToken hands the request to the wallet custom mint field for field', async () => {
  let seen: unknown;
  const p = payments({
    mintCustom: async (r) => {
      seen = r;
      return { success: true, tokenId: 'ab'.repeat(32) };
    },
  });
  const result = await mintBridgedToken(p, request);
  assert.deepEqual(result, { success: true, tokenId: 'ab'.repeat(32) });
  assert.deepEqual(seen, {
    tokenType: request.tokenType,
    salt: request.salt,
    data: request.mintData,
    justification: request.genesisReason,
    assets: [{ coinId: request.coinIdHex, amount: 5n }],
    mintJustificationVerifiers: [verifier],
  });
});

test('burnForReturn: burn, persist, THEN acknowledge — in that order', async () => {
  const p = payments();
  const result = await burnForReturn(p, {
    tokenId: 'aa'.repeat(32),
    reasonBytes: new Uint8Array([7]),
    persist: async (blob, burnId) => {
      p.log.push(`persist:${burnId}:${blob.join(',')}`);
    },
  });
  assert.deepEqual(result, { burnId: 'burn-1', burnedToken: new Uint8Array([9, 9, 9]) });
  assert.deepEqual(p.log, [`burn:${'aa'.repeat(32)}`, 'persist:burn-1:9,9,9', 'ack:burn-1']);
});

test('burnForReturn: a failing persist leaves the burn UNacknowledged (the wallet keeps the blob) and rethrows', async () => {
  const p = payments();
  await assert.rejects(
    burnForReturn(p, {
      tokenId: 'aa'.repeat(32),
      reasonBytes: new Uint8Array([7]),
      persist: async () => {
        throw new Error('disk full');
      },
    }),
    /disk full/,
  );
  assert.deepEqual(p.log, [`burn:${'aa'.repeat(32)}`]);
});

test('burnForReturn: a failed burn throws with the wallet error and never acknowledges', async () => {
  const p = payments({ burn: async ({ tokenId }) => ({ success: false, burnId: 'burn-2', tokenId, error: 'not owned' }) });
  await assert.rejects(burnForReturn(p, { tokenId: 'aa'.repeat(32), reasonBytes: new Uint8Array([7]), persist: async () => {} }), /not owned/);
  assert.deepEqual(p.log, []);
});

test('recoverPendingBurns: settled blobs are persisted and released; in-flight ones are left to the wallet', async () => {
  const pending: WalletPendingBurn[] = [
    { burnId: 'b-settled', tokenId: '11'.repeat(32), reasonBytes: new Uint8Array([1]), burnedToken: new Uint8Array([5]), settled: true },
    { burnId: 'b-flight', tokenId: '22'.repeat(32), reasonBytes: new Uint8Array([2]), burnedToken: null, settled: false },
    { burnId: 'b-certified', tokenId: '33'.repeat(32), reasonBytes: new Uint8Array([3]), burnedToken: new Uint8Array([6]), settled: false },
  ];
  const p = payments({ pendingBurns: async () => pending });
  const recovered = await recoverPendingBurns(p, async (blob, burnId) => {
    p.log.push(`persist:${burnId}:${blob.join(',')}`);
  });
  assert.deepEqual(recovered, [{ burnId: 'b-settled', burnedToken: new Uint8Array([5]) }]);
  assert.deepEqual(p.log, ['persist:b-settled:5', 'ack:b-settled']);
});
