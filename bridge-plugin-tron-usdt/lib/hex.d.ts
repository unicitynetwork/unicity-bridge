/** Small hex helpers (lowercase, no `0x`). */
export declare function toHex(bytes: Uint8Array): string;
export declare function fromHex(hex: string): Uint8Array;
export declare function bytesEqual(a: Uint8Array, b: Uint8Array): boolean;
