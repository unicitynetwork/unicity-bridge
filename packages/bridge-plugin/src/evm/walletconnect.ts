/**
 * Ethereum wallet apps through WalletConnect: a wallet on a phone, reached by a QR code from a
 * desktop browser or by a link that opens the app from the phone's own browser.
 *
 * The app builds the provider (`EthereumProvider.init` from `@walletconnect/ethereum-provider`,
 * with its project ID and the bridge chain among `optionalChains`) and passes a function that
 * loads it. The plugin takes no WalletConnect dependency, and the app can load that package,
 * which is large, only when the user picks this wallet.
 */
import type { ContractCall } from '../contract-call.js';
import type { SourceSigner, SourceWalletProvider, WalletChange } from '../wallet/signer.js';
import { SEPOLIA_CHAIN_ID } from './config.js';
import { InjectedEvmSigner, type Eip1193Provider, type EvmSignerOptions } from './signer.js';
import { DEFAULT_WALLET_REQUEST_TIMEOUT_MS, sendContractCall, withTimeout } from './wallet-request.js';

/** The part of `@walletconnect/ethereum-provider`'s `EthereumProvider` the signer uses. */
export interface WalletConnectEvmProvider extends Eip1193Provider {
  /**
   * Opens a session when none is open, by showing the QR code or the link to the wallet app, and
   * resolves to the session's accounts. The provider refuses every other request until then.
   */
  enable(): Promise<string[]>;
  /** Ends the session. */
  disconnect(): Promise<void>;
  /** The open session, if any. A session survives a page reload: the provider restores it. */
  readonly session?: { readonly namespaces: Readonly<Record<string, WalletConnectNamespace>> };
}

export interface WalletConnectNamespace {
  /** CAIP-10 accounts, `eip155:<chain id>:<address>`: the chains the wallet approved, with the address on each. */
  readonly accounts: readonly string[];
}

/** WalletConnect's code for a request the user rejected in the wallet. EIP-1193 gives the same answer 4001. */
const WALLETCONNECT_USER_REJECTED = 5000;
const EIP1193_USER_REJECTED = 4001;

/**
 * The wallet opened a session without the bridge's chain. WalletConnect asks for every chain as
 * optional, so a wallet that hides the chain (most hide test networks by default) still connects,
 * only without it. The signer ends that session, so the next connect asks for a new one.
 */
export class WalletChainNotApprovedError extends Error {
  public constructor(public readonly chainId: number) {
    super(
      chainId === SEPOLIA_CHAIN_ID
        ? 'The wallet connected without Sepolia. Turn on test networks in the wallet, then connect again.'
        : `The wallet connected without chain ${chainId}. Add that network in the wallet, then connect again.`,
    );
    this.name = 'WalletChainNotApprovedError';
  }
}

export class WalletConnectEvmSigner implements SourceSigner {
  public constructor(
    private readonly load: () => Promise<WalletConnectEvmProvider>,
    private readonly expectedChainId: number,
    private readonly options: EvmSignerOptions = {},
  ) {}

  private readonly listeners = new Set<(e: WalletChange) => void>();
  private stopFollowing: (() => void) | undefined;

  private get timeoutMs(): number {
    return this.options.requestTimeoutMs ?? DEFAULT_WALLET_REQUEST_TIMEOUT_MS;
  }

  /** The provider, with the onChange() listeners on it, whichever call got it to load. */
  private async provider(): Promise<WalletConnectEvmProvider> {
    const provider = await this.load();
    if (this.listeners.size > 0 && !this.stopFollowing) {
      this.stopFollowing = new InjectedEvmSigner(provider).onChange((e) => this.listeners.forEach((cb) => cb(e)));
    }
    return provider;
  }

  /** Opens a session, or checks the open one, and resolves to its account on the bridge chain. */
  public async connect(): Promise<string> {
    const provider = await this.provider();
    await withTimeout(enableOnce(provider), 'eth_requestAccounts', this.timeoutMs).catch(rethrowRefusal);
    const address = sessionAddress(provider, this.expectedChainId);
    if (!address) {
      // A wallet that left the chain out of the session hides it or lacks it, so a switch request
      // would fail too, after sending the user to the wallet app for it.
      await provider.disconnect().catch(() => undefined);
      throw new WalletChainNotApprovedError(this.expectedChainId);
    }
    if ((await new InjectedEvmSigner(provider).getNetwork()) !== this.expectedChainId) {
      // The chain is in the session, so the provider switches to it without asking the wallet.
      await provider.request({ method: 'wallet_switchEthereumChain', params: [{ chainId: `0x${this.expectedChainId.toString(16)}` }] });
    }
    return address;
  }

