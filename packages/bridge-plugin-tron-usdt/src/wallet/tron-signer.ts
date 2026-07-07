/**
 * `TronSigner` (06 §A1.3, §W2; reshaped 08 §1.2/§1.3) — the wallet capability
 * bridge-in needs: connect once, know the *real* network, then sign + broadcast
 * Tron `approve`/`lock`. TronLink has priority for the demo; WalletConnect +
 * managed-key drop in behind the same interface. All Tron-specific, so it lives
 * here in the plugin (decision #2) — Sphere only holds a reference to a `TronSigner`.
 *
 * Lifecycle (08 §1.2): `connect()` prompts **once** and caches the account;
 * `getAddress()` is silent afterward, so approve+lock don't re-prompt. Network
 * (08 §1.3): `getNetwork()` returns the wallet's *actual* chainId, derived from
 * its connected node's genesis block — not the manifest's expected id — so the
 * caller can refuse to sign against the wrong chain.
 */
import type { TronCall } from './facade.js';

/** A wallet capable of signing + broadcasting Tron contract calls. */
export interface TronSigner {
  /** Connect (prompting the user if needed), cache the session, return the base58 address. */
  connect(): Promise<string>;
  /** The active base58 address. Connects on first use, then answers silently. */
  getAddress(): Promise<string>;
  /**
   * The Tron chainId this wallet is *actually* connected to (genesis-derived),
   * for the wrong-network guard. Not the manifest's expected id.
   */
  getNetwork(): Promise<number>;
  /**
   * Build, sign, and broadcast a contract call (the dApp-broadcast model). Resolves
   * to the broadcast txid. Throws if the user rejects or the broadcast fails.
   */
  sendCall(call: TronCall, opts?: TronSendOptions): Promise<string>;
  /**
   * Subscribe to account/network changes (optional; TronLink supports it,
   * managed keys don't). Returns an unsubscribe fn.
   */
  onChange?(cb: (e: WalletChange) => void): () => void;
}

/** A wallet account/network change the UI can react to (abort in-flight, re-guard). */
export interface WalletChange {
  readonly kind: 'accountsChanged' | 'chainChanged' | 'disconnect';
}

export interface TronSendOptions {
  /** Energy/bandwidth fee cap in SUN (1 TRX = 1e6 SUN). Default 150 TRX. */
  readonly feeLimitSun?: number;
  /** TRX (in SUN) to send with the call. Default 0. */
  readonly callValueSun?: number;
}

const DEFAULT_FEE_LIMIT_SUN = 150_000_000; // 150 TRX — generous cap; Nile calls cost far less.

/** Minimal shape of the TronWeb instance TronLink injects on `window`. */
export interface InjectedTronWeb {
  defaultAddress: { base58: string | false };
  fullNode?: { host: string };
  transactionBuilder: {
    triggerSmartContract(
      contractAddress: string,
      functionSelector: string,
      options: Record<string, unknown>,
      parameters: readonly { type: string; value: string }[],
      issuerAddress: string,
    ): Promise<{ transaction: unknown; result?: { result?: boolean } }>;
  };
  trx: {
    sign(transaction: unknown): Promise<unknown>;
    sendRawTransaction(signed: unknown): Promise<{ result?: boolean; txid?: string; transaction?: { txID?: string } }>;
    getChainParameters?(): Promise<unknown>;
    /** Genesis-block fetch — the source of the wallet's real chainId (08 §1.3). */
    getBlockByNumber?(n: number): Promise<{ blockID?: string } | null>;
  };
  address: { toHex(addr: string): string };
}

/** The browser injections TronLink exposes. */
export interface TronLinkWindow {
  tronLink?: { request(args: { method: string }): Promise<unknown> };
  tronWeb?: InjectedTronWeb & { ready?: boolean };
  addEventListener?(type: string, listener: (e: unknown) => void): void;
  removeEventListener?(type: string, listener: (e: unknown) => void): void;
}

/**
 * Tron's chainId (as used by the bridge config, e.g. Nile `3448148188`) is the
 * low 4 bytes of the genesis block's `blockID`. Deriving it from the wallet's
 * connected node is a real network identity — not the manifest echoing itself.
 */
export function chainIdFromGenesisBlockId(blockId: string): number {
  const h = blockId.replace(/^0x/i, '');
  if (h.length < 8) throw new Error(`Malformed genesis blockID: ${blockId}`);
  return parseInt(h.slice(-8), 16);
}

/**
 * TronLink-backed signer (priority 1). Uses the injected `window.tronWeb` for
 * building + broadcasting; `tron_requestAccounts` prompts the connection once.
 */
