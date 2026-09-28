import { CborDeserializer } from '@unicitylabs/state-transition-sdk/lib/serialization/cbor/CborDeserializer.js';
import { CborSerializer } from '@unicitylabs/state-transition-sdk/lib/serialization/cbor/CborSerializer.js';

/**
 * CBOR tag of the bridge lock justification, the same on every source chain:
 * the chain id inside the justification tells the families apart, and the
 * wallet's verifier for this tag dispatches on it.
 */
export const BRIDGE_LOCK_JUSTIFICATION_TAG = 1330002n;

const VERSION = 1n;

/** Decoded contents of a lock justification (the token's mint reason). */
export interface BridgeLockJustificationData {
  /** Source chain id (Tron mainnet 728126428, Nile 3448148188, Sepolia 11155111). */
  readonly chainId: number;
  /** 20-byte address of the canonical vault (lock) contract. */
  readonly lockContract: Uint8Array;
  /** 20-byte address of the bridged asset's token contract. */
  readonly assetContract: Uint8Array;
  /** 32-byte transaction hash of the lock() call. */
  readonly txid: Uint8Array;
  /** Index of the Lock event within that transaction's logs. */
  readonly logIndex: number;
  /** Locked amount in the asset's smallest unit. */
  readonly amount: bigint;
  /** Lock nonce assigned by the contract (echoed in the Lock event). */
  readonly nonce: bigint;
}

/** Encodes/decodes the self-contained lock proof carried in a token's mint reason. */
export class BridgeLockJustification {
  public static readonly CBOR_TAG = BRIDGE_LOCK_JUSTIFICATION_TAG;

  public constructor(public readonly data: BridgeLockJustificationData) {}

  public toCBOR(): Uint8Array {
    const d = this.data;
    return CborSerializer.encodeTag(
      BridgeLockJustification.CBOR_TAG,
      CborSerializer.encodeArray(
        CborSerializer.encodeUnsignedInteger(VERSION),
        CborSerializer.encodeUnsignedInteger(d.chainId),
        CborSerializer.encodeByteString(d.lockContract),
        CborSerializer.encodeByteString(d.assetContract),
        CborSerializer.encodeByteString(d.txid),
        CborSerializer.encodeUnsignedInteger(d.logIndex),
        CborSerializer.encodeUnsignedInteger(d.amount),
        CborSerializer.encodeUnsignedInteger(d.nonce),
      ),
    );
  }

  public static fromCBOR(bytes: Uint8Array): BridgeLockJustification {
    const tag = CborDeserializer.decodeTag(bytes);
    if (tag.tag !== BridgeLockJustification.CBOR_TAG) {
      throw new Error(`Invalid CBOR tag for BridgeLockJustification: ${tag.tag}`);
    }
    const items = CborDeserializer.decodeArray(tag.data, 8);
    const version = CborDeserializer.decodeUnsignedInteger(items[0]);
    if (version !== VERSION) {
      throw new Error(`Unsupported BridgeLockJustification version: ${version}`);
    }
    return new BridgeLockJustification({
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
