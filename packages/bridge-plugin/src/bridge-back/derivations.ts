/**
 * The bridge-back derivations from protocol/interop.md.
 * Each function is the TS half of one cross-stack value and reproduces the Rust
 * reference (`protocol/vectors/gen/src/derive.rs`) byte-for-byte. The wallet uses
 * these read-only to show the user the pending nullifier / release leaf, and a
 * TS relayer reuses them; the prover (Rust) and the vault (Solidity) recompute
 * the same bytes (00 §2–7).
 */
import { sha256 } from '@noble/hashes/sha2.js';

import * as abi from './abi.js';
import * as cbor from './cbor.js';

// Domain separators (00 §2, §3, §5, §7) — must match derive.rs exactly.
export const DOMAIN_CONFIG = 'unicity-bridge-return-config:v1';
export const DOMAIN_LOCK = 'unicity-bridge-lock:v1';
export const DOMAIN_RETURN = 'unicity-bridge-return:v1';
export const DOMAIN_BURN_TRANSITION = 'unicity-burn-transition:v1';
export const DOMAIN_NULLIFIER = 'unicity-bridge-return-nullifier:v1';

/** Deployment configuration (00 §2). Byte fields are raw (no `0x`). */
export interface BridgeConfig {
  readonly sourceChainId: bigint;
  readonly vault: Uint8Array; // 20 bytes
  readonly asset: Uint8Array; // 20 bytes
  readonly tokenType: Uint8Array; // 32 bytes
  readonly coinId: Uint8Array; // 32 bytes
  readonly reasonTag: bigint;
  readonly lockDomain: Uint8Array; // 32 bytes
  readonly nullifierDomain: Uint8Array; // 32 bytes
}

/** 32-byte big-endian representation of an unsigned integer. */
export function u256be(n: bigint): Uint8Array {
  if (n < 0n) throw new Error('u256be: negative');
  const out = new Uint8Array(32);
  for (let i = 0; i < 32 && n > 0n; i++) {
    out[31 - i] = Number(n & 0xffn);
    n >>= 8n;
  }
  if (n > 0n) throw new Error('u256be: overflow');
  return out;
}

/** `configHash = K(abi.encode("...config:v1", fields...))` (00 §2). */
export function configHash(c: BridgeConfig): Uint8Array {
  return abi.keccak256(
    abi.encode([
      abi.Str(DOMAIN_CONFIG),
      abi.U64(c.sourceChainId),
      abi.Addr(c.vault),
      abi.Addr(c.asset),
      abi.B32(c.tokenType),
      abi.B32(c.coinId),
      abi.U64(c.reasonTag),
      abi.B32(c.lockDomain),
      abi.B32(c.nullifierDomain),
    ]),
  );
}

/** `domainTag = K("unicity-bridge-return:v1")` over raw bytes (00 §7). */
export function domainTag(): Uint8Array {
  return abi.keccak256(new TextEncoder().encode(DOMAIN_RETURN));
}

/** `lockDigest = K(abi.encode("...lock:v1", fields...))` (00 §3). */
export function lockDigest(params: {
  sourceChainId: bigint;
  vault: Uint8Array;
  nonce: bigint;
  asset: Uint8Array;
  tokenType: Uint8Array;
  coinId: Uint8Array;
  amount: bigint;
  unicityTokenId: Uint8Array;
  recipientCommitment: Uint8Array;
}): Uint8Array {
  return abi.keccak256(
    abi.encode([
      abi.Str(DOMAIN_LOCK),
      abi.U64(params.sourceChainId),
      abi.Addr(params.vault),
      abi.U256(params.nonce),
      abi.Addr(params.asset),
      abi.B32(params.tokenType),
      abi.B32(params.coinId),
      abi.U256(params.amount),
      abi.B32(params.unicityTokenId),
      abi.B32(params.recipientCommitment),
    ]),
  );
}

/** Per-return parameters of a `BridgeBackReason` (00 §4); config-bound fields
 *  (sourceChainId/vault/asset/tokenType/coinId) come from {BridgeConfig}. */
