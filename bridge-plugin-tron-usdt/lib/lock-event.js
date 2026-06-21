import { keccak_256 } from '@noble/hashes/sha3.js';
import { fromHex, toHex } from './hex.js';
/** Solidity signature of the UnicityLock `Lock` event. */
export const LOCK_EVENT_SIGNATURE = 'Lock(uint256,address,uint256,bytes32,bytes32)';
const utf8 = (s) => new TextEncoder().encode(s);
/** topic0 of the Lock event = keccak256(signature), lowercase hex (no 0x). */
export const LOCK_EVENT_TOPIC0 = toHex(keccak_256(utf8(LOCK_EVENT_SIGNATURE)));
function strip0x(h) {
    return h.startsWith('0x') || h.startsWith('0X') ? h.slice(2) : h;
}
function wordToBigInt(word) {
    let v = 0n;
    for (const b of word) {
        v = (v << 8n) | BigInt(b);
    }
    return v;
}
/**
 * Decode a Lock event log. Returns null if the log is not a well-formed Lock
 * event (wrong topic0, wrong arity, or short data) — callers treat that as
 * "no matching lock event".
 *
 * Event layout (indexed nonce, indexed from; non-indexed in data):
 *   topics[0] = keccak256(signature)
 *   topics[1] = nonce        (uint256)
 *   topics[2] = from         (address, left-padded to 32 bytes)
 *   data      = amount ‖ unicityTokenId ‖ recipientCommitment  (3 × 32 bytes)
 */
export function decodeLockEvent(log) {
    const topics = log.topics.map(strip0x).map((t) => t.toLowerCase());
    if (topics.length !== 3 || topics[0] !== LOCK_EVENT_TOPIC0) {
        return null;
    }
    const data = fromHex(strip0x(log.data));
    if (data.length < 96) {
        return null;
    }
    const nonce = wordToBigInt(fromHex(topics[1]));
    // address occupies the low 20 bytes of the 32-byte topic word.
    const from = topics[2].slice(24);
    return {
        nonce,
        from,
        amount: wordToBigInt(data.subarray(0, 32)),
        unicityTokenId: data.subarray(32, 64),
        recipientCommitment: data.subarray(64, 96),
    };
}
