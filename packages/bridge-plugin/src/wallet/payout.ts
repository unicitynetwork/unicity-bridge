import { toEvmAddressHex } from '../address.js';
import type { ContractCall } from '../contract-call.js';
import type { ConstantCaller } from '../source-chain.js';
import type { LoadedBridge } from './manifest.js';

export async function owedTo(bridge: LoadedBridge, rpc: ConstantCaller, destination: string): Promise<bigint> {
  const destinationHex = toEvmAddressHex(destination);
  const word = await rpc.constantCall({
    ownerHex: destinationHex,
    contractHex: bridge.plugin.resolvedConfig.lockContractHex,
    functionSignature: 'owed(address)',
    parameterHex: destinationHex.padStart(64, '0'),
  });
  return word ? BigInt('0x' + word) : 0n;
}

export function withdrawCall(bridge: LoadedBridge): ContractCall {
  return { contractHex: bridge.plugin.resolvedConfig.lockContractHex, functionSignature: 'withdraw()', parameters: [] };
}