export interface BridgeBackReason {
  readonly version: bigint; // = 1n
  readonly recipient: Uint8Array; // 20 bytes — external release recipient
  readonly amount: bigint; // gross
  readonly feeRecipient: Uint8Array; // 20 bytes — zero address ⇒ no fee
  readonly feeAmount: bigint; // ≤ amount
  readonly deadline: bigint; // gates the fee only
}

/**
 * Canonical CBOR of the 11-field `BridgeBackReason` array under `reasonTag`
 * (00 §4). This is the `reasonBytes` that ride in the terminal burn's auxiliary
 * data; the burn predicate binds `H(reasonBytes)` (see {reasonHash}).
 */
export function encodeBridgeBackReason(c: BridgeConfig, r: BridgeBackReason): Uint8Array {
  return cbor.concatBytes([
    cbor.tag(c.reasonTag),
    cbor.arrayHeader(11),
    cbor.uint(r.version),
    cbor.uint(c.sourceChainId),
    cbor.bytes(c.vault),
    cbor.bytes(c.asset),
    cbor.bytes(c.tokenType),
    cbor.bytes(c.coinId),
    cbor.bytes(r.recipient),
    cbor.bytes(cbor.minimalBe(u256be(r.amount))),
    cbor.bytes(r.feeRecipient),
    cbor.bytes(cbor.minimalBe(u256be(r.feeAmount))),
    cbor.uint(r.deadline),
  ]);
}

/** `reasonHash = H(reasonBytes)` — the value `BurnPredicate(H(reasonBytes))`
 *  binds (00 §4, PROVISIONAL preimage pending M0 SDK confirmation). */
export function reasonHash(reasonBytes: Uint8Array): Uint8Array {
  return sha256(reasonBytes);
}

/** The fully-decoded `BridgeBackReason`, including its config-bound fields. */
export interface DecodedBridgeBackReason {
  readonly reasonTag: bigint;
  readonly version: bigint;
  readonly sourceChainId: bigint;
  readonly vault: Uint8Array;
  readonly asset: Uint8Array;
  readonly tokenType: Uint8Array;
  readonly coinId: Uint8Array;
  readonly recipient: Uint8Array;
  readonly amount: bigint;
  readonly feeRecipient: Uint8Array;
  readonly feeAmount: bigint;
  readonly deadline: bigint;
}

/**
 * Inverse of {encodeBridgeBackReason}: decode the canonical `reasonBytes` a
 * burned token blob carries (the circuit performs the same decode). Strict —
 * rejects non-canonical CBOR and any trailing bytes (00 §4). `amount`/`feeAmount`
 * come back as the minimal-big-endian byte strings re-read as integers.
 */
export function decodeBridgeBackReason(reasonBytes: Uint8Array): DecodedBridgeBackReason {
  const r = new cbor.CborReader(reasonBytes);
  const reasonTag = r.readTag();
  const n = r.readArrayHeader();
  if (n !== 11) throw new Error(`BridgeBackReason: expected 11 fields, got ${n}`);
  const version = r.readUint();
  const sourceChainId = r.readUint();
  const vault = r.readBytes();
  const asset = r.readBytes();
  const tokenType = r.readBytes();
  const coinId = r.readBytes();
  const recipient = r.readBytes();
  const amount = beToBigInt(r.readBytes());
  const feeRecipient = r.readBytes();
  const feeAmount = beToBigInt(r.readBytes());
  const deadline = r.readUint();
  if (!r.done) throw new Error('BridgeBackReason: trailing bytes after reason');
  return {
    reasonTag,
    version,
    sourceChainId,
    vault,
    asset,
    tokenType,
    coinId,
    recipient,
    amount,
    feeRecipient,
    feeAmount,
    deadline,
  };
}

function beToBigInt(b: Uint8Array): bigint {
  let n = 0n;
  for (const x of b) n = (n << 8n) | BigInt(x);
  return n;
}

