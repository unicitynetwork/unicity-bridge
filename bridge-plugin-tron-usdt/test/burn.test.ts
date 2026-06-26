/**
 * Bridge-back burn construction (02 §2a–2c): the wallet produces the canonical
 * reason bytes, the binding BurnPredicate(H(reasonBytes)), the read-only return
 * preview, and the prover hand-off envelope. The conformance-critical bytes are
 * cross-checked against `bridge-vectors`; the SDK binding is checked by decoding
 * the predicate back.
 */
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { test } from 'node:test';

import { BuiltInPredicateType } from '@unicitylabs/state-transition-sdk/lib/predicate/builtin/BuiltInPredicateType.js';

import { fromHex, toHex } from '../src/hex.js';
import {
  type BridgeConfig,
  buildBridgeBackBurnReason,
  buildWitnessRequest,
  decodeBridgeBackReason,
  previewReturn,
  type BridgeBackReason,
} from '../src/bridge-back/index.js';

function vector(group: string, file: string): any {
  return JSON.parse(readFileSync(new URL(`../../bridge-vectors/${group}/${file}`, import.meta.url), 'utf8'));
}
const hx = (s: string) => fromHex(s);
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

function reasonFromVector(): BridgeBackReason {
  const r = vector('reason', 'reason-00.json');
  return {
    version: BigInt(r.in.version),
    recipient: hx(r.in.recipient),
    amount: BigInt(r.in.amount),
    feeRecipient: hx(r.in.fee_recipient),
    feeAmount: BigInt(r.in.fee_amount),
    deadline: BigInt(r.in.deadline),
  };
}

test('buildBridgeBackBurnReason binds BurnPredicate(H(reasonBytes)) with reasonBytes as aux (00 §4)', () => {
  const r = vector('reason', 'reason-00.json');
  const { reasonBytes, reasonHash, burnPredicate } = buildBridgeBackBurnReason(
    configFromVector(),
    reasonFromVector(),
  );

  // the aux-data bytes are the canonical reason; the predicate binds their hash
  eqHex(reasonBytes, r.out.reason_cbor);
  eqHex(reasonHash, r.out.reason_hash);

  // the SDK predicate is a Burn predicate whose payload is exactly reasonHash
  assert.equal(burnPredicate.type, BuiltInPredicateType.Burn);
  eqHex(burnPredicate.encodeParameters(), r.out.reason_hash, 'predicate payload must equal reasonHash');
});

test('decodeBridgeBackReason round-trips the canonical reason (self-contained blob)', () => {
  const cfg = configFromVector();
  const { reasonBytes } = buildBridgeBackBurnReason(cfg, reasonFromVector());
  const d = decodeBridgeBackReason(reasonBytes);

  const r = vector('reason', 'reason-00.json');
  assert.equal(d.reasonTag, BigInt(r.in.reason_tag));
  assert.equal(d.version, 1n);
  assert.equal(d.sourceChainId, cfg.sourceChainId);
  eqHex(d.vault, '0x' + toHex(cfg.vault));
  eqHex(d.tokenType, '0x' + toHex(cfg.tokenType));
  eqHex(d.recipient, r.in.recipient);
  assert.equal(d.amount, BigInt(r.in.amount));
  assert.equal(d.feeAmount, BigInt(r.in.fee_amount));
  assert.equal(d.deadline, BigInt(r.in.deadline));
});

test('decodeBridgeBackReason rejects trailing bytes', () => {
  const { reasonBytes } = buildBridgeBackBurnReason(configFromVector(), reasonFromVector());
  const withTrailer = new Uint8Array(reasonBytes.length + 1);
  withTrailer.set(reasonBytes);
  assert.throws(() => decodeBridgeBackReason(withTrailer), /trailing bytes/);
});

test('previewReturn derives the pending nullifier + return leaf (matches nullifier vector)', () => {
  const n = vector('nullifier', 'nullifier-00.json');
  const cfg = configFromVector();
  const r = reasonFromVector();
  // use the nullifier fixture's burn state id / tx hash + its configHash
  const preview = previewReturn(hx(n.in.config_hash), r, hx(n.in.state_id), hx(n.in.tx_hash));
  eqHex(preview.burnTransitionId, n.out.burn_transition_id);
  eqHex(preview.nullifier, n.out.nullifier);
  // the leaf carries the reason's release params + the derived nullifier
  eqHex(preview.returnLeaf.nullifier, n.out.nullifier);
  assert.equal(preview.returnLeaf.amount, r.amount);
  assert.equal(preview.returnLeaf.deadline, r.deadline);
  // configHash field of cfg is unused by previewReturn (caller passes it in)
  assert.ok(cfg.vault.length === 20);
});

test('buildWitnessRequest assembles the prover hand-off envelope (02 §2c)', () => {
  const cfg = configFromVector();
  const { reasonBytes } = buildBridgeBackBurnReason(cfg, reasonFromVector());
  const tokenCbor = Uint8Array.of(1, 2, 3);
  const configHash = hx(vector('config', 'config-00.json').out.config_hash);

  const base = buildWitnessRequest({ tokenCbor, configHash, reasonBytes });
  assert.equal(base.anchorHint, undefined);
  eqHex(base.reasonBytes, '0x' + toHex(reasonBytes));
  eqHex(base.configHash, '0x' + toHex(configHash));

  const withHint = buildWitnessRequest({ tokenCbor, configHash, reasonBytes, anchorHint: 42n });
  assert.equal(withHint.anchorHint, 42n);
});
