/**
 * Allowance query (08 §1.1) — the read that lets bridge-in skip a redundant
 * `approve`. Pure ABI encoding over a {TronConstantCaller}; no TronWeb, no wallet
 * involvement (it's a node read, not a signed tx). Kept in the plugin so Sphere
 * never encodes a Tron call.
 */
import type { TronConstantCaller } from '../TronRpcClient.js';
import { toEvmAddressHex } from '../tron-address.js';

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
  /** TRC20 asset contract (any Tron address form). */
  readonly assetAddress: string;
  /** Token holder (the wallet) — any Tron address form. */
  readonly owner: string;
  /** Spender (the vault) — any Tron address form. */
  readonly spender: string;
}

/**
 * Read `allowance(owner, spender)` off the TRC20 asset. Returns the current
 * approved amount in the asset's smallest unit; `0n` when nothing is approved.
 */
export async function queryAllowance(rpc: TronConstantCaller, q: AllowanceQuery): Promise<bigint> {
  const ownerHex = toEvmAddressHex(q.owner);
  const spenderHex = toEvmAddressHex(q.spender);
  const assetHex = toEvmAddressHex(q.assetAddress);
  const word = await rpc.triggerConstantContract({
    ownerHex,
    contractHex: assetHex,
    functionSelector: 'allowance(address,address)',
    parameterHex: addressWord(ownerHex) + addressWord(spenderHex),
  });
  return wordToBigInt(word);
}
