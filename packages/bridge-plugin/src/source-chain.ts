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
