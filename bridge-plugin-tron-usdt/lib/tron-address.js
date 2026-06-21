import { sha256 } from '@noble/hashes/sha2.js';
import { fromHex, toHex } from './hex.js';
const BASE58_ALPHABET = '123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz';
const BASE58_MAP = {};
for (let i = 0; i < BASE58_ALPHABET.length; i++) {
    BASE58_MAP[BASE58_ALPHABET[i]] = i;
}
/** Tron mainnet address prefix byte (0x41). */
export const TRON_ADDRESS_PREFIX = 0x41;
function base58Decode(input) {
    const bytes = [0];
    for (const ch of input) {
        const value = BASE58_MAP[ch];
        if (value === undefined) {
            throw new Error(`Invalid base58 character: ${ch}`);
        }
        let carry = value;
        for (let j = 0; j < bytes.length; j++) {
            carry += bytes[j] * 58;
            bytes[j] = carry & 0xff;
            carry >>= 8;
        }
        while (carry > 0) {
            bytes.push(carry & 0xff);
            carry >>= 8;
        }
    }
    // Leading '1's encode leading zero bytes.
    for (let k = 0; k < input.length && input[k] === '1'; k++) {
        bytes.push(0);
    }
    return Uint8Array.from(bytes.reverse());
}
/**
 * Normalize any Tron address form to the 20-byte EVM-style address as lowercase
 * hex (no `0x`, no `41` prefix) — the form Tron event logs use for `address`.
 *
 * Accepts:
 *  - base58check `T...` addresses,
 *  - `41`-prefixed 21-byte hex (TronWeb `toHex()` form),
 *  - bare 20-byte hex (optionally `0x`-prefixed).
 */
export function toEvmAddressHex(address) {
    if (address.startsWith('T')) {
        const decoded = base58Decode(address);
        if (decoded.length !== 25) {
            throw new Error(`Invalid Tron base58 address length: ${address}`);
        }
        const payload = decoded.subarray(0, 21);
        const checksum = decoded.subarray(21);
        const expected = sha256(sha256(payload)).subarray(0, 4);
        for (let i = 0; i < 4; i++) {
            if (checksum[i] !== expected[i]) {
                throw new Error(`Bad checksum for Tron address: ${address}`);
            }
        }
        if (payload[0] !== TRON_ADDRESS_PREFIX) {
            throw new Error(`Unexpected Tron address prefix: ${address}`);
        }
        return toHex(payload.subarray(1));
    }
    const bytes = fromHex(address);
    if (bytes.length === 21 && bytes[0] === TRON_ADDRESS_PREFIX) {
        return toHex(bytes.subarray(1));
    }
    if (bytes.length === 20) {
        return toHex(bytes);
    }
    throw new Error(`Unrecognized Tron address form: ${address}`);
}
