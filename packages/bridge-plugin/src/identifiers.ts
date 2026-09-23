import { sha256 } from '@noble/hashes/sha2.js';
import type { ChainFamily } from '@unicitylabs/bridge-core';

import { toEvmAddressHex } from './address.js';

const utf8 = (s: string): Uint8Array => new TextEncoder().encode(s);

/**
 * Deterministic 32-byte Unicity TokenType for a bridged asset, from its source
 * chain family (CAIP-2 namespace), chain id and asset contract:
 * `SHA256("unicity-bridge:<family>:<chainId>:<assetEvmHex>")` (interop §2).
 */
export function deriveTokenType(family: ChainFamily, chainId: number, assetContract: string): Uint8Array {
  return sha256(utf8(`unicity-bridge:${family}:${chainId}:${toEvmAddressHex(assetContract)}`));
}

/** Deterministic 32-byte Sphere coinId for a bridged asset, same inputs as {deriveTokenType}. */
export function deriveCoinId(family: ChainFamily, chainId: number, assetContract: string): Uint8Array {
  return sha256(utf8(`unicity-bridge-coin:${family}:${chainId}:${toEvmAddressHex(assetContract)}`));
}

/**
 * Commitment the Tron lock must carry so the bridged token can only be owned by
 * the intended recipient: `SHA256(recipient.toCBOR())`.
 */
export function recipientCommitment(recipientCbor: Uint8Array): Uint8Array {
  return sha256(recipientCbor);
}
