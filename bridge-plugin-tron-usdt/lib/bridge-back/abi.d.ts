export type Val = {
    t: 'str';
    v: string;
} | {
    t: 'u32';
    v: number | bigint;
} | {
    t: 'u64';
    v: number | bigint;
} | {
    t: 'u256';
    v: bigint;
} | {
    t: 'addr';
    v: Uint8Array;
} | {
    t: 'b32';
    v: Uint8Array;
};
export declare const Str: (v: string) => Val;
export declare const U32: (v: number | bigint) => Val;
export declare const U64: (v: number | bigint) => Val;
export declare const U256: (v: bigint) => Val;
export declare const Addr: (v: Uint8Array) => Val;
export declare const B32: (v: Uint8Array) => Val;
export declare function keccak256(input: Uint8Array): Uint8Array;
/** Solidity `abi.encode(vals...)` with proper head/tail framing. */
export declare function encode(vals: Val[]): Uint8Array;
/** Concatenated fixed-width 32-byte words, no array framing (00 §7). */
export declare function packWords(vals: Val[]): Uint8Array;
