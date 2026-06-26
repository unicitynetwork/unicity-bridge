/**
 * Minimal deterministic (canonical) CBOR encoder — only the item types the
 * bridge structures use: definite-length, minimal-length integer arguments.
 *
 * This mirrors `bridge-vectors/gen/src/cbor.rs` byte-for-byte; it is the TS half
 * of the cross-stack contract (00 §4, §5, §8). `H(...)` is SHA-256 over a CBOR
 * array of fields. Conformance is asserted against `bridge-vectors/{reason,
 * nullifier}` — do not "improve" the encoding without bumping BRIDGE_PROTO_VERSION.
 */
import { sha256 } from '@noble/hashes/sha2.js';

/** major<<5 | argument, minimal-length per canonical CBOR. */
function head(major: number, arg: number | bigint): Uint8Array {
  const m = major << 5;
  const n = BigInt(arg);
  if (n < 24n) return Uint8Array.of(m | Number(n));
  if (n <= 0xffn) return Uint8Array.of(m | 24, Number(n));
  if (n <= 0xffffn) {
    return Uint8Array.of(m | 25, Number((n >> 8n) & 0xffn), Number(n & 0xffn));
  }
  if (n <= 0xffffffffn) {
    return Uint8Array.of(
      m | 26,
      Number((n >> 24n) & 0xffn),
      Number((n >> 16n) & 0xffn),
      Number((n >> 8n) & 0xffn),
      Number(n & 0xffn),
    );
  }
  const out = new Uint8Array(9);
  out[0] = m | 27;
  for (let i = 0; i < 8; i++) out[8 - i] = Number((n >> BigInt(8 * i)) & 0xffn);
  return out;
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

/** major type 0: unsigned integer. */
export function uint(n: number | bigint): Uint8Array {
  return head(0, n);
}

/** major type 2: byte string. */
export function bytes(b: Uint8Array): Uint8Array {
  return concat([head(2, b.length), b]);
}

/** major type 3: text string. */
export function text(s: string): Uint8Array {
  const enc = new TextEncoder().encode(s);
  return concat([head(3, enc.length), enc]);
}

/** major type 4: array header (followed by `len` items). */
export function arrayHeader(len: number): Uint8Array {
  return head(4, len);
}

/** major type 6: semantic tag. */
export function tag(t: number | bigint): Uint8Array {
  return head(6, t);
}

/** Concatenate pre-encoded CBOR items. */
export { concat as concatBytes };

/** `H(fields...) = SHA-256( CBOR-array(fields) )` (00 §8 / appendix convention). */
export function hArray(items: Uint8Array[]): Uint8Array {
  return sha256(concat([arrayHeader(items.length), ...items]));
}

/** Strip leading zero bytes (minimal big-endian); zero becomes the empty string. */
export function minimalBe(b: Uint8Array): Uint8Array {
  let first = 0;
  while (first < b.length && b[first] === 0) first++;
  return b.slice(first);
}