export class TronLinkSigner implements TronSigner {
  /** True once `connect()` has prompted; `getAddress()` then reads live silently. */
  private connected = false;

  public constructor(
    /** The injected window (defaults to the global one in a browser). */
    private readonly win: TronLinkWindow = globalThis as unknown as TronLinkWindow,
    /** Expected Tron chainId (from the manifest) — for the caller's wrong-network message. */
    private readonly expectedChainId?: number,
  ) {}

  private tronWeb(): InjectedTronWeb & { ready?: boolean } {
    const tw = this.win.tronWeb;
    if (!tw) {
      throw new Error('TronLink not found. Install the TronLink extension and reload.');
    }
    return tw;
  }

  public async connect(): Promise<string> {
    // Prompt the connection once if TronLink is present but not yet authorized.
    if (this.win.tronLink) {
      await this.win.tronLink.request({ method: 'tron_requestAccounts' });
    }
    const addr = this.tronWeb().defaultAddress.base58;
    if (!addr) {
      throw new Error('TronLink is locked or no account is selected.');
    }
    this.connected = true;
    return addr;
  }

  public async getAddress(): Promise<string> {
    // Read the wallet's *current* account live (silent) — no cache, so a mid-flow
    // account switch is visible to the pre-signature guard (08 §1.4). Only
    // `connect()` ever prompts.
    const addr = this.tronWeb().defaultAddress.base58;
    if (addr) {
      this.connected = true;
      return addr;
    }
    if (!this.connected) return this.connect();
    throw new Error('TronLink is locked or no account is selected.');
  }

  public async getNetwork(): Promise<number> {
    // Derive the wallet node's *current* chainId live (no cache) — a network
    // switch changes the injected node, and the guard must see it (08 §1.4).
    const trx = this.tronWeb().trx;
    if (!trx.getBlockByNumber) {
      throw new Error('This Tron wallet cannot report its network (no genesis access); cannot verify the chain.');
    }
    const genesis = await trx.getBlockByNumber(0);
    const blockId = genesis?.blockID;
    if (!blockId) {
      throw new Error('Could not read the wallet node’s genesis block to determine the network.');
    }
    return chainIdFromGenesisBlockId(blockId);
  }

  public onChange(cb: (e: WalletChange) => void): () => void {
    const win = this.win;
    if (!win.addEventListener || !win.removeEventListener) return () => {};
    const listener = (e: unknown): void => {
      // TronLink relays wallet state via window 'message' events.
      const action = (e as { data?: { message?: { action?: string } } })?.data?.message?.action;
      if (action === 'accountsChanged' || action === 'setAccount') {
        cb({ kind: 'accountsChanged' });
      } else if (action === 'setNode' || action === 'connectWeb' || action === 'chainChanged') {
        cb({ kind: 'chainChanged' });
      } else if (action === 'disconnect' || action === 'disconnectWeb') {
        this.connected = false;
        cb({ kind: 'disconnect' });
      }
    };
    win.addEventListener('message', listener);
    return () => win.removeEventListener?.('message', listener);
  }

  /** The manifest's expected chainId, for building a clear wrong-network message. */
  public get expected(): number | undefined {
    return this.expectedChainId;
  }

  public async sendCall(call: TronCall, opts: TronSendOptions = {}): Promise<string> {
    return sendCallVia(this.tronWeb(), await this.getAddress(), call, opts);
  }
}

/** Sign an unsigned Tron transaction, resolving to the signed form. */
export type TronTxSign = (unsignedTx: unknown) => Promise<unknown>;

/**
 * Build → **sign (via `sign`)** → broadcast a call over any {InjectedTronWeb} (the
 * dApp-broadcast model). The build + broadcast use `tw`; only the signature is the
 * wallet's concern — so this serves both the injected path (`tw.trx.sign`) and
 * adapter/WalletConnect (the adapter's `signTransaction`).
 */
export async function sendCallSigned(
  tw: InjectedTronWeb,
  issuerBase58: string,
  call: TronCall,
  sign: TronTxSign,
  opts: TronSendOptions = {},
): Promise<string> {
  const built = await tw.transactionBuilder.triggerSmartContract(
    call.contractHex,
    call.functionSignature,
    { feeLimit: opts.feeLimitSun ?? DEFAULT_FEE_LIMIT_SUN, callValue: opts.callValueSun ?? 0 },
    call.parameters as { type: string; value: string }[],
    tw.address.toHex(issuerBase58),
  );
  if (built.result && built.result.result === false) {
    throw new Error(`Tron call ${call.functionSignature} could not be built (constant-call reverted).`);
  }
  const signed = await sign(built.transaction);
  const receipt = await tw.trx.sendRawTransaction(signed);
  // Reject an explicit broadcast failure (SIGERROR, DUP_TRANSACTION, …) instead
  // of returning a phantom txid the caller would then wait on forever (08 §1.4).
  if (receipt.result === false) {
    throw new Error(`Tron broadcast of ${call.functionSignature} was rejected: ${JSON.stringify(receipt)}`);
  }
  const txid = receipt.txid ?? receipt.transaction?.txID;
  if (!txid) {
    throw new Error(`Tron broadcast of ${call.functionSignature} returned no txid: ${JSON.stringify(receipt)}`);
  }
  return txid;
}

