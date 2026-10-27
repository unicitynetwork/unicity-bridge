import assert from 'node:assert/strict';
import { test } from 'node:test';

import { selectorHex } from '../src/index.js';
import {
  WalletChainNotApprovedError,
  walletConnectEvmWallet,
  WalletConnectEvmSigner,
  WalletRequestTimeoutError,
  type WalletConnectEvmProvider,
} from '../src/wallet/index.js';

const SEPOLIA = 11155111;
const ADDRESS = '0xAbCd000000000000000000000000000000000001';
const USDC = '1c7d4b196cb0c7b01d743fbc6116a902379c7238';
const VAULT = '9c2bf4ed5b85130ffd14be8fa65c60f299fc9a2e';
const APPROVE = {
  contractHex: USDC,
  functionSignature: 'approve(address,uint256)',
  parameters: [
    { type: 'address' as const, value: VAULT },
    { type: 'uint256' as const, value: '5' },
  ],
};

/**
 * Behaves as `@walletconnect/ethereum-provider` was seen to against a live relay: it refuses
 * requests before a session, answers `eth_chainId` with a number, switches between the session's
 * chains on its own and sends a switch to any other chain to the wallet.
 */
function fakeWalletConnect(opts: {
  chains: number[];
  defaultChain: number;
  /** Approve each chain under its own key (`eip155:<id>`) instead of under `eip155`. */
  chainScoped?: boolean;
  /** Start with the session already open, as the provider restores it after a page reload. */
  restored?: boolean;
  /** The accounts the wallet approves on each chain, in session order; ADDRESS alone unless given. */
  accountsOn?: (chainId: number) => string[];
  /** What `eth_accounts` answers on each chain, when it should differ from the session order. */
  ethAccounts?: (chainId: number) => string[];
  send?: (tx: unknown) => Promise<unknown>;
  enable?: () => Promise<string[]>;
}) {
  const calls: string[] = [];
  const toWallet: string[] = [];
  const listeners = new Map<string, Set<(...args: unknown[]) => void>>();
  let session: WalletConnectEvmProvider['session'];
  let chain = opts.defaultChain;
  const approved = opts.accountsOn ?? (() => [ADDRESS]);
  const accountsOn = (id: number) => (opts.chains.includes(id) ? (opts.ethAccounts ?? approved)(id) : []);
  const caip10 = (id: number) => approved(id).map((a) => `eip155:${id}:${a}`);
  const open = () =>
    (session ??= {
      namespaces: opts.chainScoped
        ? Object.fromEntries(opts.chains.map((id) => [`eip155:${id}`, { accounts: caip10(id) }]))
        : { eip155: { accounts: opts.chains.flatMap(caip10) } },
    });
  if (opts.restored) open();
  const provider: WalletConnectEvmProvider = {
    get session() {
      return session;
    },
    async enable() {
      calls.push('enable');
      if (opts.enable) return opts.enable();
      open();
      return accountsOn(chain);
    },
    async disconnect() {
      calls.push('disconnect');
      session = undefined;
    },
    async request({ method, params }) {
      calls.push(method);
      if (!session) throw new Error('Please call connect() before request()');
      switch (method) {
        case 'eth_chainId':
          return chain;
        case 'eth_accounts':
        case 'eth_requestAccounts':
          return accountsOn(chain);
        case 'wallet_switchEthereumChain': {
          const id = Number.parseInt((params![0] as { chainId: string }).chainId, 16);
          if (opts.chains.includes(id)) {
            chain = id;
            return null;
          }
          toWallet.push(method);
          throw Object.assign(new Error('Unrecognized chain ID'), { code: 4902 });
        }
        case 'eth_sendTransaction':
          toWallet.push(`${method} on ${chain}`);
          return opts.send ? opts.send(params![0]) : '0x' + 'ee'.repeat(32);
        default:
          throw new Error(`unexpected ${method}`);
      }
    },
    on(event, listener) {
      listeners.set(event, (listeners.get(event) ?? new Set()).add(listener));
    },
    removeListener(event, listener) {
      listeners.get(event)?.delete(listener);
    },
  };
  const emit = (event: string) => listeners.get(event)?.forEach((l) => l());
  return { provider, calls, toWallet, emit, listeners };
}

test('a WalletConnect wallet connects through enable, takes its address on the bridge chain and switches to it without asking the wallet', async () => {
  const wc = fakeWalletConnect({ chains: [1, SEPOLIA], defaultChain: 1 });
  const signer = new WalletConnectEvmSigner(async () => wc.provider, SEPOLIA);
  assert.equal(await signer.connect(), ADDRESS);
  assert.equal(wc.calls[0], 'enable', 'the session is opened before any request');
  assert.equal(await signer.getNetwork(), SEPOLIA, 'the numeric chain id is read as is');
  assert.equal(await signer.getAddress(), ADDRESS);
  assert.deepEqual(wc.toWallet, [], 'switching between the session chains never reaches the wallet');
});

