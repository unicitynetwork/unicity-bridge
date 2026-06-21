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
type FetchLike = (input: string, init?: {
    method: string;
    headers: Record<string, string>;
    body: string;
}) => Promise<{
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
/** Tron full-node HTTP API client (plain JSON — no tronweb dependency). */
export declare class TronHttpRpcClient implements TronRpc {
    private readonly baseUrl;
    private readonly apiKey?;
    private readonly fetchFn;
    constructor(options: TronHttpRpcClientOptions);
    private post;
    getTransactionInfo(txidHex: string): Promise<TronTxInfo | null>;
    getNowBlockNumber(): Promise<bigint>;
}
export {};
