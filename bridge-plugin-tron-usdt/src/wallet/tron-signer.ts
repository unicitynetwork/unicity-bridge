/**
 * `TronSigner` (06 §A1.3, §W2) — the one new wallet capability bridge-in needs:
 * sign + broadcast a Tron `approve`/`lock`. TronLink has priority for the demo
 * (prettiest desktop UX); WalletConnect + managed-key drop in behind the same
 * interface without touching the flow. All Tron-specific, so it lives here in the
 * plugin (decision #2) — Sphere only holds a reference to a `TronSigner`.
 */
import type { TronCall } from './facade.js';

/** A wallet capable of signing + broadcasting Tron contract calls. */
export interface TronSigner {
  /** Connect (prompting the user if needed) and return the active base58 address. */
  getAddress(): Promise<string>;
  /** The Tron chainId this signer is connected to (guard against wrong-network sends). */
  getChainId(): Promise<number>;
  /**
   * Build, sign, and broadcast a contract call (the dApp-broadcast model). Resolves
   * to the broadcast txid. Throws if the user rejects or the broadcast fails.
   */
  sendCall(call: TronCall, opts?: TronSendOptions): Promise<string>;
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
  };
  address: { toHex(addr: string): string };
}

/** The browser injections TronLink exposes. */
export interface TronLinkWindow {
  tronLink?: { request(args: { method: string }): Promise<unknown> };
  tronWeb?: InjectedTronWeb & { ready?: boolean };
}

/**
 * TronLink-backed signer (priority 1, 06 §A1.3). Uses the injected `window.tronWeb`
 * for both building and broadcasting; `tron_requestAccounts` prompts the connection.
 */
export class TronLinkSigner implements TronSigner {
  public constructor(
    /** The injected window (defaults to the global one in a browser). */
    private readonly win: TronLinkWindow = globalThis as unknown as TronLinkWindow,
    /** Expected Tron chainId (from the manifest) — surfaced for the wrong-network guard. */
    private readonly expectedChainId?: number,
  ) {}

  private tronWeb(): InjectedTronWeb & { ready?: boolean } {
    const tw = this.win.tronWeb;
    if (!tw) {
      throw new Error('TronLink not found. Install the TronLink extension and reload.');
    }
    return tw;
  }

  public async getAddress(): Promise<string> {
    // Prompt the connection if TronLink is present but not yet authorized.
    if (this.win.tronLink) {
      await this.win.tronLink.request({ method: 'tron_requestAccounts' });
    }
    const addr = this.tronWeb().defaultAddress.base58;
    if (!addr) {
      throw new Error('TronLink is locked or no account is selected.');
    }
    return addr;
  }

  public async getChainId(): Promise<number> {
    // TronLink does not expose a clean chainId; the genesis-derived id is what the
    // bridge config uses. We trust the manifest's expected id and let the verifier
    // reject a lock from the wrong chain. Surface the expected id for the UI guard.
    if (this.expectedChainId === undefined) {
      throw new Error('TronLinkSigner: no expected chainId configured.');
    }
    return this.expectedChainId;
  }

  public async sendCall(call: TronCall, opts: TronSendOptions = {}): Promise<string> {
    return sendCallVia(this.tronWeb(), await this.getAddress(), call, opts);
  }
}

/**
 * Shared build → sign → broadcast over any {InjectedTronWeb} (the dApp-broadcast
 * model). TronLink prompts on `trx.sign`; a key-bearing TronWeb signs silently.
 */
export async function sendCallVia(
  tw: InjectedTronWeb,
  issuerBase58: string,
  call: TronCall,
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
  const signed = await tw.trx.sign(built.transaction);
  const receipt = await tw.trx.sendRawTransaction(signed);
  const txid = receipt.txid ?? receipt.transaction?.txID;
  if (!txid) {
    throw new Error(`Tron broadcast of ${call.functionSignature} returned no txid: ${JSON.stringify(receipt)}`);
  }
  return txid;
}

/**
 * Managed-key / WalletConnect signer (06 §W4): wraps a pre-built `TronWeb`-shaped
 * object that already holds a key (managed: `new TronWeb({ privateKey })`; the
 * proven `demo/tron.ts` path) or a WalletConnect-provided signer. No extension,
 * no flow change — same `TronSigner` surface, signs silently.
 */
export class ManagedTronSigner implements TronSigner {
  public constructor(
    private readonly tw: InjectedTronWeb,
    private readonly chainId: number,
  ) {}

  public async getAddress(): Promise<string> {
    const addr = this.tw.defaultAddress.base58;
    if (!addr) throw new Error('ManagedTronSigner: TronWeb has no default address (no key configured).');
    return addr;
  }

  public async getChainId(): Promise<number> {
    return this.chainId;
  }

  public sendCall(call: TronCall, opts?: TronSendOptions): Promise<string> {
    return this.getAddress().then((a) => sendCallVia(this.tw, a, call, opts));
  }
}