test('a session that approves the bridge chain under a chain-scoped namespace key is accepted', async () => {
  const wc = fakeWalletConnect({ chains: [1, SEPOLIA], defaultChain: 1, chainScoped: true });
  const signer = new WalletConnectEvmSigner(async () => wc.provider, SEPOLIA);
  assert.equal(wc.provider.session, undefined);
  assert.equal(await signer.connect(), ADDRESS);
  assert.deepEqual(Object.keys(wc.provider.session!.namespaces), ['eip155:1', `eip155:${SEPOLIA}`]);
  assert.equal(await signer.getNetwork(), SEPOLIA);
  assert.ok(!wc.calls.includes('disconnect'));

  const other = fakeWalletConnect({ chains: [1], defaultChain: 1, chainScoped: true });
  await assert.rejects(new WalletConnectEvmSigner(async () => other.provider, SEPOLIA).connect(), WalletChainNotApprovedError);
});

test('a session without the bridge chain is ended, nothing is asked of the wallet, and the error says what to do', async () => {
  const wc = fakeWalletConnect({ chains: [1], defaultChain: 1 });
  const signer = new WalletConnectEvmSigner(async () => wc.provider, SEPOLIA);
  await assert.rejects(signer.connect(), (e: unknown) => {
    assert.ok(e instanceof WalletChainNotApprovedError);
    assert.equal(e.chainId, SEPOLIA);
    assert.match(e.message, /without Sepolia\. Turn on test networks in the wallet, then connect again/);
    return true;
  });
  assert.equal(wc.provider.session, undefined, 'the next connect asks for a new session');
  assert.ok(wc.calls.includes('disconnect'));
  assert.deepEqual(wc.toWallet, []);
  assert.match(new WalletChainNotApprovedError(1).message, /without chain 1\. Add that network in the wallet/);
});

test('a transaction goes out encoded over the session, and a rejection in the wallet reads as 4001 whichever code the wallet used', async () => {
  const answers: (() => Promise<unknown>)[] = [];
  const sent: { from: string; to: string; data: string }[] = [];
  const wc = fakeWalletConnect({
    chains: [SEPOLIA],
    defaultChain: SEPOLIA,
    send: (tx) => (sent.push(tx as (typeof sent)[number]), answers.shift()!()),
  });
  const signer = new WalletConnectEvmSigner(async () => wc.provider, SEPOLIA);
  await signer.connect();

  answers.push(async () => '0x' + 'ab'.repeat(32));
  assert.equal(await signer.sendCall(APPROVE), '0x' + 'ab'.repeat(32));
  assert.equal(sent[0].from, ADDRESS);
  assert.equal(sent[0].to, '0x' + USDC);
  assert.equal(sent[0].data.slice(0, 10), '0x' + selectorHex('approve(address,uint256)'));

  const walletConnectRejection = Object.assign(new Error('User rejected.'), { code: 5000 });
  answers.push(async () => Promise.reject(walletConnectRejection));
  await assert.rejects(signer.sendCall(APPROVE), (e: Error & { code?: unknown }) => {
    assert.equal(e.code, 4001);
    assert.equal(e.message, 'User rejected.');
    assert.equal(e.cause, walletConnectRejection);
    return true;
  });

  answers.push(async () => Promise.reject(Object.assign(new Error('User rejected the request.'), { code: 4001 })));
  await assert.rejects(signer.sendCall(APPROVE), { code: 4001, message: 'User rejected the request.' });

  const failure = Object.assign(new Error('insufficient funds for gas'), { code: -32000 });
  answers.push(async () => Promise.reject(failure));
  await assert.rejects(signer.sendCall(APPROVE), (e) => e === failure);
});

test('a WalletConnect wallet that does not answer times out without a refusal code', async () => {
  const silent = fakeWalletConnect({ chains: [SEPOLIA], defaultChain: SEPOLIA, enable: () => new Promise(() => {}) });
  const connecting = new WalletConnectEvmSigner(async () => silent.provider, SEPOLIA, { requestTimeoutMs: 20 });
  await assert.rejects(connecting.connect(), (e: unknown) => {
    assert.ok(e instanceof WalletRequestTimeoutError);
    assert.equal(e.method, 'eth_requestAccounts');
    assert.equal((e as { code?: unknown }).code, undefined);
    assert.match(e.message, /^The wallet did not connect within 1 second\.$/);
    return true;
  });

  const wc = fakeWalletConnect({ chains: [SEPOLIA], defaultChain: SEPOLIA, send: () => new Promise(() => {}) });
  const sending = new WalletConnectEvmSigner(async () => wc.provider, SEPOLIA, { requestTimeoutMs: 20 });
  await sending.connect();
  await assert.rejects(sending.sendCall(APPROVE), (e: unknown) => {
    assert.ok(e instanceof WalletRequestTimeoutError);
    assert.equal(e.method, 'eth_sendTransaction');
    assert.equal((e as { code?: unknown }).code, undefined);
    return true;
  });
});

