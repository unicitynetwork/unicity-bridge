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

/** Inputs to a read-only (constant) contract call. */
export interface ConstantCallInput {
  /** Caller address, 20-byte EVM-form hex (no `0x`/`41`). */
  readonly ownerHex: string;
  /** Target contract, 20-byte EVM-form hex. */
  readonly contractHex: string;
  /** Solidity function signature, e.g. `allowance(address,address)`. */
  readonly functionSelector: string;
  /** ABI-encoded arguments, hex (no `0x`); empty for no-arg calls. */
  readonly parameterHex?: string;
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

/** A node that can also answer read-only (constant) contract calls (allowance, etc.). */
export interface TronConstantCaller {
  /** Returns the first `constant_result` word (hex, no `0x`); throws on revert. */
  triggerConstantContract(input: ConstantCallInput): Promise<string>;
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
export class TronHttpRpcClient implements TronRpc, TronConstantCaller {
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

  /**
   * Read-only (constant) contract call — used for `allowance(owner, spender)` so
   * bridge-in can skip a redundant `approve`. Addresses go over the wire in the
   * hex `41…` form (visible:false). Throws if the node reports the call reverted
   * or returns no result word.
   */
  public async triggerConstantContract(input: ConstantCallInput): Promise<string> {
    const res = await this.post('/wallet/triggerconstantcontract', {
      owner_address: '41' + strip0x(input.ownerHex).toLowerCase(),
      contract_address: '41' + strip0x(input.contractHex).toLowerCase(),
      function_selector: input.functionSelector,
      parameter: strip0x(input.parameterHex ?? ''),
      visible: false,
    });
    const result = (res.result ?? {}) as Record<string, unknown>;
    // `result.result === true` on success; a revert sets a code/message instead.
    if (result.result !== true && (result.code != null || result.message != null)) {
      const msg = typeof result.message === 'string' ? hexMessage(result.message) : String(result.code);
      throw new Error(`triggerconstantcontract ${input.functionSelector} reverted: ${msg}`);
    }
    const cr = res.constant_result;
    if (!Array.isArray(cr) || cr.length === 0 || typeof cr[0] !== 'string') {
      throw new Error(`triggerconstantcontract ${input.functionSelector} returned no result`);
    }
    return strip0x(cr[0]);
  }
}

/** Decode a possibly-hex Tron revert message into readable text (best effort). */
function hexMessage(m: string): string {
  if (!/^[0-9a-fA-F]*$/.test(m) || m.length % 2 !== 0) return m;
  try {
    const bytes = new Uint8Array(m.length / 2);
    for (let i = 0; i < bytes.length; i++) bytes[i] = parseInt(m.slice(i * 2, i * 2 + 2), 16);
    return new TextDecoder().decode(bytes).replace(/\0+/g, '').trim() || m;
  } catch {
    return m;
  }
}
