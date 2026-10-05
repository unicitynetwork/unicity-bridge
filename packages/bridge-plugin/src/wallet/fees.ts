import type { BridgeBackReason } from '../bridge-back/derivations.js';
import { fromHex } from '../hex.js';

export interface FeeQuote {
  readonly feeRecipient: string;
  readonly feeAmount: string;
  readonly deadline: number;
}

export interface FeeLimits {
  readonly amount: bigint;
  readonly maxFee: bigint;
  readonly nowMs: number;
}

export type FeeTerms = Pick<BridgeBackReason, 'feeRecipient' | 'feeAmount' | 'deadline'>;

const WHOLE_NUMBER = /^(0|[1-9][0-9]*)$/;
const ADDRESS = /^0x[0-9a-fA-F]{40}$/;

export function parseFeeQuote(quote: FeeQuote, nowMs: number): FeeTerms {
  const feeAmount = quotedAmount(quote);
  return { feeRecipient: quotedRecipient(quote, feeAmount), feeAmount, deadline: quotedDeadline(quote, nowMs) };
}

export function feeTerms(quote: FeeQuote, limits: FeeLimits): FeeTerms {
  const terms = parseFeeQuote(quote, limits.nowMs);
  if (terms.feeAmount > limits.maxFee) {
    throw new Error(`The return service asks a fee of ${terms.feeAmount}, above the ${limits.maxFee} allowed for this burn.`);
  }
  if (terms.feeAmount >= limits.amount) {
    throw new Error(`A fee of ${terms.feeAmount} leaves nothing of the ${limits.amount} being returned.`);
  }
  return terms;
}

function quotedAmount(quote: FeeQuote): bigint {
  if (typeof quote.feeAmount !== 'string' || !WHOLE_NUMBER.test(quote.feeAmount)) {
    throw new Error('The return service quoted a fee that is not a whole number.');
  }
  return BigInt(quote.feeAmount);
}

function quotedRecipient(quote: FeeQuote, feeAmount: bigint): Uint8Array {
  if (typeof quote.feeRecipient !== 'string' || !ADDRESS.test(quote.feeRecipient)) {
    throw new Error('The return service quoted a fee recipient that is not a 20-byte address.');
  }
  const recipient = fromHex(quote.feeRecipient);
  if (feeAmount > 0n && recipient.every((byte) => byte === 0)) {
    throw new Error('The return service quoted a fee with no recipient.');
  }
  return recipient;
}

function quotedDeadline(quote: FeeQuote, nowMs: number): bigint {
  if (!Number.isSafeInteger(quote.deadline) || quote.deadline * 1000 <= nowMs) {
    throw new Error('The return service quoted a fee deadline that is not in the future.');
  }
  return BigInt(quote.deadline);
}