test('the picker entry loads the provider once, when a signer first needs it, and tries again after a failed load', async () => {
  const wc = fakeWalletConnect({ chains: [SEPOLIA], defaultChain: SEPOLIA });
  let loads = 0;
  const wallet = walletConnectEvmWallet(async () => {
    loads++;
    if (loads === 1) throw new Error('the WalletConnect chunk did not load');
    return wc.provider;
  });
  assert.deepEqual([wallet.id, wallet.name, wallet.isAvailable()], ['walletconnect', 'WalletConnect', true]);
  const first = wallet.create(SEPOLIA);
  const second = wallet.create(SEPOLIA);
  assert.equal(loads, 0, 'listing the wallet and creating signers loads nothing');
  await assert.rejects(first.connect(), /chunk did not load/);
  assert.equal(await second.connect(), ADDRESS);
  assert.equal(await first.getNetwork(), SEPOLIA);
  assert.equal(loads, 2);
});

test('a transaction without a session, as after the wallet ended it, opens a new one first', async () => {
  const wc = fakeWalletConnect({ chains: [SEPOLIA], defaultChain: SEPOLIA });
  const signer = new WalletConnectEvmSigner(async () => wc.provider, SEPOLIA);
  await signer.connect();
  await wc.provider.disconnect();
  wc.calls.length = 0;
  assert.equal(await signer.sendCall(APPROVE), '0x' + 'ee'.repeat(32));
  assert.equal(wc.calls[0], 'enable');
  assert.ok(wc.calls.indexOf('enable') < wc.calls.indexOf('eth_sendTransaction'));
});

test('a transaction over a restored session checks it includes the bridge chain and switches to it first', async () => {
  const onSepolia = fakeWalletConnect({ chains: [1, SEPOLIA], defaultChain: 1, restored: true });
  const signer = new WalletConnectEvmSigner(async () => onSepolia.provider, SEPOLIA);
  assert.equal(await signer.sendCall(APPROVE), '0x' + 'ee'.repeat(32));
  assert.deepEqual(onSepolia.toWallet, [`eth_sendTransaction on ${SEPOLIA}`]);

  const mainnetOnly = fakeWalletConnect({ chains: [1], defaultChain: 1, restored: true });
  await assert.rejects(new WalletConnectEvmSigner(async () => mainnetOnly.provider, SEPOLIA).sendCall(APPROVE), WalletChainNotApprovedError);
  assert.deepEqual(mainnetOnly.toWallet, [], 'nothing is sent on the wrong chain');
  assert.equal(mainnetOnly.provider.session, undefined);
});

test('the address is the session account on the bridge chain, even while the session is on another chain with another account', async () => {
  const MAINNET_ADDRESS = '0x' + '22'.repeat(20);
  const sent: { from: string }[] = [];
  const wc = fakeWalletConnect({
    chains: [1, SEPOLIA],
    defaultChain: 1,
    restored: true,
    accountsOn: (id) => [id === 1 ? MAINNET_ADDRESS : ADDRESS],
    send: async (tx) => (sent.push(tx as { from: string }), '0x' + 'ee'.repeat(32)),
  });
  const signer = new WalletConnectEvmSigner(async () => wc.provider, SEPOLIA);
  assert.equal(await signer.getAddress(), ADDRESS);
  await signer.sendCall(APPROVE);
  assert.equal(sent[0].from, ADDRESS);
  assert.equal(await signer.connect(), ADDRESS);

  const mainnetOnly = fakeWalletConnect({ chains: [1], defaultChain: 1, restored: true });
  await assert.rejects(new WalletConnectEvmSigner(async () => mainnetOnly.provider, SEPOLIA).getAddress(), WalletChainNotApprovedError);
});

test('the account reported and the account a transaction is sent from are the same, even when eth_accounts orders them otherwise', async () => {
  const SECOND = '0x' + '33'.repeat(20);
  const sent: { from: string }[] = [];
  const wc = fakeWalletConnect({
    chains: [SEPOLIA],
    defaultChain: SEPOLIA,
    accountsOn: () => [ADDRESS, SECOND],
    ethAccounts: () => [SECOND, ADDRESS],
    send: async (tx) => (sent.push(tx as { from: string }), '0x' + 'ee'.repeat(32)),
  });
  const signer = new WalletConnectEvmSigner(async () => wc.provider, SEPOLIA);
  assert.equal(await signer.connect(), ADDRESS);
  assert.equal(await signer.getAddress(), ADDRESS);
  await signer.sendCall(APPROVE);
  assert.equal(sent[0].from, ADDRESS);
});

