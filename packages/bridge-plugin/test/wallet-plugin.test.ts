import assert from 'node:assert/strict';
import { test } from 'node:test';

import { mintBridgedToken, type BridgePayments } from '@unicitylabs/bridge-core';
import { VerificationStatus } from '@unicitylabs/state-transition-sdk/lib/verification/VerificationStatus.js';

import { MockTronRpc } from '../src/cli/scenario.js';
import {
  createBridgePlugin,
  decodeBridgePaymentData,
  toHex,
  TRON_NILE_CHAIN_ID,
  BRIDGE_LOCK_JUSTIFICATION_TAG,
  BridgeLockJustification,
} from '../src/index.js';
import {
  bridgeTokenPlugin,
  createSourceAdapter,
  loadBridges,
  mergeBridgeTokenPlugins,
  mintedAgainst,
  NILE_USDT_BRIDGE,
  NILE_USDT_BRIDGE_V1,
  SEPOLIA_USDC_BRIDGE,
} from '../src/wallet/index.js';
import { BridgeMintJustificationVerifier, LockMintJustificationVerifier, fromHex } from '../src/index.js';
import { buildScenario, CONFIG } from './helpers.js';

const deps = () => ({ rpc: new MockTronRpc(null, 0n) });
const noNestedTokens = (): void => {};

test('bridgeTokenPlugin: one plugin per bridged asset, carrying the STRICT verifier under the asset tag', () => {
  const [loaded] = loadBridges(NILE_USDT_BRIDGE, deps());
  const plugin = bridgeTokenPlugin(loaded!);

  assert.equal(plugin.id, `bridge:${NILE_USDT_BRIDGE.chainRef}:usdt`);
  assert.equal(plugin.mintJustificationVerifiers[0], loaded!.plugin.verifier);
  assert.ok(plugin.mintJustificationVerifiers.every((v) => v.tag === BRIDGE_LOCK_JUSTIFICATION_TAG));
});

test('bridgeTokenPlugin: the bridged token type must be minted against a lock or split off one, and alone issues the bridged coin', () => {
  const [loaded] = loadBridges(SEPOLIA_USDC_BRIDGE, deps());
  const plugin = bridgeTokenPlugin(loaded!);

  assert.equal(plugin.tokenIssuancePolicies.length, 1);
  assert.equal(toHex(plugin.tokenIssuancePolicies[0]!.tokenType.bytes), loaded!.plugin.tokenTypeHex);
  assert.deepEqual(plugin.tokenIssuancePolicies[0]!.coinIds, [loaded!.plugin.coinIdHex]);
});

test('a replaced vault stays verify-only: its locks still back tokens of the same type', () => {
  const [loaded] = loadBridges(NILE_USDT_BRIDGE, deps());
  const plugin = bridgeTokenPlugin(loaded!);
  const verifiers = plugin.mintJustificationVerifiers as LockMintJustificationVerifier[];
  const lockedAt = (vault: string) => ({ chainId: NILE_USDT_BRIDGE.chainId, lockContract: fromHex(vault) }) as never;
  const v1 = loadBridges(NILE_USDT_BRIDGE_V1, deps())[0]!.plugin.resolvedConfig.lockContractHex;

  assert.deepEqual(
    verifiers.map((v) => v.accepts(lockedAt(v1))),
    [false, true],
  );
  assert.equal(verifiers[0], loaded!.plugin.verifier);
  assert.equal(plugin.tokenIssuancePolicies.length, 1);
});

test('mergeBridgeTokenPlugins: one dispatching verifier over every vault, and every bridged type keeps its policy', () => {
  const plugins = loadBridges([NILE_USDT_BRIDGE, SEPOLIA_USDC_BRIDGE], deps()).map(bridgeTokenPlugin);
  const merged = mergeBridgeTokenPlugins(plugins);

  assert.equal(merged.mintJustificationVerifiers.length, 1);
  assert.ok(merged.mintJustificationVerifiers[0] instanceof BridgeMintJustificationVerifier);
  assert.deepEqual(
    merged.tokenIssuancePolicies.map((p) => toHex(p.tokenType.bytes)),
    plugins.map((p) => toHex(p.tokenIssuancePolicies[0]!.tokenType.bytes)),
  );
});

