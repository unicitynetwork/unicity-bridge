import { selectorHex } from '../contract-call.js';
import type { ConstantCallInput, ConstantCaller, LogFilter, LogReader, SourceChainRpc, SourceLog, SourceLogEntry, SourceTxInfo } from '../source-chain.js';

type FetchLike = (
  input: string,
  init?: { method: string; headers: Record<string, string>; body: string },
) => Promise<{ ok: boolean; status: number; json: () => Promise<unknown> }>;

export interface EvmJsonRpcClientOptions {
  readonly rpcUrl: string;
  /** Injectable fetch (defaults to globalThis.fetch). */
  readonly fetchFn?: FetchLike;
}

interface RpcLog {
  readonly address: string;
  readonly topics: string[];
  readonly data: string;
}

interface RpcReceipt {
  readonly blockNumber: string;
  readonly status: string;
  readonly logs: readonly RpcLog[];
}

interface RpcLogEntry extends RpcLog {
  readonly blockNumber: string;
  readonly transactionHash: string;
}

function strip0x(h: string): string {
  return h.startsWith('0x') || h.startsWith('0X') ? h.slice(2) : h;
}

function quantity(n: bigint): string {
  return `0x${n.toString(16)}`;
}

function shapeLog(l: RpcLog): SourceLog {
  return {
    address: strip0x(l.address).toLowerCase(),
    topics: l.topics.map((t) => strip0x(t).toLowerCase()),
    data: strip0x(l.data).toLowerCase(),
  };
}

export class EvmJsonRpcClient implements SourceChainRpc, ConstantCaller, LogReader {
  private readonly rpcUrl: string;
  private readonly fetchFn: FetchLike;

  public constructor(options: EvmJsonRpcClientOptions) {
    this.rpcUrl = options.rpcUrl;
    const f = options.fetchFn ?? (globalThis.fetch?.bind(globalThis) as FetchLike | undefined);
    if (!f) {
      throw new Error('No fetch available; pass fetchFn in EvmJsonRpcClientOptions.');
    }
    this.fetchFn = f;
  }

  private async call<T>(method: string, params: unknown[]): Promise<T> {
    const res = await this.fetchFn(this.rpcUrl, {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({ jsonrpc: '2.0', id: 1, method, params }),
    });
    if (!res.ok) {
      throw new Error(`Ethereum RPC ${method} failed: HTTP ${res.status}`);
    }
    const body = (await res.json()) as { result?: T; error?: { code: number; message: string; data?: unknown } };
    if (body.error) {
      throw new Error(`Ethereum RPC ${method} failed: ${body.error.message}`);
    }
    return body.result as T;
  }

  public async getTransactionInfo(txidHex: string): Promise<SourceTxInfo | null> {
    const receipt = await this.call<RpcReceipt | null>('eth_getTransactionReceipt', [`0x${strip0x(txidHex)}`]);
    if (!receipt) {
      return null;
    }
    return { blockNumber: BigInt(receipt.blockNumber), success: receipt.status === '0x1', logs: receipt.logs.map(shapeLog) };
  }

  public async getLogs(filter: LogFilter): Promise<SourceLogEntry[]> {
    const entries = await this.call<RpcLogEntry[]>('eth_getLogs', [
      {
        address: `0x${strip0x(filter.address)}`,
        topics: filter.topics.map((t) => (t === null ? null : `0x${strip0x(t)}`)),
        fromBlock: quantity(filter.fromBlock),
        toBlock: quantity(filter.toBlock),
      },
    ]);
    return entries.map((l) => ({ ...shapeLog(l), blockNumber: BigInt(l.blockNumber), transactionHash: strip0x(l.transactionHash).toLowerCase() }));
  }

  public async getNowBlockNumber(): Promise<bigint> {
    return BigInt(await this.call<string>('eth_blockNumber', []));
  }

  public async constantCall(input: ConstantCallInput): Promise<string> {
    const data = `0x${selectorHex(input.functionSignature)}${strip0x(input.parameterHex ?? '')}`;
    const result = await this.call<string>('eth_call', [
      { from: `0x${strip0x(input.ownerHex)}`, to: `0x${strip0x(input.contractHex)}`, data },
      'latest',
    ]);
    return strip0x(result);
  }
}
