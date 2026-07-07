/**
 * Cross-stack conformance: the TS bridge-back derivations must reproduce the
 * Rust-generated `protocol/vectors` byte-for-byte (interop §10; TS must pass config,
 * reason, nullifier, lock-recipientCommitment, public). This is the CBOR/ABI
 * guard that keeps the wallet's bytes consumable by the circuit and the vault.
 */
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { test } from 'node:test';

import { fromHex, toHex } from '../src/hex.js';
import { recipientCommitment } from '../src/index.js';
import {
  type BridgeConfig,
  burnTransitionId,
  configHash,
  domainTag,
  encodeBridgeBackReason,
  lockDigest,
  lockRefRoot,
  nullifier,
  publicValuesAbi,
  reasonHash,
  returnRoot,
} from '../src/bridge-back/index.js';

function vector(group: string, file: string): any {
  const url = new URL(`../../../protocol/vectors/${group}/${file}`, import.meta.url);
  return JSON.parse(readFileSync(url, 'utf8'));
}

const hx = (s: string) => fromHex(s);
/** Assert two byte arrays are equal, reporting hex on failure. */
const eqHex = (got: Uint8Array, wantHex: string, msg?: string) =>
  assert.equal('0x' + toHex(got), wantHex, msg);

function configFromVector(): BridgeConfig {
  const c = vector('config', 'config-00.json');
  return {
    sourceChainId: BigInt(c.in.source_chain_id),
    vault: hx(c.in.vault),
    asset: hx(c.in.asset),
    tokenType: hx(c.out.token_type),
    coinId: hx(c.out.coin_id),
    reasonTag: BigInt(c.in.reason_tag),
    lockDomain: hx(c.in.lock_domain),
    nullifierDomain: hx(c.in.nullifier_domain),
  };
}

test('VERSION matches the pinned vectors', () => {
  const v = readFileSync(new URL('../../../protocol/vectors/VERSION', import.meta.url), 'utf8').trim();
  assert.equal(v, '1');
});

test('config -> configHash (00 §2)', () => {
  const c = vector('config', 'config-00.json');
  eqHex(configHash(configFromVector()), c.out.config_hash);
});

test('lock -> recipientCommitment + lockDigest (00 §3)', () => {
  const cfg = configFromVector();
  const lk = vector('lock', 'lock-00.json');

  // recipientCommitment = SHA256(recipient predicate CBOR)
  eqHex(recipientCommitment(hx(lk.in.recipient_cbor)), lk.out.recipient_commitment);

  const digest = lockDigest({
    sourceChainId: cfg.sourceChainId,
    vault: cfg.vault,
    nonce: BigInt(lk.in.nonce),
    asset: cfg.asset,
    tokenType: cfg.tokenType,
    coinId: cfg.coinId,
    amount: BigInt(lk.in.amount),
    unicityTokenId: hx(lk.in.unicity_token_id),
    recipientCommitment: hx(lk.out.recipient_commitment),
  });
  eqHex(digest, lk.out.lock_digest);
});

test('reason -> canonical CBOR + reasonHash (00 §4)', () => {
  const cfg = configFromVector();
  const r = vector('reason', 'reason-00.json');
  const reasonBytes = encodeBridgeBackReason(cfg, {
    version: BigInt(r.in.version),
    recipient: hx(r.in.recipient),
    amount: BigInt(r.in.amount),
    feeRecipient: hx(r.in.fee_recipient),
    feeAmount: BigInt(r.in.fee_amount),
    deadline: BigInt(r.in.deadline),
  });
  eqHex(reasonBytes, r.out.reason_cbor, 'reason CBOR mismatch');
  eqHex(reasonHash(reasonBytes), r.out.reason_hash, 'reasonHash mismatch');
});

test('nullifier -> burnTransitionId + nullifier (00 §5)', () => {
  const n = vector('nullifier', 'nullifier-00.json');
  const btId = burnTransitionId(hx(n.in.state_id), hx(n.in.tx_hash));
  eqHex(btId, n.out.burn_transition_id, 'burnTransitionId mismatch');
  eqHex(nullifier(hx(n.in.config_hash), btId), n.out.nullifier, 'nullifier mismatch');
});

test('public -> domainTag / returnRoot / lockRefRoot / PublicValues ABI (00 §7)', () => {
  const p = vector('public', 'public-00.json');

  eqHex(domainTag(), p.out.domain_tag, 'domainTag mismatch');

  const leaves = p.in.leaves.map((l: any) => ({
    nullifier: hx(l.nullifier),
    recipient: hx(l.recipient),
    amount: BigInt(l.amount),
    feeRecipient: hx(l.fee_recipient),
    feeAmount: BigInt(l.fee_amount),
    deadline: BigInt(l.deadline),
  }));
  eqHex(returnRoot(leaves), p.out.return_root, 'returnRoot mismatch');

  const refs = p.in.lock_refs.map((r: any) => ({ nonce: BigInt(r.nonce), digest: hx(r.digest) }));
  eqHex(lockRefRoot(refs), p.out.lock_ref_root, 'lockRefRoot mismatch');

  const pvAbi = publicValuesAbi({
    domainTag: hx(p.out.domain_tag),
    configHash: hx(p.out.config_hash),
    trustBaseHash: hx(p.in.trust_base_hash),
    spentRootOld: hx(p.in.spent_root_old),
    spentRootNew: hx(p.in.spent_root_new),
    returnRoot: hx(p.out.return_root),
    lockRefRoot: hx(p.out.lock_ref_root),
    batchSize: Number(p.in.batch_size),
    totalAmount: BigInt(p.in.total_amount),
  });
  eqHex(pvAbi, p.out.public_values_abi, 'PublicValues ABI mismatch');
});

test('lockRefRoot rejects duplicate nonces', () => {
  const d = new Uint8Array(32);
  assert.throws(
    () =>
      lockRefRoot([
        { nonce: 1n, digest: d },
        { nonce: 1n, digest: d },
      ]),
    /duplicate nonce/,
  );
});