test('the adapter mint request carries the wallet-format value, the lock reason, and a 0-confirmation self verifier', async () => {
  const [loaded] = loadBridges(NILE_USDT_BRIDGE, deps());
  const wallet = { getAddress: async () => 'TAddr', sendCall: async () => 'txid' };
  const rpc = { allowance: async () => 0n } as never;
  const adapter = createSourceAdapter(loaded!, wallet, rpc, deps());

  const req = adapter.buildMintRequest({
    saltHex: 'a5'.repeat(32),
    amount: 1_000_000n,
    commit: { nonce: 19n, blockNumber: 71_070_991n, logIndex: 0 },
    commitTxid: 'e7'.repeat(32),
  });

  assert.equal(decodeBridgePaymentData(req.mintData, loaded!.plugin.resolvedConfig.coinId), 1_000_000n);
  assert.equal(toHex(req.tokenType), loaded!.plugin.tokenTypeHex);
  assert.equal(toHex(req.salt), 'a5'.repeat(32));
  assert.equal(req.coinIdHex, loaded!.plugin.coinIdHex);
  assert.equal(req.mintJustificationVerifiers.length, 1);
  assert.equal(req.mintJustificationVerifiers[0]!.tag, BRIDGE_LOCK_JUSTIFICATION_TAG);
  assert.notEqual(req.mintJustificationVerifiers[0], loaded!.plugin.verifier, 'the self verifier is a separate, weaker instance');
});

test('the self verifier accepts a lock that is in a block but not final; the strict one is not ready to answer', async () => {
  const s = await buildScenario({ tip: 101n });
  await assert.rejects(s.plugin.verifier.verify(s.certifiedTx, noNestedTokens), /Insufficient confirmations: 1 < 20/);

  const self = createBridgePlugin({ ...CONFIG, confirmations: 0 }, { rpc: s.rpc }).verifier;
  const own = await self.verify(s.certifiedTx, noNestedTokens);
  assert.equal(own.status, VerificationStatus.OK, own.message);
});

test('mintBridgedToken maps the adapter request onto the wallet custom mint 1:1', async () => {
  const [loaded] = loadBridges(NILE_USDT_BRIDGE, deps());
  const wallet = { getAddress: async () => 'TAddr', sendCall: async () => 'txid' };
  const adapter = createSourceAdapter(loaded!, wallet, { allowance: async () => 0n } as never, deps());
  const req = adapter.buildMintRequest({
    saltHex: 'a5'.repeat(32),
    amount: 5n,
    commit: { nonce: 1n, blockNumber: 2n, logIndex: 0 },
    commitTxid: 'e7'.repeat(32),
  });
  const calls: unknown[] = [];
  const payments: BridgePayments = {
    mintCustom: async (r) => {
      calls.push(r);
      return { success: true, tokenId: 'ab'.repeat(32) };
    },
    burn: async () => {
      throw new Error('not used');
    },
    tokenJustification: async () => null,
    pendingBurns: async () => [],
    acknowledgeBurn: async () => {},
  };

  const result = await mintBridgedToken(payments, req);

  assert.deepEqual(result, { success: true, tokenId: 'ab'.repeat(32) });
  assert.deepEqual(calls, [
    {
      tokenType: req.tokenType,
      salt: req.salt,
      data: req.mintData,
      justification: req.genesisReason,
      assets: [{ coinId: req.coinIdHex, amount: 5n }],
      mintJustificationVerifiers: req.mintJustificationVerifiers,
    },
  ]);
});

test('mintedAgainst: only a token whose lock names this vault is returnable here', async () => {
  const [loaded] = loadBridges(NILE_USDT_BRIDGE, deps());
  const cfg = loaded!.plugin.resolvedConfig;
  const lockedBy = (lockContractHex: string, chainId = cfg.chainId): Uint8Array =>
    new BridgeLockJustification({
      chainId,
      lockContract: Buffer.from(lockContractHex, 'hex'),
      assetContract: Buffer.from(cfg.assetContractHex, 'hex'),
      txid: new Uint8Array(32),
      logIndex: 0,
      amount: 1_000_000n,
      nonce: 1n,
    }).toCBOR();

  assert.equal(await mintedAgainst(loaded!, lockedBy(cfg.lockContractHex)), true);
  assert.equal(await mintedAgainst(loaded!, lockedBy('ab'.repeat(20))), false, 'a superseded vault');
  assert.equal(await mintedAgainst(loaded!, lockedBy(cfg.lockContractHex, TRON_NILE_CHAIN_ID + 1)), false, 'another chain');
  assert.equal(await mintedAgainst(loaded!, new Uint8Array([1, 2, 3])), false, 'not a lock justification at all');
  assert.equal(await mintedAgainst(loaded!, null), false, 'minted without a reason');
});