test('calls that need a session at the same time open one pairing, and getNetwork connects too', async () => {
  const wc = fakeWalletConnect({ chains: [SEPOLIA], defaultChain: SEPOLIA });
  const signer = new WalletConnectEvmSigner(async () => wc.provider, SEPOLIA);
  assert.deepEqual(await Promise.all([signer.getAddress(), signer.getNetwork()]), [ADDRESS, SEPOLIA]);
  assert.equal(wc.calls.filter((c) => c === 'enable').length, 1);

  const fresh = fakeWalletConnect({ chains: [SEPOLIA], defaultChain: SEPOLIA });
  assert.equal(await new WalletConnectEvmSigner(async () => fresh.provider, SEPOLIA).getNetwork(), SEPOLIA);
  assert.equal(fresh.calls[0], 'enable');
});

test('a connect after a timeout waits on the pairing still open, from any signer, instead of opening another', async () => {
  let approve!: () => void;
  let pairingsOpened = 0;
  const wc = fakeWalletConnect({ chains: [SEPOLIA], defaultChain: SEPOLIA });
  const enable = wc.provider.enable.bind(wc.provider);
  wc.provider.enable = () => {
    pairingsOpened++;
    return new Promise<void>((r) => (approve = r)).then(enable);
  };
  const first = new WalletConnectEvmSigner(async () => wc.provider, SEPOLIA, { requestTimeoutMs: 20 });
  await assert.rejects(first.connect(), WalletRequestTimeoutError);
  const second = new WalletConnectEvmSigner(async () => wc.provider, SEPOLIA, { requestTimeoutMs: 1_000 });
  const retried = second.connect();
  approve();
  assert.equal(await retried, ADDRESS);
  assert.equal(pairingsOpened, 1);
});

test('getAddress before a session connects first, and disconnect ends the session once', async () => {
  const wc = fakeWalletConnect({ chains: [SEPOLIA], defaultChain: SEPOLIA });
  const signer = new WalletConnectEvmSigner(async () => wc.provider, SEPOLIA);
  assert.equal(await signer.getAddress(), ADDRESS);
  assert.equal(wc.calls[0], 'enable');
  await signer.disconnect();
  await signer.disconnect();
  assert.equal(wc.calls.filter((c) => c === 'disconnect').length, 1);
  assert.equal(wc.provider.session, undefined);
});

test('onChange follows the wallet once the provider has loaded, and nothing is attached after an early unsubscribe', async () => {
  const wc = fakeWalletConnect({ chains: [SEPOLIA], defaultChain: SEPOLIA });
  const seen: string[] = [];
  const signer = new WalletConnectEvmSigner(async () => wc.provider, SEPOLIA);
  const stop = signer.onChange((e) => seen.push(e.kind));
  await new Promise((r) => setImmediate(r));
  wc.emit('accountsChanged');
  wc.emit('disconnect');
  assert.deepEqual(seen, ['accountsChanged', 'disconnect']);
  stop();
  wc.emit('chainChanged');
  assert.deepEqual(seen, ['accountsChanged', 'disconnect']);

  let release!: (p: WalletConnectEvmProvider) => void;
  const late = fakeWalletConnect({ chains: [SEPOLIA], defaultChain: SEPOLIA });
  const early = new WalletConnectEvmSigner(() => new Promise((r) => (release = r)), SEPOLIA).onChange(() => seen.push('late'));
  early();
  release(late.provider);
  await new Promise((r) => setImmediate(r));
  assert.equal([...late.listeners.values()].reduce((n, s) => n + s.size, 0), 0);
});

test('a listener added while the provider failed to load is attached when a later call loads it', async () => {
  const wc = fakeWalletConnect({ chains: [SEPOLIA], defaultChain: SEPOLIA });
  let loads = 0;
  const wallet = walletConnectEvmWallet(async () => {
    if (++loads === 1) throw new Error('the WalletConnect chunk did not load');
    return wc.provider;
  });
  const signer = wallet.create(SEPOLIA);
  const seen: string[] = [];
  const stop = signer.onChange!((e) => seen.push(e.kind));
  await new Promise((r) => setImmediate(r));
  assert.equal(loads, 1);
  await signer.connect();
  wc.emit('accountsChanged');
  assert.deepEqual(seen, ['accountsChanged']);
  await signer.connect();
  wc.emit('chainChanged');
  assert.deepEqual(seen, ['accountsChanged', 'chainChanged'], 'attached once, however many calls load the provider');
  stop();
  assert.equal([...wc.listeners.values()].reduce((n, s) => n + s.size, 0), 0);
});
