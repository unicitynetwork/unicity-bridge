import { encodeCallData, type ContractCall } from '../contract-call.js';
import type { Eip1193Provider } from './signer.js';

/** How long a wallet has to connect or to answer a transaction request, unless the signer is given another limit. */
export const DEFAULT_WALLET_REQUEST_TIMEOUT_MS = 5 * 60_000;

/**
 * A wallet that did not answer in time. The request may still be pending in the wallet, and a
 * transaction approved after this is still sent. The error has no `code`, so it never reads as a
 * refusal (4001): a caller that drops what it saved for a refused transaction keeps it here.
 */
export class WalletRequestTimeoutError extends Error {
  public constructor(
    public readonly method: string,
    public readonly timeoutMs: number,
  ) {
    const what = method === 'eth_sendTransaction' ? 'answer the transaction request' : 'connect';
    super(`The wallet did not ${what} within ${describeDuration(timeoutMs)}.`);
    this.name = 'WalletRequestTimeoutError';
  }
}

export function withTimeout<T>(request: Promise<T>, method: string, timeoutMs: number): Promise<T> {
  let timer: ReturnType<typeof setTimeout> | undefined;
  const timeout = new Promise<never>((_, reject) => {
    timer = setTimeout(() => reject(new WalletRequestTimeoutError(method, timeoutMs)), timeoutMs);
  });
  return Promise.race([request, timeout]).finally(() => clearTimeout(timer));
}

/** Sends `call` as an `eth_sendTransaction` from `from` and returns the wallet's transaction hash. */
export async function sendContractCall(provider: Eip1193Provider, from: string, call: ContractCall, timeoutMs: number): Promise<string> {
  const request = provider.request({
    method: 'eth_sendTransaction',
    params: [{ from, to: `0x${call.contractHex}`, data: `0x${encodeCallData(call)}` }],
  });
  const hash = (await withTimeout(request, 'eth_sendTransaction', timeoutMs)) as string | undefined;
  if (!hash) {
    throw new Error(`The wallet returned no transaction hash for ${call.functionSignature}.`);
  }
  return hash;
}

/**
 * The chain id from an `eth_chainId` answer. EIP-1193 makes it a hex string, but WalletConnect's
 * provider answers with a number, and reading that number as hex turns Sepolia's 11155111 into
 * 286609681.
 */
export function chainIdFrom(answer: unknown): number {
  return typeof answer === 'number' ? answer : Number.parseInt(String(answer), 16);
}

function describeDuration(ms: number): string {
  const [n, unit] = ms % 60_000 === 0 ? [ms / 60_000, 'minute'] : [Math.ceil(ms / 1000), 'second'];
  return `${n} ${unit}${n === 1 ? '' : 's'}`;
}
