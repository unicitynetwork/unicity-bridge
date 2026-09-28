/**
 * Allowance query (08 §1.1) — the read that lets bridge-in skip a redundant
 * `approve`. Pure ABI encoding over a {ConstantCaller}; no chain library, no
 * wallet involvement (it's a node read, not a signed tx). Kept in the plugin so
 * Sphere never encodes a contract call.
 */
import type { ConstantCaller } from '../source-chain.js';
import { toEvmAddressHex } from '../address.js';

/** Left-pad a 20-byte EVM-form hex address to a 32-byte ABI word. */
function addressWord(evmHex: string): string {
  const h = evmHex.replace(/^0x/i, '').toLowerCase();
  return h.padStart(64, '0');
}

/** Parse a 32-byte ABI uint256 word (hex, no `0x`) to a bigint. */
function wordToBigInt(word: string): bigint {
  const h = word.replace(/^0x/i, '');
  return h ? BigInt('0x' + h) : 0n;
}

export interface AllowanceQuery {
  /** Token contract, in any address form of its chain. */
  readonly assetAddress: string;
  /** Token holder (the wallet). */
  readonly owner: string;
  /** Spender (the vault). */
  readonly spender: string;
}

/**
 * Read `allowance(owner, spender)` off the token contract. Returns the current
 * approved amount in the asset's smallest unit; `0n` when nothing is approved.
 */
export async function queryAllowance(rpc: ConstantCaller, q: AllowanceQuery): Promise<bigint> {
  const ownerHex = toEvmAddressHex(q.owner);
  const spenderHex = toEvmAddressHex(q.spender);
  const assetHex = toEvmAddressHex(q.assetAddress);
  const word = await rpc.constantCall({
    ownerHex,
    contractHex: assetHex,
    functionSignature: 'allowance(address,address)',
    parameterHex: addressWord(ownerHex) + addressWord(spenderHex),
  });
  return wordToBigInt(word);
}
