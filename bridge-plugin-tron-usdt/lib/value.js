import { CborDeserializer } from '@unicitylabs/state-transition-sdk/lib/serialization/cbor/CborDeserializer.js';
import { CborSerializer } from '@unicitylabs/state-transition-sdk/lib/serialization/cbor/CborSerializer.js';
import { bytesEqual } from './hex.js';
/**
 * Minimal self-contained value envelope used by the CLI/tests:
 * `CBOR [ coinId: bstr, amount: uint ]`.
 */
export function encodeBridgedValue(coinId, amount) {
    return CborSerializer.encodeArray(CborSerializer.encodeByteString(coinId), CborSerializer.encodeUnsignedInteger(amount));
}
export function decodeBridgedValue(data, coinId) {
    if (!data) {
        return null;
    }
    let items;
    try {
        items = CborDeserializer.decodeArray(data, 2);
    }
    catch {
        return null;
    }
    const encodedCoinId = CborDeserializer.decodeByteString(items[0]);
    if (!bytesEqual(encodedCoinId, coinId)) {
        return null;
    }
    return CborDeserializer.decodeUnsignedInteger(items[1]);
}
