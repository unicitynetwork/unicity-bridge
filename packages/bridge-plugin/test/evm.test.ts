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
  evmWallets,
  InjectedEvmSigner,
  injectedEvmProvider,
  isValidEvmAddress,
  loadBridges,
  ManagedEvmSigner,
  SEPOLIA_USDC_BRIDGE,
  type Eip1193Provider,
  type Eip6963ProviderInfo,
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

test('the JSON-RPC client reads logs by address, topics and block range, shaped like receipt logs', async () => {
  const calls: { method: string; params: unknown[] }[] = [];
  const fetchFn = async (_url: string, init?: { body: string }) => {
    const req = JSON.parse(init!.body);
    calls.push(req);
    const result = [{ address: '0x' + VAULT.toUpperCase(), topics: ['0x' + LOCK_EVENT_TOPIC0], data: '0xAB', blockNumber: '0xb3', transactionHash: '0x' + 'CD'.repeat(32) }];
    return { ok: true, status: 200, json: async () => ({ jsonrpc: '2.0', id: 1, result }) };
  };
  const rpc = new EvmJsonRpcClient({ rpcUrl: 'http://node', fetchFn });
  const logs = await rpc.getLogs({ address: VAULT, topics: [LOCK_EVENT_TOPIC0, null, 'aa'.repeat(20).padStart(64, '0')], fromBlock: 100n, toBlock: 255n });
  assert.deepEqual(logs, [{ address: VAULT, topics: [LOCK_EVENT_TOPIC0], data: 'ab', blockNumber: 179n, transactionHash: 'cd'.repeat(32) }]);
  assert.equal(calls[0].method, 'eth_getLogs');
  assert.deepEqual(calls[0].params[0], {
    address: '0x' + VAULT,
    topics: ['0x' + LOCK_EVENT_TOPIC0, null, '0x' + 'aa'.repeat(20).padStart(64, '0')],
    fromBlock: '0x64',
    toBlock: '0xff',
  });
});

test('a JSON-RPC error and an unknown receipt surface as expected, the error with its code', async () => {
  const rpc = new EvmJsonRpcClient({
    rpcUrl: 'http://node',
    fetchFn: async (_url, init) => {
      const req = JSON.parse(init!.body);
      const body = req.method === 'eth_getTransactionReceipt' ? { result: null } : { error: { code: -32000, message: 'execution reverted' } };
      return { ok: true, status: 200, json: async () => ({ jsonrpc: '2.0', id: 1, ...body }) };
    },
  });
  assert.equal(await rpc.getTransactionInfo('00'.repeat(32)), null);
  await assert.rejects(rpc.getNowBlockNumber(), /\[-32000\] execution reverted/);
});