  /**
   * The session's account on the bridge chain, whichever chain the session is on now: a session
   * can hold a different account on each chain. Transactions are sent from this account too.
   * Without a session, or without the chain in it, this connects.
   */
  public async getAddress(): Promise<string> {
    return sessionAddress(await this.provider(), this.expectedChainId) ?? this.connect();
  }

  /** The session's current chain. Without a session, this connects first. */
  public async getNetwork(): Promise<number> {
    const provider = await this.provider();
    if (!provider.session) await this.connect();
    return new InjectedEvmSigner(provider).getNetwork();
  }

  public async sendCall(call: ContractCall): Promise<string> {
    const provider = await this.provider();
    // Every transaction goes through connect(). Without a session (never opened, or ended by the
    // wallet) it opens one. With a session, including one restored after a reload, it checks the
    // session includes the bridge chain and switches to it, which asks the wallet nothing. The
    // transaction is sent from the account getAddress() reports.
    const from = await this.connect();
    return sendContractCall(provider, from, call, this.timeoutMs).catch(rethrowRefusal);
  }

  /**
   * Follows the wallet from when its provider has loaded. This starts loading it; if that load
   * fails, the next call that loads the provider attaches the listener.
   */
  public onChange(cb: (e: WalletChange) => void): () => void {
    this.listeners.add(cb);
    this.provider().catch(() => undefined);
    return () => {
      this.listeners.delete(cb);
      if (this.listeners.size === 0) {
        this.stopFollowing?.();
        this.stopFollowing = undefined;
      }
    };
  }

  public async disconnect(): Promise<void> {
    const provider = await this.provider();
    if (provider.session) await provider.disconnect();
  }
}

/**
 * The enable() in flight on each provider. A pairing outlives a timeout: its QR code or link stays
 * valid and the wallet can still approve it. So a later connect, from any signer over the same
 * provider, waits on that pairing instead of opening a second one, which could leave two sessions
 * approved. Calls that need a session at the same moment share it too.
 */
const pairings = new WeakMap<WalletConnectEvmProvider, Promise<string[]>>();

function enableOnce(provider: WalletConnectEvmProvider): Promise<string[]> {
  let pairing = pairings.get(provider);
  if (!pairing) {
    pairing = provider.enable().finally(() => pairings.delete(provider));
    pairings.set(provider, pairing);
  }
  return pairing;
}

/**
 * The WalletConnect entry for the wallet picker. `load` builds the provider and runs once, when a
 * signer first needs it; after a failed load the next use tries again. Create this entry once
 * per page: each one builds its own provider, and two providers would each open a session.
 */
export function walletConnectEvmWallet(load: () => Promise<WalletConnectEvmProvider>): SourceWalletProvider {
  let loading: Promise<WalletConnectEvmProvider> | undefined;
  const loadOnce = (): Promise<WalletConnectEvmProvider> =>
    (loading ??= load().catch((e: unknown) => {
      loading = undefined;
      throw e;
    }));
  return {
    id: 'walletconnect',
    name: 'WalletConnect',
    isAvailable: () => true,
    create: (chainId) => new WalletConnectEvmSigner(loadOnce, chainId),
  };
}

function sessionAddress(provider: WalletConnectEvmProvider, chainId: number): string | undefined {
  // A wallet may approve the chain under `eip155` or under a chain-scoped key such as
  // `eip155:11155111`. The CAIP-10 accounts name their chain either way.
  const prefix = `eip155:${chainId}:`;
  const accounts = Object.values(provider.session?.namespaces ?? {}).flatMap((namespace) => namespace.accounts);
  return accounts.find((a) => a.startsWith(prefix))?.slice(prefix.length);
}

/** A rejection in the wallet reads as the EIP-1193 refusal (4001), whichever code the wallet used. */
function rethrowRefusal(e: unknown): never {
  if ((e as { code?: unknown } | null)?.code === WALLETCONNECT_USER_REJECTED) {
    const message = (e as { message?: unknown }).message;
    throw Object.assign(new Error(typeof message === 'string' && message ? message : 'The request was rejected in the wallet.', { cause: e }), {
      code: EIP1193_USER_REJECTED,
    });
  }
  throw e;
}
