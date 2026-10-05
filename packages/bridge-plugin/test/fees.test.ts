import assert from 'node:assert/strict';
import { test } from 'node:test';

import { toHex } from '../src/index.js';
import { feeTerms, parseFeeQuote, ReturnServiceClient, type FeeLimits, type FeeQuote } from '../src/wallet/index.js';

const COLLECTOR = '0x2b00d708fc777f174a248b9be01c8e8379d69caf';
const NOW_MS = 1_800_000_000_000;
const QUOTE: FeeQuote = { feeRecipient: COLLECTOR, feeAmount: '50000', deadline: 1_800_691_200 };
const FREE: FeeQuote = { feeRecipient: `0x${'00'.repeat(20)}`, feeAmount: '0', deadline: 1_800_691_200 };
const LIMITS: FeeLimits = { amount: 1_000_000n, maxFee: 50_000n, nowMs: NOW_MS };

const quoteWith = (change: Record<string, unknown>) => ({ ...QUOTE, ...change }) as FeeQuote;

test('getFees reads the quote the service publishes', async () => {
  const requested: string[] = [];
  const fetchQuote = (async (url: string) => (requested.push(url), new Response(JSON.stringify(QUOTE)))) as typeof fetch;
  const client = new ReturnServiceClient('http://service:8787/', { fetch: fetchQuote });
  assert.deepEqual(await client.getFees(), QUOTE);
  assert.deepEqual(requested, ['http://service:8787/fees']);
});

test('feeTerms turns a quote into the fee fields of a burn reason', () => {
  const terms = feeTerms(QUOTE, LIMITS);
  assert.equal(`0x${toHex(terms.feeRecipient)}`, COLLECTOR);
  assert.equal(terms.feeAmount, 50_000n);
  assert.equal(terms.deadline, 1_800_691_200n);
});

test('feeTerms of a free quote pays nobody, even with no fee allowed', () => {
  const terms = feeTerms(FREE, { ...LIMITS, maxFee: 0n });
  assert.deepEqual(terms.feeRecipient, new Uint8Array(20));
  assert.equal(terms.feeAmount, 0n);
});

test('feeTerms refuses a fee above what the burn allows', () => {
  assert.throws(() => feeTerms(QUOTE, { ...LIMITS, maxFee: 49_999n }), /above/);
});

test('feeTerms refuses a fee that leaves the owner nothing', () => {
  assert.throws(() => feeTerms(QUOTE, { ...LIMITS, amount: 50_000n }), /leaves nothing/);
  assert.throws(() => feeTerms(QUOTE, { ...LIMITS, amount: 49_999n }), /leaves nothing/);
  assert.equal(feeTerms(QUOTE, { ...LIMITS, amount: 50_001n }).feeAmount, 50_000n);
});

test('feeTerms refuses a recipient that is not a 20-byte address', () => {
  for (const feeRecipient of ['0x2b00', `0x${'ab'.repeat(21)}`, 'not hex', '', undefined, 7]) {
    assert.throws(() => feeTerms(quoteWith({ feeRecipient }), LIMITS), /recipient/, String(feeRecipient));
  }
});

test('feeTerms refuses a fee paid to the zero address', () => {
  assert.throws(() => feeTerms(quoteWith({ feeRecipient: FREE.feeRecipient }), LIMITS), /recipient/);
});

test('feeTerms refuses an amount that is not a whole decimal number', () => {
  for (const feeAmount of ['-1', '0x10', '1.5', '1e3', '', ' 5', '05', undefined, 50_000]) {
    assert.throws(() => feeTerms(quoteWith({ feeAmount }), LIMITS), /whole number/, String(feeAmount));
  }
});

test('feeTerms refuses a deadline that is not a future second', () => {
  for (const deadline of [NOW_MS / 1000, 0, -1, 1.5, Number.MAX_SAFE_INTEGER + 1, '1800691200', undefined]) {
    assert.throws(() => feeTerms(quoteWith({ deadline }), LIMITS), /deadline/, String(deadline));
  }
});

test('parseFeeQuote reads a quote whatever is being returned and refuses anything that is not one', () => {
  assert.equal(parseFeeQuote(QUOTE, NOW_MS).feeAmount, 50_000n);
  assert.equal(parseFeeQuote(FREE, NOW_MS).feeAmount, 0n);
  assert.throws(() => parseFeeQuote(quoteWith({ feeAmount: 'free' }), NOW_MS), /whole number/);
  assert.throws(() => parseFeeQuote(quoteWith({ feeRecipient: '0x2b00' }), NOW_MS), /recipient/);
  assert.throws(() => parseFeeQuote(quoteWith({ deadline: 1 }), NOW_MS), /deadline/);
});