test('the JSON-RPC client reads an account transaction count at a block tag', async () => {
  const calls: { method: string; params: unknown[] }[] = [];
  const rpc = new EvmJsonRpcClient({
    rpcUrl: 'http://node',
    fetchFn: async (_url, init) => {
      calls.push(JSON.parse(init!.body));
      return { ok: true, status: 200, json: async () => ({ jsonrpc: '2.0', id: 1, result: '0x2a' }) };
    },
  });
  assert.equal(await rpc.getTransactionCount('aa'.repeat(20), 'pending'), 42n);
  assert.deepEqual(calls[0], { jsonrpc: '2.0', id: 1, method: 'eth_getTransactionCount', params: ['0x' + 'aa'.repeat(20), 'pending'] });
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
  const signer = new InjectedEvmSigner(provider, 11155111);
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

test('the browser wallet is offered only when a wallet is injected', () => {
  assert.equal(injectedEvmProvider({}).isAvailable(), false);
  assert.throws(() => injectedEvmProvider({}).create(11155111), /No Ethereum wallet found/);
  const { provider } = fakeProvider('0xaa36a7');
  const p = injectedEvmProvider({ ethereum: provider });
  assert.equal(p.isAvailable(), true);
  assert.equal(p.id, 'injected-evm');
  assert.equal(p.name, 'Browser wallet');
  assert.ok(p.create(11155111) instanceof InjectedEvmSigner);
});

function fakeWindow(ethereum?: Eip1193Provider) {
  return Object.assign(new EventTarget(), { ethereum });
}

function walletInfo(rdns: string, uuid = rdns): Eip6963ProviderInfo {
  return { uuid, name: rdns.split('.').at(-1)!, icon: `data:image/svg+xml,${rdns}`, rdns };
}

function announce(win: EventTarget, info: Eip6963ProviderInfo, provider: Eip1193Provider): void {
  win.dispatchEvent(new CustomEvent('eip6963:announceProvider', { detail: { info, provider } }));
}

test('wallets announced through eip-6963 are listed by rdns with name and icon, and a later announcement cannot replace a listed wallet', async () => {
  const win = fakeWindow();
  const wallets = evmWallets(win);
  const rabby = fakeProvider('0xaa36a7');
  const metamask = fakeProvider('0x1');
  announce(win, walletInfo('io.rabby', 'rabby-first-uuid'), rabby.provider);
  announce(win, walletInfo('io.rabby', 'rabby-second-uuid'), fakeProvider('0x5').provider);
  announce(win, walletInfo('io.metamask'), metamask.provider);

  const listed = wallets.list();
  assert.deepEqual(listed.map((w) => [w.id, w.name, w.icon, w.isAvailable()]), [
    ['io.rabby', 'rabby', 'data:image/svg+xml,io.rabby', true],
    ['io.metamask', 'metamask', 'data:image/svg+xml,io.metamask', true],
  ]);
  assert.equal(await listed[1].create(11155111).getNetwork(), 1);
  assert.deepEqual(metamask.requests.map((r) => r.method), ['eth_chainId']);
  assert.equal(await listed[0].create(11155111).getNetwork(), 11155111);
  assert.deepEqual(rabby.requests.map((r) => r.method), ['eth_chainId']);
});

test('a wallet already on the page answers the request for providers', () => {
  const win = fakeWindow();
  const { provider } = fakeProvider('0xaa36a7');
  win.addEventListener('eip6963:requestProvider', () => announce(win, walletInfo('io.metamask'), provider));
  assert.deepEqual(evmWallets(win).list().map((w) => w.id), ['io.metamask']);
});

test('a wallet that announces itself later is listed from then on', () => {
  const win = fakeWindow();
  const wallets = evmWallets(win);
  assert.deepEqual(wallets.list().map((w) => w.id), ['injected-evm']);
  announce(win, walletInfo('io.metamask'), fakeProvider('0x1').provider);
  assert.deepEqual(wallets.list().map((w) => w.id), ['io.metamask']);
});

test('the browser wallet is listed only when nothing announced itself', () => {
  const nothing = evmWallets(fakeWindow()).list();
  assert.deepEqual(nothing.map((w) => [w.id, w.isAvailable()]), [['injected-evm', false]]);

  const { provider: legacy } = fakeProvider('0x1');
  const only = evmWallets(fakeWindow(legacy)).list();
  assert.deepEqual(only.map((w) => [w.id, w.isAvailable()]), [['injected-evm', true]]);

  const other = fakeWindow(legacy);
  const otherWallets = evmWallets(other);
  announce(other, walletInfo('io.rabby'), fakeProvider('0x1').provider);
  assert.deepEqual(otherWallets.list().map((w) => w.id), ['io.rabby']);
});

test('an announcement without a usable provider, name or rdns is ignored', () => {
  const win = fakeWindow();
  const wallets = evmWallets(win);
  const { provider } = fakeProvider('0x1');
  win.dispatchEvent(new CustomEvent('eip6963:announceProvider', { detail: null }));
  win.dispatchEvent(new CustomEvent('eip6963:announceProvider', { detail: { info: walletInfo('io.noprovider') } }));
  win.dispatchEvent(new CustomEvent('eip6963:announceProvider', { detail: { info: { ...walletInfo('io.badrdns'), rdns: '' }, provider } }));
  win.dispatchEvent(new CustomEvent('eip6963:announceProvider', { detail: { info: { ...walletInfo('io.noname'), name: '' }, provider } }));
  win.dispatchEvent(new CustomEvent('eip6963:announceProvider', { detail: { info: walletInfo('io.notafunction'), provider: { request: 'nope' } } }));
  assert.deepEqual(wallets.list().map((w) => w.id), ['injected-evm']);
});

test('an icon is kept only when it is an inline data image, as the standard requires', () => {
  const win = fakeWindow();
  const wallets = evmWallets(win);
  const { provider } = fakeProvider('0x1');
  announce(win, { ...walletInfo('io.inline'), icon: 'data:image/svg+xml;base64,PHN2Zy8+' }, provider);
  announce(win, { ...walletInfo('io.remote'), icon: 'https://evil.example/icon.svg' }, provider);
  announce(win, { ...walletInfo('io.none'), icon: '' }, provider);
  assert.deepEqual(wallets.list().map((w) => [w.id, w.icon]), [
    ['io.inline', 'data:image/svg+xml;base64,PHN2Zy8+'],
    ['io.remote', undefined],
    ['io.none', undefined],
  ]);
});

test('where nothing can announce itself, as under node, only the browser wallet is considered', () => {
  const wallets = evmWallets({} as never);
  assert.deepEqual(wallets.list().map((w) => [w.id, w.isAvailable()]), [['injected-evm', false]]);
  const { provider } = fakeProvider('0x1');
  assert.deepEqual(evmWallets({ ethereum: provider } as never).list().map((w) => [w.id, w.isAvailable()]), [['injected-evm', true]]);
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
  assert.equal(p.explorerAddressUrl(VAULT), 'https://sepolia.etherscan.io/address/0x' + VAULT);
  assert.equal(p.explorerAddressUrl('0x' + VAULT), 'https://sepolia.etherscan.io/address/0x' + VAULT);
  assert.equal(p.validateAddress('0x' + VAULT), true);
  assert.equal(isValidEvmAddress('T' + 'a'.repeat(33)), false);
  assert.equal(isValidEvmAddress('0x' + 'g'.repeat(40)), false);
});
