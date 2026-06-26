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
function head(major, arg) {
    const m = major << 5;
    const n = BigInt(arg);
    if (n < 24n)
        return Uint8Array.of(m | Number(n));
    if (n <= 0xffn)
        return Uint8Array.of(m | 24, Number(n));
    if (n <= 0xffffn) {
        return Uint8Array.of(m | 25, Number((n >> 8n) & 0xffn), Number(n & 0xffn));
    }
    if (n <= 0xffffffffn) {
        return Uint8Array.of(m | 26, Number((n >> 24n) & 0xffn), Number((n >> 16n) & 0xffn), Number((n >> 8n) & 0xffn), Number(n & 0xffn));
    }
    const out = new Uint8Array(9);
    out[0] = m | 27;
    for (let i = 0; i < 8; i++)
        out[8 - i] = Number((n >> BigInt(8 * i)) & 0xffn);
    return out;
}
function concat(parts) {
    let len = 0;
    for (const p of parts)
        len += p.length;
    const out = new Uint8Array(len);
    let off = 0;
    for (const p of parts) {
        out.set(p, off);
        off += p.length;
    }
    return out;
}
/** major type 0: unsigned integer. */
export function uint(n) {
    return head(0, n);
}
/** major type 2: byte string. */
export function bytes(b) {
    return concat([head(2, b.length), b]);
}
/** major type 3: text string. */
export function text(s) {
    const enc = new TextEncoder().encode(s);
    return concat([head(3, enc.length), enc]);
}
/** major type 4: array header (followed by `len` items). */
export function arrayHeader(len) {
    return head(4, len);
}
/** major type 6: semantic tag. */
export function tag(t) {
    return head(6, t);
}
/** Concatenate pre-encoded CBOR items. */
export { concat as concatBytes };
/** `H(fields...) = SHA-256( CBOR-array(fields) )` (00 §8 / appendix convention). */
export function hArray(items) {
    return sha256(concat([arrayHeader(items.length), ...items]));
}
/** Strip leading zero bytes (minimal big-endian); zero becomes the empty string. */
export function minimalBe(b) {
    let first = 0;
    while (first < b.length && b[first] === 0)
        first++;
    return b.slice(first);
}
// --- canonical reader (decode side) ----------------------------------------
// Only the item types the bridge structures use, with strict canonical checks:
// minimal-length argument, no trailing bytes (the caller asserts full
// consumption). Mirrors the encoder above so a blob round-trips exactly.
/** A cursor over CBOR bytes. */
export class CborReader {
    buf;
    pos = 0;
    constructor(buf) {
        this.buf = buf;
    }
    get done() {
        return this.pos >= this.buf.length;
    }
    /** Read one item head; returns its major type and argument. Rejects
     *  non-minimal (non-canonical) argument encodings. */
    head() {
        if (this.done)
            throw new Error('CBOR: unexpected end');
        const ib = this.buf[this.pos++];
        const major = ib >> 5;
        const ai = ib & 0x1f;
        if (ai < 24)
            return { major, arg: BigInt(ai) };
        let len;
        if (ai === 24)
            len = 1;
        else if (ai === 25)
            len = 2;
        else if (ai === 26)
            len = 4;
        else if (ai === 27)
            len = 8;
        else
            throw new Error(`CBOR: bad additional info ${ai}`);
        if (this.pos + len > this.buf.length)
            throw new Error('CBOR: truncated argument');
        let arg = 0n;
        for (let i = 0; i < len; i++)
            arg = (arg << 8n) | BigInt(this.buf[this.pos++]);
        // canonical: the value must not fit in a shorter encoding
        const min = ai === 24 ? 24n : ai === 25 ? 0x100n : ai === 26 ? 0x10000n : 0x100000000n;
        if (arg < min)
            throw new Error('CBOR: non-canonical (non-minimal) integer');
        return { major, arg };
    }
    expect(major, what) {
        const h = this.head();
        if (h.major !== major)
            throw new Error(`CBOR: expected ${what} (major ${major}), got major ${h.major}`);
        return h.arg;
    }
    readTag() {
        return this.expect(6, 'tag');
    }
    readArrayHeader() {
        return Number(this.expect(4, 'array'));
    }
    readUint() {
        return this.expect(0, 'uint');
    }
    readBytes() {
        const len = Number(this.expect(2, 'byte string'));
        if (this.pos + len > this.buf.length)
            throw new Error('CBOR: truncated byte string');
        const out = this.buf.slice(this.pos, this.pos + len);
        this.pos += len;
        return out;
    }
}
