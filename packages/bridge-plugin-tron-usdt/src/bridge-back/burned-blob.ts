import { StateId } from '@unicitylabs/state-transition-sdk/lib/api/StateId.js';
import { Token } from '@unicitylabs/state-transition-sdk/lib/transaction/Token.js';

export interface BurnIdentifiers {
  readonly burnStateId: Uint8Array;
  readonly burnTxHash: Uint8Array;
  readonly reasonBytes: Uint8Array;
}

export async function burnIdentifiers(burnedToken: Uint8Array): Promise<BurnIdentifiers> {
  const token = await Token.fromCBOR(burnedToken);
  const burn = token.transactions.at(-1);
  if (!burn) throw new Error('burnIdentifiers: the blob has no transfer; it is not a burned token');
  const reasonBytes = burn.data;
  if (!reasonBytes) throw new Error('burnIdentifiers: the terminal transfer carries no reason bytes');
  return {
    burnStateId: (await StateId.fromTransaction(burn)).data,
    burnTxHash: (await burn.calculateTransactionHash()).data,
    reasonBytes: new Uint8Array(reasonBytes),
  };
}
