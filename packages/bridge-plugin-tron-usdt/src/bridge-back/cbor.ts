/**
 * Minimal deterministic (canonical) CBOR encoder — only the item types the
 * bridge structures use: definite-length, minimal-length integer arguments.
 *
 * This mirrors `protocol/vectors/gen/src/cbor.rs` byte-for-byte; it is the TS half
 * of the cross-stack contract (00 §4, §5, §8). `H(...)` is SHA-256 over a CBOR
 * array of fields. Conformance is asserted against `protocol/vectors/{reason,
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

// --- canonical reader (decode side) ----------------------------------------
// Only the item types the bridge structures use, with strict canonical checks:
// minimal-length argument, no trailing bytes (the caller asserts full
// consumption). Mirrors the encoder above so a blob round-trips exactly.

/** A cursor over CBOR bytes. */
export class CborReader {
  public pos = 0;
  public constructor(public readonly buf: Uint8Array) {}

  public get done(): boolean {
    return this.pos >= this.buf.length;
  }

  /** Read one item head; returns its major type and argument. Rejects
   *  non-minimal (non-canonical) argument encodings. */
  private head(): { major: number; arg: bigint } {
    if (this.done) throw new Error('CBOR: unexpected end');
    const ib = this.buf[this.pos++];
    const major = ib >> 5;
    const ai = ib & 0x1f;
    if (ai < 24) return { major, arg: BigInt(ai) };
    let len: number;
    if (ai === 24) len = 1;
    else if (ai === 25) len = 2;
    else if (ai === 26) len = 4;
    else if (ai === 27) len = 8;
    else throw new Error(`CBOR: bad additional info ${ai}`);
    if (this.pos + len > this.buf.length) throw new Error('CBOR: truncated argument');
    let arg = 0n;
    for (let i = 0; i < len; i++) arg = (arg << 8n) | BigInt(this.buf[this.pos++]);
    // canonical: the value must not fit in a shorter encoding
    const min = ai === 24 ? 24n : ai === 25 ? 0x100n : ai === 26 ? 0x10000n : 0x100000000n;
    if (arg < min) throw new Error('CBOR: non-canonical (non-minimal) integer');
    return { major, arg };
  }

  private expect(major: number, what: string): bigint {
    const h = this.head();
    if (h.major !== major) throw new Error(`CBOR: expected ${what} (major ${major}), got major ${h.major}`);
    return h.arg;
  }

  public readTag(): bigint {
    return this.expect(6, 'tag');
  }

  public readArrayHeader(): number {
    return Number(this.expect(4, 'array'));
  }

  public readUint(): bigint {
    return this.expect(0, 'uint');
  }

  public readBytes(): Uint8Array {
    const len = Number(this.expect(2, 'byte string'));
    if (this.pos + len > this.buf.length) throw new Error('CBOR: truncated byte string');
    const out = this.buf.slice(this.pos, this.pos + len);
    this.pos += len;
    return out;
  }
}
