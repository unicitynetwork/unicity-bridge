import assert from 'node:assert/strict';
import { test } from 'node:test';

import { MintJustificationVerifierService } from '@unicitylabs/state-transition-sdk/lib/transaction/verification/MintJustificationVerifierService.js';
import { VerificationStatus } from '@unicitylabs/state-transition-sdk/lib/verification/VerificationStatus.js';

import { BRIDGE_LOCK_JUSTIFICATION_TAG, BridgeMintJustificationVerifier } from '../src/index.js';
import { bridgeTokenPlugin, loadBridges, mergeBridgeTokenPlugins, NILE_USDT_BRIDGE, SEPOLIA_USDC_BRIDGE } from '../src/wallet/index.js';
import { buildScenario, MockTronRpc } from './helpers.js';

const noNestedTokens = (): void => {};
const idle = () => ({ rpc: new MockTronRpc(null, 0n) });

test('two bridges register once: the merged plugin carries one verifier for the shared tag', () => {
  const [nile] = loadBridges(NILE_USDT_BRIDGE, idle());
  const [sepolia] = loadBridges(SEPOLIA_USDC_BRIDGE, idle());
  const separate = [bridgeTokenPlugin(nile), bridgeTokenPlugin(sepolia)];
  assert.throws(() => {
    const service = new MintJustificationVerifierService();
    for (const p of separate) for (const v of p.mintJustificationVerifiers) service.register(v);
  }, /Duplicate mint justification verifier for tag 1330002/);

  const merged = mergeBridgeTokenPlugins(separate);
  assert.equal(merged.id, 'bridge');
  assert.equal(merged.mintJustificationVerifiers.length, 1);
  assert.equal(merged.mintJustificationVerifiers[0].tag, BRIDGE_LOCK_JUSTIFICATION_TAG);
  const service = new MintJustificationVerifierService();
  service.register(merged.mintJustificationVerifiers[0]);
});

test('the merged verifier hands a mint to the bridge that owns its chain and vault', async () => {
  const scenario = await buildScenario();
  const [sepolia] = loadBridges(SEPOLIA_USDC_BRIDGE, idle());
  const verifier = new BridgeMintJustificationVerifier([sepolia.plugin.verifier, scenario.plugin.verifier]);
  const result = await verifier.verify(scenario.certifiedTx, noNestedTokens);
  assert.equal(result.status, VerificationStatus.OK, result.message);
});

test('a mint whose lock names a chain no bridge serves is refused, naming the chain', async () => {
  const scenario = await buildScenario({ justification: (d) => ({ ...d, chainId: 999 }) });
  const verifier = new BridgeMintJustificationVerifier([scenario.plugin.verifier]);
  const result = await verifier.verify(scenario.certifiedTx, noNestedTokens);
  assert.equal(result.status, VerificationStatus.FAIL);
  assert.match(result.message ?? '', /No bridge verifies chain 999/);
});