/**
 * Shared build → sign → broadcast over any {InjectedTronWeb} (the dApp-broadcast
 * model). TronLink prompts on `trx.sign`; a key-bearing TronWeb signs silently.
 */
export function sendCallVia(
  tw: InjectedTronWeb,
  issuerBase58: string,
  call: TronCall,
  opts: TronSendOptions = {},
): Promise<string> {
  return sendCallSigned(tw, issuerBase58, call, (t) => tw.trx.sign(t), opts);
}

/**
 * A wallet that connects + signs but does **not** build/broadcast (WalletConnect,
 * or any `@tronweb3/tronwallet-adapters` adapter). The app adapts the concrete
 * adapter to this shape; {AdapterTronSigner} pairs it with a node TronWeb for the
 * build/broadcast, so the plugin needs no WalletConnect/tronweb dependency.
 */
export interface AdapterWallet {
  /** Connect (e.g. open the WalletConnect modal); resolve to the base58 address. */
  connect(): Promise<string>;
  /** Sign a built Tron transaction; resolve to the signed transaction. */
  signTransaction(unsignedTx: unknown): Promise<unknown>;
  /** Optional: subscribe to account/disconnect changes. */
  onChange?(cb: (e: WalletChange) => void): () => void;
  /** Optional: tear down the session. */
  disconnect?(): Promise<void>;
}

/**
 * A {TronSigner} backed by an {AdapterWallet} (WalletConnect / adapter) for signing,
 * plus a lazily-provided node TronWeb for build + broadcast. The node builds the tx
 * against the bridge's chain, so the tx is bound to that chain regardless of what the
 * remote wallet reports — hence `getNetwork()` returns the configured chainId (as
 * {ManagedTronSigner} does), and the wrong-network guard is satisfied by construction.
 */
export class AdapterTronSigner implements TronSigner {
  private address: string | null = null;
  private tw: InjectedTronWeb | null = null;

  public constructor(
    private readonly wallet: AdapterWallet,
    /** Lazily builds the node TronWeb (keeps the heavy `tronweb` import off the hot path). */
    private readonly tronWebProvider: () => Promise<InjectedTronWeb>,
    private readonly chainId: number,
  ) {}

  public async connect(): Promise<string> {
    this.address = await this.wallet.connect();
    return this.address;
  }

  public async getAddress(): Promise<string> {
    return this.address ?? this.connect();
  }

  public async getNetwork(): Promise<number> {
    return this.chainId;
  }

  public async sendCall(call: TronCall, opts?: TronSendOptions): Promise<string> {
    const tw = (this.tw ??= await this.tronWebProvider());
    const issuer = await this.getAddress();
    return sendCallSigned(tw, issuer, call, (t) => this.wallet.signTransaction(t), opts);
  }

  public onChange(cb: (e: WalletChange) => void): () => void {
    return this.wallet.onChange?.(cb) ?? (() => {});
  }
}

/**
 * Managed-key / WalletConnect signer (06 §W4): wraps a pre-built `TronWeb`-shaped
 * object that already holds a key (managed: `new TronWeb({ privateKey })`; the
 * proven `demo/tron.ts` path) or a WalletConnect-provided signer. No extension,
 * no flow change — same `TronSigner` surface, signs silently. Its network is
 * known from construction (the node it was built against), so it's trusted.
 */
export class ManagedTronSigner implements TronSigner {
  public constructor(
    private readonly tw: InjectedTronWeb,
    private readonly chainId: number,
  ) {}

  public async connect(): Promise<string> {
    return this.getAddress();
  }

  public async getAddress(): Promise<string> {
    const addr = this.tw.defaultAddress.base58;
    if (!addr) throw new Error('ManagedTronSigner: TronWeb has no default address (no key configured).');
    return addr;
  }

  public async getNetwork(): Promise<number> {
    return this.chainId;
  }

  public sendCall(call: TronCall, opts?: TronSendOptions): Promise<string> {
    return this.getAddress().then((a) => sendCallVia(this.tw, a, call, opts));
  }
}
