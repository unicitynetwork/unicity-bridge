/** One event log from a Tron transaction (hex fields, lowercase, no `0x`). */
export interface TronLog {
  /** 20-byte EVM-form contract address that emitted the log. */
  readonly address: string;
  /** Indexed topics (topic0 = event signature hash). */
  readonly topics: string[];
  /** ABI-encoded non-indexed args. */
  readonly data: string;
}

/** Normalized transaction info the verifier needs from a Tron node. */
export interface TronTxInfo {
  readonly blockNumber: bigint;
  readonly success: boolean;
  readonly logs: TronLog[];
}

/**
 * The minimal Tron node surface the verifier depends on. Implemented by
 * {@link TronHttpRpcClient} over the public HTTP API; mockable in tests/CLI.
 */
export interface TronRpc {
  /** Returns null when the transaction is unknown to the node. */
  getTransactionInfo(txidHex: string): Promise<TronTxInfo | null>;
  /** Current chain tip block number. */
  getNowBlockNumber(): Promise<bigint>;
}

type FetchLike = (input: string, init?: { method: string; headers: Record<string, string>; body: string }) => Promise<{
  ok: boolean;
  status: number;
  json: () => Promise<unknown>;
}>;

export interface TronHttpRpcClientOptions {
  /** Base URL, e.g. https://api.trongrid.io or https://nile.trongrid.io */
  readonly baseUrl: string;
  /** TronGrid API key (optional; sent as TRON-PRO-API-KEY). */
  readonly apiKey?: string;
  /** Injectable fetch (defaults to globalThis.fetch). */
  readonly fetchFn?: FetchLike;
}

function strip0x(h: string): string {
  return h.startsWith('0x') || h.startsWith('0X') ? h.slice(2) : h;
}

/** Tron full-node HTTP API client (plain JSON — no tronweb dependency). */
export class TronHttpRpcClient implements TronRpc {
  private readonly baseUrl: string;
  private readonly apiKey?: string;
  private readonly fetchFn: FetchLike;

  public constructor(options: TronHttpRpcClientOptions) {
    this.baseUrl = options.baseUrl.replace(/\/+$/, '');
    this.apiKey = options.apiKey;
    // Bind to globalThis: native `fetch` is spec'd to require `this` to be the
    // real global (window/self) — assigning the bare reference and later
    // calling it as `this.fetchFn(...)` rebinds `this` to the client instance,
    // which throws "Illegal invocation".
    const f = options.fetchFn ?? (globalThis.fetch?.bind(globalThis) as FetchLike | undefined);
    if (!f) {
      throw new Error('No fetch available; pass fetchFn in TronHttpRpcClientOptions.');
    }
    this.fetchFn = f;
  }

  private async post(path: string, body: unknown): Promise<Record<string, unknown>> {
    const headers: Record<string, string> = { 'content-type': 'application/json' };
    if (this.apiKey) {
      headers['TRON-PRO-API-KEY'] = this.apiKey;
    }
    const res = await this.fetchFn(`${this.baseUrl}${path}`, {
      method: 'POST',
      headers,
      body: JSON.stringify(body),
    });
    if (!res.ok) {
      throw new Error(`Tron RPC ${path} failed: HTTP ${res.status}`);
    }
    return (await res.json()) as Record<string, unknown>;
  }

  public async getTransactionInfo(txidHex: string): Promise<TronTxInfo | null> {
    const info = await this.post('/wallet/gettransactioninfobyid', { value: strip0x(txidHex) });
    if (!info || Object.keys(info).length === 0 || info.id == null) {
      return null;
    }
    const receipt = (info.receipt ?? {}) as Record<string, unknown>;
    const rawLogs = Array.isArray(info.log) ? (info.log as Record<string, unknown>[]) : [];
    const logs: TronLog[] = rawLogs.map((l) => ({
      address: String(l.address ?? '').toLowerCase(),
      topics: Array.isArray(l.topics) ? (l.topics as string[]).map((t) => String(t).toLowerCase()) : [],
      data: String(l.data ?? '').toLowerCase(),
    }));
    return {
      blockNumber: BigInt((info.blockNumber as number | string | undefined) ?? 0),
      success: String(receipt.result ?? '') === 'SUCCESS',
      logs,
    };
  }

  public async getNowBlockNumber(): Promise<bigint> {
    const block = await this.post('/wallet/getnowblock', {});
    const header = (block.block_header ?? {}) as Record<string, unknown>;
    const raw = (header.raw_data ?? {}) as Record<string, unknown>;
    return BigInt((raw.number as number | string | undefined) ?? 0);
  }
}
