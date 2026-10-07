export interface SourceLog {
  readonly address: string;
  /** Indexed topics (topic0 = event signature hash). */
  readonly topics: string[];
  /** ABI-encoded non-indexed args. */
  readonly data: string;
}

export interface SourceTxInfo {
  readonly blockNumber: bigint;
  readonly success: boolean;
  readonly logs: SourceLog[];
}

/** A log as a node's log query returns it: the receipt log plus where it was emitted. */
export interface SourceLogEntry extends SourceLog {
  readonly blockNumber: bigint;
  readonly transactionHash: string;
}

export interface LogFilter {
  readonly address: string;
  /** Topic per position; `null` matches any value at that position. */
  readonly topics: readonly (string | null)[];
  readonly fromBlock: bigint;
  readonly toBlock: bigint;
}

export interface LogReader {
  getLogs(filter: LogFilter): Promise<SourceLogEntry[]>;
}

export interface ConstantCallInput {
  readonly ownerHex: string;
  readonly contractHex: string;
  /** Solidity function signature, e.g. `allowance(address,address)`. */
  readonly functionSignature: string;
  /** ABI-encoded arguments, hex (no `0x`); empty for no-arg calls. */
  readonly parameterHex?: string;
}

export interface SourceChainRpc {
  /** Returns null when the transaction is unknown to the node. */
  getTransactionInfo(txidHex: string): Promise<SourceTxInfo | null>;
  /** Current chain tip block number. */
  getNowBlockNumber(): Promise<bigint>;
}

export interface ConstantCaller {
  constantCall(input: ConstantCallInput): Promise<string>;
}
