/** Tron mainnet address prefix byte (0x41). */
export declare const TRON_ADDRESS_PREFIX = 65;
/**
 * Normalize any Tron address form to the 20-byte EVM-style address as lowercase
 * hex (no `0x`, no `41` prefix) — the form Tron event logs use for `address`.
 *
 * Accepts:
 *  - base58check `T...` addresses,
 *  - `41`-prefixed 21-byte hex (TronWeb `toHex()` form),
 *  - bare 20-byte hex (optionally `0x`-prefixed).
 */
export declare function toEvmAddressHex(address: string): string;
