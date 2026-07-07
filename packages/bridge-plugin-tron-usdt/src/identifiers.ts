import { sha256 } from '@noble/hashes/sha2.js';

import { toEvmAddressHex } from './tron-address.js';

const utf8 = (s: string): Uint8Array => new TextEncoder().encode(s);

/**
 * Deterministic 32-byte Unicity TokenType for a Tron-bridged asset.
 * `TRON_USDT_TYPE = SHA256("unicity-bridge:tron:<chainId>:<assetEvmHex>")`.
 */
export function deriveTokenType(chainId: number, assetContract: string): Uint8Array {
  return sha256(utf8(`unicity-bridge:tron:${chainId}:${toEvmAddressHex(assetContract)}`));
}

/** Deterministic 32-byte Sphere coinId for a Tron-bridged asset. */
export function deriveCoinId(chainId: number, assetContract: string): Uint8Array {
  return sha256(utf8(`unicity-bridge-coin:tron:${chainId}:${toEvmAddressHex(assetContract)}`));
}

/**
 * Commitment the Tron lock must carry so the bridged token can only be owned by
 * the intended recipient: `SHA256(recipient.toCBOR())`.
 */
export function recipientCommitment(recipientCbor: Uint8Array): Uint8Array {
  return sha256(recipientCbor);
}
