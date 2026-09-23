import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { test } from 'node:test';

import {
  BRIDGE_LOCK_JUSTIFICATION_TAG,
  deriveCoinId,
  deriveTokenType,
  encodeCallData,
  EvmJsonRpcClient,
  evmChainRef,
  LOCK_EVENT_TOPIC0,
  selectorHex,
  toHex,
} from '../src/index.js';
import {
  bridgePresentation,
  buildBridgeInPlan,
  InjectedEvmSigner,
  injectedEvmProvider,
  isValidEvmAddress,
  loadBridges,
  ManagedEvmSigner,
  SEPOLIA_USDC_BRIDGE,
  type Eip1193Provider,
} from '../src/wallet/index.js';

function frozen(): any {
  return JSON.parse(readFileSync(new URL('../../../deployments/sepolia/sepolia-usdc.json', import.meta.url), 'utf8'));
}

const VAULT = '9c2bf4ed5b85130ffd14be8fa65c60f299fc9a2e';
const USDC = '1c7d4b196cb0c7b01d743fbc6116a902379c7238';

test('the Sepolia USDC manifest describes the deployed vault', () => {
  const [loaded] = loadBridges(SEPOLIA_USDC_BRIDGE, { rpc: { getTransactionInfo: async () => null, getNowBlockNumber: async () => 0n } });
  const c = frozen().config;
  assert.equal('0x' + toHex(loaded.configHash), c.config_hash);
  assert.equal('0x' + loaded.plugin.tokenTypeHex, c.token_type);
  assert.equal('0x' + loaded.plugin.coinIdHex, c.coin_id);
  assert.equal(loaded.plugin.resolvedConfig.family, 'eip155');
  assert.equal(loaded.plugin.resolvedConfig.lockContractHex, VAULT);
  assert.equal(loaded.plugin.cborTag, BRIDGE_LOCK_JUSTIFICATION_TAG);
  assert.equal(evmChainRef(11155111), 'eip155:11155111');
});

test('the eip155 derivation is the family-labelled one and differs from tron for the same inputs', () => {
  assert.equal('0x' + toHex(deriveTokenType('eip155', 11155111, '0x' + USDC)), frozen().config.token_type);
  assert.equal('0x' + toHex(deriveCoinId('eip155', 11155111, '0x' + USDC)), frozen().config.coin_id);
  assert.notEqual(toHex(deriveTokenType('tron', 11155111, USDC)), toHex(deriveTokenType('eip155', 11155111, USDC)));
});

test('the JSON-RPC client shapes a receipt, the tip and a constant call', async () => {
  const calls: { method: string; params: unknown[] }[] = [];
  const fetchFn = async (_url: string, init?: { body: string }) => {
    const req = JSON.parse(init!.body);
    calls.push(req);
    const result =
      req.method === 'eth_getTransactionReceipt'
        ? { blockNumber: '0xb3', status: '0x1', logs: [{ address: '0x' + VAULT.toUpperCase(), topics: ['0x' + LOCK_EVENT_TOPIC0], data: '0xAB' }] }
        : req.method === 'eth_blockNumber'
          ? '0xff'
          : '0x' + '1'.padStart(64, '0');
    return { ok: true, status: 200, json: async () => ({ jsonrpc: '2.0', id: 1, result }) };
  };
  const rpc = new EvmJsonRpcClient({ rpcUrl: 'http://node', fetchFn });
  const info = await rpc.getTransactionInfo('ab'.repeat(32));
  assert.deepEqual(info, { blockNumber: 179n, success: true, logs: [{ address: VAULT, topics: [LOCK_EVENT_TOPIC0], data: 'ab' }] });
  assert.equal(calls[0].params[0], '0x' + 'ab'.repeat(32));
  assert.equal(await rpc.getNowBlockNumber(), 255n);
  const word = await rpc.constantCall({ ownerHex: 'aa'.repeat(20), contractHex: USDC, functionSignature: 'allowance(address,address)', parameterHex: '00'.repeat(64) });
  assert.equal(BigInt('0x' + word), 1n);
  const call = calls[2].params[0] as { from: string; to: string; data: string };
  assert.equal(call.to, '0x' + USDC);
  assert.equal(call.data, '0x' + selectorHex('allowance(address,address)') + '00'.repeat(64));
});

test('a JSON-RPC error and an unknown receipt surface as expected', async () => {
  const rpc = new EvmJsonRpcClient({
    rpcUrl: 'http://node',
    fetchFn: async (_url, init) => {
      const req = JSON.parse(init!.body);
      const body = req.method === 'eth_getTransactionReceipt' ? { result: null } : { error: { code: -32000, message: 'execution reverted' } };
      return { ok: true, status: 200, json: async () => ({ jsonrpc: '2.0', id: 1, ...body }) };
    },
  });
  assert.equal(await rpc.getTransactionInfo('00'.repeat(32)), null);
  await assert.rejects(rpc.getNowBlockNumber(), /execution reverted/);
});