/** `burnTransitionId = H("unicity-burn-transition:v1", stateId, txHash)` (00 §5). */
export function burnTransitionId(stateId: Uint8Array, txHash: Uint8Array): Uint8Array {
  return cbor.hArray([cbor.text(DOMAIN_BURN_TRANSITION), cbor.bytes(stateId), cbor.bytes(txHash)]);
}

/** `nullifier = H("...nullifier:v1", configHash, burnTransitionId)` (00 §5). */
export function nullifier(configHashBytes: Uint8Array, burnTransitionIdBytes: Uint8Array): Uint8Array {
  return cbor.hArray([
    cbor.text(DOMAIN_NULLIFIER),
    cbor.bytes(configHashBytes),
    cbor.bytes(burnTransitionIdBytes),
  ]);
}

/** One settlement leaf committed by `returnRoot`, in submission order (00 §7). */
export interface ReturnLeaf {
  readonly nullifier: Uint8Array; // 32 bytes
  readonly recipient: Uint8Array; // 20 bytes
  readonly amount: bigint;
  readonly feeRecipient: Uint8Array; // 20 bytes
  readonly feeAmount: bigint;
  readonly deadline: bigint;
}

function leafWords(l: ReturnLeaf): abi.Val[] {
  return [
    abi.B32(l.nullifier),
    abi.Addr(l.recipient),
    abi.U256(l.amount),
    abi.Addr(l.feeRecipient),
    abi.U256(l.feeAmount),
    abi.U64(l.deadline),
  ];
}

/** `returnRoot = K( concat of fixed-width abi.encode(leaf) )` (00 §7). */
export function returnRoot(leaves: ReturnLeaf[]): Uint8Array {
  const parts = leaves.map((l) => abi.packWords(leafWords(l)));
  return abi.keccak256(concatU8(parts));
}

/** One source-lock reference committed by `lockRefRoot` (00 §7). */
export interface SourceLockRef {
  readonly nonce: bigint;
  readonly digest: Uint8Array; // 32 bytes
}

/** `lockRefRoot = K( concat of fixed-width abi.encode(ref) )`, sorted by nonce
 *  with duplicates rejected (00 §7). */
export function lockRefRoot(refs: SourceLockRef[]): Uint8Array {
  const sorted = [...refs].sort((a, b) => (a.nonce < b.nonce ? -1 : a.nonce > b.nonce ? 1 : 0));
  for (let i = 1; i < sorted.length; i++) {
    if (sorted[i].nonce === sorted[i - 1].nonce) {
      throw new Error(`lockRefRoot: duplicate nonce ${sorted[i].nonce}`);
    }
  }
  const parts = sorted.map((r) => abi.packWords([abi.U256(r.nonce), abi.B32(r.digest)]));
  return abi.keccak256(concatU8(parts));
}

/** The public statement the circuit commits and the vault decodes (00 §7). */
export interface PublicValues {
  readonly domainTag: Uint8Array;
  readonly configHash: Uint8Array;
  readonly trustBaseHash: Uint8Array;
  readonly spentRootOld: Uint8Array;
  readonly spentRootNew: Uint8Array;
  readonly returnRoot: Uint8Array;
  readonly lockRefRoot: Uint8Array;
  readonly batchSize: number;
  readonly totalAmount: bigint;
}

/** `abi.encode(PublicValues)` — the all-static layout the vault abi.decodes (00 §7). */
export function publicValuesAbi(p: PublicValues): Uint8Array {
  return abi.encode([
    abi.B32(p.domainTag),
    abi.B32(p.configHash),
    abi.B32(p.trustBaseHash),
    abi.B32(p.spentRootOld),
    abi.B32(p.spentRootNew),
    abi.B32(p.returnRoot),
    abi.B32(p.lockRefRoot),
    abi.U32(p.batchSize),
    abi.U256(p.totalAmount),
  ]);
}

function concatU8(parts: Uint8Array[]): Uint8Array {
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
