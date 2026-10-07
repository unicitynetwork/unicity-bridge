import { toEvmAddressHex } from '../address.js';
import type { ConstantCaller } from '../source-chain.js';

export interface BalanceQuery {
  /** The asset contract, in any of its chain's address forms. */
  readonly assetAddress: string;
  /** The account, in any of its chain's address forms. */
  readonly owner: string;
}

/** What `owner` holds of the asset on the source chain, in its smallest unit. */
export async function queryBalance(rpc: ConstantCaller, q: BalanceQuery): Promise<bigint> {
  const ownerHex = toEvmAddressHex(q.owner);
  const word = await rpc.constantCall({
    ownerHex,
    contractHex: toEvmAddressHex(q.assetAddress),
    functionSignature: 'balanceOf(address)',
    parameterHex: ownerHex.padStart(64, '0'),
  });
  if (!/^[0-9a-fA-F]{64}$/.test(word)) {
    throw new Error(`balanceOf did not answer with one word (${word || 'empty'}); is the asset address right for this chain?`);
  }
  return BigInt(`0x${word}`);
}