test('calldata encoding: selector plus static words', () => {
  assert.equal(selectorHex('approve(address,uint256)'), '095ea7b3');
  assert.equal(selectorHex('lock(uint256,bytes32,bytes32)'), toHex(new Uint8Array(selectorHexBytes('lock(uint256,bytes32,bytes32)'))));
  const data = encodeCallData({
    contractHex: USDC,
    functionSignature: 'approve(address,uint256)',
    parameters: [
      { type: 'address', value: VAULT },
      { type: 'uint256', value: '1000000' },
    ],
  });
  assert.equal(data, '095ea7b3' + VAULT.padStart(64, '0') + (1_000_000).toString(16).padStart(64, '0'));
});

function selectorHexBytes(sig: string): number[] {
  const hex = selectorHex(sig);
  return Array.from({ length: 4 }, (_, i) => Number.parseInt(hex.slice(i * 2, i * 2 + 2), 16));
}

function fakeProvider(chainIdHex: string) {
  const requests: { method: string; params?: unknown[] }[] = [];
  let chain = chainIdHex;
  const provider: Eip1193Provider = {
    async request(args) {
      requests.push(args);
      switch (args.method) {
        case 'eth_requestAccounts':
        case 'eth_accounts':
          return ['0xAbCd000000000000000000000000000000000001'];
        case 'eth_chainId':
          return chain;
        case 'wallet_switchEthereumChain':
          chain = (args.params![0] as { chainId: string }).chainId;
          return null;
        case 'eth_sendTransaction':
          return '0x' + 'ee'.repeat(32);
        default:
          throw new Error(`unexpected ${args.method}`);
      }
    },
  };
  return { provider, requests };
}

test('the injected signer connects, switches chain, reports the network and sends encoded calls', async () => {
  const { provider, requests } = fakeProvider('0x1');
  const signer = new InjectedEvmSigner({ ethereum: provider }, 11155111);
  assert.equal(await signer.connect(), '0xAbCd000000000000000000000000000000000001');
  assert.ok(requests.some((r) => r.method === 'wallet_switchEthereumChain'), 'asks the wallet to switch to the bridge chain');
  assert.equal(await signer.getNetwork(), 11155111);

  const [bridge] = loadBridges(SEPOLIA_USDC_BRIDGE, { rpc: { getTransactionInfo: async () => null, getNowBlockNumber: async () => 0n } });
  const plan = await buildBridgeInPlan({ plugin: bridge.plugin, amount: 1_000_000n, networkId: 4, recipientPubkey: new Uint8Array(33).fill(2) });
  const txid = await signer.sendCall(plan.lock);
  assert.equal(txid, '0x' + 'ee'.repeat(32));
  const sent = requests.at(-1)!.params![0] as { from: string; to: string; data: string };
  assert.equal(sent.to, '0x' + VAULT);
  assert.equal(sent.data.slice(0, 10), '0x' + selectorHex('lock(uint256,bytes32,bytes32)'));
  assert.equal(sent.data.length, 2 + 8 + 3 * 64);
});

test('the injected provider is offered only when a wallet is injected', () => {
  assert.equal(injectedEvmProvider({}).isAvailable(), false);
  const { provider } = fakeProvider('0xaa36a7');
  const p = injectedEvmProvider({ ethereum: provider });
  assert.equal(p.isAvailable(), true);
  assert.equal(p.id, 'injected-evm');
  assert.ok(p.create(11155111) instanceof InjectedEvmSigner);
});

test('the managed signer signs through a key-holding sender and knows its chain', async () => {
  const sent: { to: string; data: string }[] = [];
  const signer = new ManagedEvmSigner(
    { getAddress: async () => '0x' + '11'.repeat(20), sendTransaction: async (tx) => (sent.push(tx), { hash: '0x' + 'cc'.repeat(32) }) },
    11155111,
  );
  assert.equal(await signer.getNetwork(), 11155111);
  assert.equal(await signer.connect(), '0x' + '11'.repeat(20));
  const txid = await signer.sendCall({ contractHex: USDC, functionSignature: 'approve(address,uint256)', parameters: [{ type: 'address', value: VAULT }, { type: 'uint256', value: '5' }] });
  assert.equal(txid, '0x' + 'cc'.repeat(32));
  assert.equal(sent[0].to, '0x' + USDC);
  assert.equal(sent[0].data, '0x095ea7b3' + VAULT.padStart(64, '0') + '5'.padStart(64, '0'));
});

test('presentation: Etherscan links and 0x addresses', () => {
  const [bridge] = loadBridges(SEPOLIA_USDC_BRIDGE, { rpc: { getTransactionInfo: async () => null, getNowBlockNumber: async () => 0n } });
  const p = bridgePresentation(bridge);
  assert.equal(p.explorerTxUrl('ab'.repeat(32)), 'https://sepolia.etherscan.io/tx/0x' + 'ab'.repeat(32));
  assert.equal(p.explorerTxUrl('0x' + 'ab'.repeat(32)), 'https://sepolia.etherscan.io/tx/0x' + 'ab'.repeat(32));
  assert.equal(p.validateAddress('0x' + VAULT), true);
  assert.equal(isValidEvmAddress('T' + 'a'.repeat(33)), false);
  assert.equal(isValidEvmAddress('0x' + 'g'.repeat(40)), false);
});
