import { keccak_256 } from '@noble/hashes/sha3.js';

import { Addr, B32, U256, encode, type Val } from './bridge-back/abi.js';
import { fromHex, toHex } from './hex.js';

export interface CallParam {
  readonly type: 'address' | 'uint256' | 'bytes32';
  readonly value: string;
}

export interface ContractCall {
  readonly contractHex: string;
  /** Solidity function signature, e.g. `lock(uint256,bytes32,bytes32)`. */
  readonly functionSignature: string;
  /** Typed parameters in declaration order. */
  readonly parameters: readonly CallParam[];
}

export function selectorHex(functionSignature: string): string {
  return toHex(keccak_256(new TextEncoder().encode(functionSignature)).subarray(0, 4));
}

function paramValue(p: CallParam): Val {
  switch (p.type) {
    case 'address':
      return Addr(fromHex(p.value));
    case 'bytes32':
      return B32(fromHex(p.value));
    case 'uint256':
      return U256(BigInt(p.value));
  }
}

export function encodeCallData(call: ContractCall): string {
  return selectorHex(call.functionSignature) + toHex(encode(call.parameters.map(paramValue)));
}
