declare function concat(parts: Uint8Array[]): Uint8Array;
/** major type 0: unsigned integer. */
export declare function uint(n: number | bigint): Uint8Array;
/** major type 2: byte string. */
export declare function bytes(b: Uint8Array): Uint8Array;
/** major type 3: text string. */
export declare function text(s: string): Uint8Array;
/** major type 4: array header (followed by `len` items). */
export declare function arrayHeader(len: number): Uint8Array;
/** major type 6: semantic tag. */
export declare function tag(t: number | bigint): Uint8Array;
/** Concatenate pre-encoded CBOR items. */
export { concat as concatBytes };
/** `H(fields...) = SHA-256( CBOR-array(fields) )` (00 §8 / appendix convention). */
export declare function hArray(items: Uint8Array[]): Uint8Array;
/** Strip leading zero bytes (minimal big-endian); zero becomes the empty string. */
export declare function minimalBe(b: Uint8Array): Uint8Array;
