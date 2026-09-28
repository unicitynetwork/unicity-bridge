/**
 * Minimal Solidity `abi.encode` for the static types the vault recomputes, plus
 * the single dynamic `string` domain literal used inside `configHash`/`lockDigest`.
 *
 * Mirrors `protocol/vectors/gen/src/abi.rs` byte-for-byte (interop §1, §7). `encode`
 * does proper head/tail framing; `packWords` is the non-framed 32-byte-word
 * concatenation the vault keccak-hashes for `returnRoot`/`lockRefRoot`.
 */
import { keccak_256 } from '@noble/hashes/sha3.js';

export type Val =
  | { t: 'str'; v: string }
  | { t: 'u32'; v: number | bigint }
  | { t: 'u64'; v: number | bigint }
  | { t: 'u256'; v: bigint }
  | { t: 'addr'; v: Uint8Array } // 20 bytes
  | { t: 'b32'; v: Uint8Array }; // 32 bytes

export const Str = (v: string): Val => ({ t: 'str', v });
export const U32 = (v: number | bigint): Val => ({ t: 'u32', v });
export const U64 = (v: number | bigint): Val => ({ t: 'u64', v });
export const U256 = (v: bigint): Val => ({ t: 'u256', v });
export const Addr = (v: Uint8Array): Val => ({ t: 'addr', v });
export const B32 = (v: Uint8Array): Val => ({ t: 'b32', v });

export function keccak256(input: Uint8Array): Uint8Array {
  return keccak_256(input);
}

/** 32-byte big-endian word for a small unsigned integer. */
function wordU(n: bigint): Uint8Array {
  const w = new Uint8Array(32);
  for (let i = 0; i < 32 && n > 0n; i++) {
    w[31 - i] = Number(n & 0xffn);
    n >>= 8n;
  }
  return w;
}

/** The 32-byte head word for a static value (numbers/addresses left-padded). */
function staticWord(v: Val): Uint8Array {
  switch (v.t) {
    case 'u32':
    case 'u64':
      return wordU(BigInt(v.v));
    case 'u256':
      return wordU(v.v);
    case 'addr': {
      if (v.v.length !== 20) throw new Error('addr must be 20 bytes');
      const w = new Uint8Array(32);
      w.set(v.v, 12);
      return w;
    }
    case 'b32': {
      if (v.v.length !== 32) throw new Error('b32 must be 32 bytes');
      return v.v.slice();
    }
    case 'str':
      throw new Error('dynamic value has no head word');
  }
}

function concat(parts: Uint8Array[]): Uint8Array {
  let len = 0;
  for (const p of parts) len += p.length;
  const out = new Uint8Array(len);
  let off = 0;
  for (const p of parts) {
    out.set(p, off);
    off += p.length;
  }
  return out;
}

/** Solidity `abi.encode(vals...)` with proper head/tail framing. */
export function encode(vals: Val[]): Uint8Array {
  const headSize = 32 * vals.length;
  const heads: Uint8Array[] = [];
  const tails: Uint8Array[] = [];
  let tailLen = 0;
  for (const v of vals) {
    if (v.t === 'str') {
      heads.push(wordU(BigInt(headSize + tailLen)));
      const data = new TextEncoder().encode(v.v);
      const padded = new Uint8Array(Math.ceil(data.length / 32) * 32);
      padded.set(data);
      const piece = concat([wordU(BigInt(data.length)), padded]);
      tails.push(piece);
      tailLen += piece.length;
    } else {
      heads.push(staticWord(v));
    }
  }
  return concat([...heads, ...tails]);
}

/** Concatenated fixed-width 32-byte words, no array framing (00 §7). */
export function packWords(vals: Val[]): Uint8Array {
  return concat(vals.map(staticWord));
}
