import type { TronLog } from './TronRpcClient.js';
/** Solidity signature of the UnicityLock `Lock` event. */
export declare const LOCK_EVENT_SIGNATURE = "Lock(uint256,address,uint256,bytes32,bytes32)";
/** topic0 of the Lock event = keccak256(signature), lowercase hex (no 0x). */
export declare const LOCK_EVENT_TOPIC0: string;
/** Decoded fields of a `Lock(nonce, from, amount, unicityTokenId, recipientCommitment)` event. */
export interface DecodedLockEvent {
    readonly nonce: bigint;
    /** 20-byte EVM-form address that called lock(), lowercase hex. */
    readonly from: string;
    readonly amount: bigint;
    /** 32-byte Unicity tokenId the deposit is bound to. */
    readonly unicityTokenId: Uint8Array;
    /** 32-byte recipient commitment the deposit is bound to. */
    readonly recipientCommitment: Uint8Array;
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
export declare function decodeLockEvent(log: TronLog): DecodedLockEvent | null;
