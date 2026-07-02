/**
 * Wallet façade (06 §W0). The only surface Sphere calls — verify the manifest
 * loader's integrity-pin, the bridge-in plan derivation, and that the loaded
 * plugin's identifiers/configHash match the frozen Nile deployment
 * (`bridge-vectors/deployment/nile-usdt.json`, cross-stack freeze).
 */
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { test } from 'node:test';

import { EncodedPredicate } from '@unicitylabs/state-transition-sdk/lib/predicate/EncodedPredicate.js';
import { SignaturePredicate } from '@unicitylabs/state-transition-sdk/lib/predicate/builtin/SignaturePredicate.js';
import { SigningService } from '@unicitylabs/state-transition-sdk/lib/crypto/secp256k1/SigningService.js';

import { fromHex, recipientCommitment, toHex } from '../src/index.js';
import {
  buildBridgeInPlan,
  loadBridges,
  NILE_USDT_BRIDGE,
  MAX_UINT256,
} from '../src/wallet/index.js';
import { MockTronRpc } from '../src/cli/scenario.js';

function frozen(): any {
  const url = new URL('../../bridge-vectors/deployment/nile-usdt.json', import.meta.url);
  return JSON.parse(readFileSync(url, 'utf8'));
}

const deps = () => ({ rpc: new MockTronRpc(null, 0n) });

test('loadBridges resolves the Nile manifest (internal consistency + stable ids)', () => {
  const [loaded] = loadBridges(NILE_USDT_BRIDGE, deps());
  const c = frozen().config;

  // tokenType/coinId/asset are asset-derived — stable across vault redeploys, so
  // they still match the frozen reference even though the vault was redeployed.
  assert.equal('0x' + loaded.plugin.tokenTypeHex, c.token_type, 'tokenType');
  assert.equal('0x' + loaded.plugin.coinIdHex, c.coin_id, 'coinId');
  assert.equal('0x' + loaded.plugin.resolvedConfig.assetContractHex, c.asset, 'asset');

  // The integrity pin: the derived configHash equals the manifest's declared one
  // (loadBridges throws otherwise) — the live 39048 vault (integrity pin).
  assert.equal(toHex(loaded.configHash), NILE_USDT_BRIDGE.configHash, 'configHash internal-consistent');
  assert.equal(loaded.bridgeConfig.reasonTag, 39048n, 'reason_tag 39048');
});

test('loadBridges rejects a manifest whose configHash does not match its fields', () => {
  const bad = { ...NILE_USDT_BRIDGE, configHash: '00'.repeat(32) };
  assert.throws(() => loadBridges(bad, deps()), /configHash mismatch/);
});

test('loadBridges rejects a tokenTypeHex that does not match (integrity pin)', () => {
  const bad = { ...NILE_USDT_BRIDGE, tokenTypeHex: 'ab'.repeat(32) };
  assert.throws(() => loadBridges(bad, deps()), /tokenTypeHex mismatch/);
});

test('buildBridgeInPlan derives tokenId + recipientCommitment + Tron calls', async () => {
  const [loaded] = loadBridges(NILE_USDT_BRIDGE, deps());
  const owner = SignaturePredicate.create(SigningService.generate().publicKey);
  const ownerCbor = EncodedPredicate.fromPredicate(owner).toCBOR();

  const plan = await buildBridgeInPlan({
    plugin: loaded.plugin,
    amount: 5_000_000n,
    networkId: 4,
    ownerPredicateCbor: ownerCbor,
  });

  // recipientCommitment is exactly SHA256(ownerPredicateCbor) (00 §lock).
  assert.equal(plan.recipientCommitmentHex, toHex(recipientCommitment(ownerCbor)));
  assert.equal(plan.tokenIdHex.length, 64);
  assert.equal(plan.amount, 5_000_000n);

  // approve(asset → vault, amount); lock(amount, tokenId, recipientCommitment).
  assert.equal(plan.approve.functionSignature, 'approve(address,uint256)');
  assert.equal(plan.approve.contractHex, '41' + loaded.plugin.resolvedConfig.assetContractHex);
  assert.equal(plan.approve.parameters[0].value, '41' + loaded.plugin.resolvedConfig.lockContractHex);
  assert.equal(plan.approve.parameters[1].value, '5000000');

  assert.equal(plan.lock.functionSignature, 'lock(uint256,bytes32,bytes32)');
  assert.equal(plan.lock.contractHex, '41' + loaded.plugin.resolvedConfig.lockContractHex);
  assert.equal(plan.lock.parameters[0].value, '5000000');
  assert.equal(plan.lock.parameters[1].value, '0x' + plan.tokenIdHex);
  assert.equal(plan.lock.parameters[2].value, '0x' + plan.recipientCommitmentHex);

  // The salt re-derives the same tokenId (the wallet keeps it to mint THAT token).
  assert.equal(fromHex(plan.saltHex).length, 32);
});

test('buildBridgeInPlan(recipientPubkey) commits to SignaturePredicate(pubkey)', async () => {
  const [loaded] = loadBridges(NILE_USDT_BRIDGE, deps());
  const pubkey = SigningService.generate().publicKey;
  // The façade must commit to exactly the predicate the engine mints to.
  const expected = toHex(
    recipientCommitment(EncodedPredicate.fromPredicate(SignaturePredicate.create(pubkey)).toCBOR()),
  );
  const plan = await buildBridgeInPlan({
    plugin: loaded.plugin,
    amount: 1_000_000n,
    networkId: 4,
    recipientPubkey: pubkey,
  });
  assert.equal(plan.recipientCommitmentHex, expected);
});

test('buildBridgeInPlan honors a one-time max approve', async () => {
  const [loaded] = loadBridges(NILE_USDT_BRIDGE, deps());
  const owner = SignaturePredicate.create(SigningService.generate().publicKey);
  const plan = await buildBridgeInPlan({
    plugin: loaded.plugin,
    amount: 1n,
    networkId: 4,
    ownerPredicateCbor: EncodedPredicate.fromPredicate(owner).toCBOR(),
    approveAmount: MAX_UINT256,
  });
  assert.equal(plan.approve.parameters[1].value, MAX_UINT256.toString());
  assert.equal(plan.lock.parameters[0].value, '1');
});
