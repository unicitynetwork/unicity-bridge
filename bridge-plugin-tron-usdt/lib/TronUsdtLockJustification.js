import { CborDeserializer } from '@unicitylabs/state-transition-sdk/lib/serialization/cbor/CborDeserializer.js';
import { CborSerializer } from '@unicitylabs/state-transition-sdk/lib/serialization/cbor/CborSerializer.js';
/**
 * CBOR tag for the Tron-USDT lock justification. Bridge tags are allocated
 * one-per-asset so the SDK's tag→verifier registry stays 1:1.
 */
export const TRON_USDT_LOCK_JUSTIFICATION_TAG = 1330002n;
const VERSION = 1n;
/** Encodes/decodes the self-contained lock proof carried in a token's mint reason. */
export class TronUsdtLockJustification {
    data;
    static CBOR_TAG = TRON_USDT_LOCK_JUSTIFICATION_TAG;
    constructor(data) {
        this.data = data;
    }
    toCBOR() {
        const d = this.data;
        return CborSerializer.encodeTag(TronUsdtLockJustification.CBOR_TAG, CborSerializer.encodeArray(CborSerializer.encodeUnsignedInteger(VERSION), CborSerializer.encodeUnsignedInteger(d.chainId), CborSerializer.encodeByteString(d.lockContract), CborSerializer.encodeByteString(d.assetContract), CborSerializer.encodeByteString(d.txid), CborSerializer.encodeUnsignedInteger(d.logIndex), CborSerializer.encodeUnsignedInteger(d.amount), CborSerializer.encodeUnsignedInteger(d.nonce)));
    }
    static fromCBOR(bytes) {
        const tag = CborDeserializer.decodeTag(bytes);
        if (tag.tag !== TronUsdtLockJustification.CBOR_TAG) {
            throw new Error(`Invalid CBOR tag for TronUsdtLockJustification: ${tag.tag}`);
        }
        const items = CborDeserializer.decodeArray(tag.data, 8);
        const version = CborDeserializer.decodeUnsignedInteger(items[0]);
        if (version !== VERSION) {
            throw new Error(`Unsupported TronUsdtLockJustification version: ${version}`);
        }
        return new TronUsdtLockJustification({
            chainId: Number(CborDeserializer.decodeUnsignedInteger(items[1])),
            lockContract: CborDeserializer.decodeByteString(items[2]),
            assetContract: CborDeserializer.decodeByteString(items[3]),
            txid: CborDeserializer.decodeByteString(items[4]),
            logIndex: Number(CborDeserializer.decodeUnsignedInteger(items[5])),
            amount: CborDeserializer.decodeUnsignedInteger(items[6]),
            nonce: CborDeserializer.decodeUnsignedInteger(items[7]),
        });
    }
}
