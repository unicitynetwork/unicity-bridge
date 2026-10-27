import { encodeCallData, type ContractCall } from '../contract-call.js';
import type { SourceSigner, WalletChange } from '../wallet/signer.js';
import { chainIdFrom, DEFAULT_WALLET_REQUEST_TIMEOUT_MS, sendContractCall, withTimeout } from './wallet-request.js';

export { DEFAULT_WALLET_REQUEST_TIMEOUT_MS, WalletRequestTimeoutError } from './wallet-request.js';

export interface Eip1193Provider {
  request(args: { method: string; params?: unknown[] }): Promise<unknown>;
  on?(event: string, listener: (...args: unknown[]) => void): void;
  removeListener?(event: string, listener: (...args: unknown[]) => void): void;
}

export interface EvmSignerOptions {
  /**
   * How long the wallet has to connect or to answer a transaction request before the call fails
   * with `WalletRequestTimeoutError`; `DEFAULT_WALLET_REQUEST_TIMEOUT_MS` when not given.
   */
  readonly requestTimeoutMs?: number;
}

export class InjectedEvmSigner implements SourceSigner {
  public constructor(
    private readonly provider: Eip1193Provider,
    private readonly expectedChainId?: number,
    private readonly options: EvmSignerOptions = {},
  ) {}

  private get timeoutMs(): number {
    return this.options.requestTimeoutMs ?? DEFAULT_WALLET_REQUEST_TIMEOUT_MS;
  }

  public async connect(): Promise<string> {
    const request = this.provider.request({ method: 'eth_requestAccounts' });
    const accounts = (await withTimeout(request, 'eth_requestAccounts', this.timeoutMs)) as string[] | undefined;
    const address = accounts?.[0];
    if (!address) {
      throw new Error('The Ethereum wallet is locked or no account is selected.');
    }
    if (this.expectedChainId !== undefined && (await this.getNetwork()) !== this.expectedChainId) {
      await this.switchTo(this.expectedChainId);
    }
    return address;
  }

  private async switchTo(chainId: number): Promise<void> {
    try {
      await this.provider.request({ method: 'wallet_switchEthereumChain', params: [{ chainId: `0x${chainId.toString(16)}` }] });
    } catch {
      return;
    }
  }

  public async getAddress(): Promise<string> {
    const accounts = (await this.provider.request({ method: 'eth_accounts' })) as string[] | undefined;
    return accounts?.[0] ?? this.connect();
  }

  public async getNetwork(): Promise<number> {
    return chainIdFrom(await this.provider.request({ method: 'eth_chainId' }));
  }

  public onChange(cb: (e: WalletChange) => void): () => void {
    const p = this.provider;
    if (!p.on || !p.removeListener) return () => {};
    const listeners: [string, () => void][] = [
      ['accountsChanged', () => cb({ kind: 'accountsChanged' })],
      ['chainChanged', () => cb({ kind: 'chainChanged' })],
      ['disconnect', () => cb({ kind: 'disconnect' })],
    ];
    for (const [event, listener] of listeners) p.on(event, listener);
    return () => {
      for (const [event, listener] of listeners) p.removeListener?.(event, listener);
    };
  }

  public async sendCall(call: ContractCall): Promise<string> {
    return sendContractCall(this.provider, await this.getAddress(), call, this.timeoutMs);
  }
}

export interface EvmTxSender {
  getAddress(): Promise<string>;
  sendTransaction(tx: { to: string; data: string }): Promise<{ hash: string }>;
}

export class ManagedEvmSigner implements SourceSigner {
  public constructor(
    private readonly sender: EvmTxSender,
    private readonly chainId: number,
  ) {}

  public connect(): Promise<string> {
    return this.sender.getAddress();
  }

  public getAddress(): Promise<string> {
    return this.sender.getAddress();
  }

  public async getNetwork(): Promise<number> {
    return this.chainId;
  }

  public async sendCall(call: ContractCall): Promise<string> {
    const tx = await this.sender.sendTransaction({ to: `0x${call.contractHex}`, data: `0x${encodeCallData(call)}` });
    return tx.hash;
  }
}
