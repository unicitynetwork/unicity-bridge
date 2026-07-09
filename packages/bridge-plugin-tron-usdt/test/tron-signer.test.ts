import assert from 'node:assert/strict';
import { test } from 'node:test';

import { sendCallSigned, type InjectedTronWeb } from '../src/wallet/index.js';

const CALL = {
  contractHex: '41' + 'ab'.repeat(20),
  functionSignature: 'lock(uint256)',
  parameters: [{ type: 'uint256', value: '1' }],
};

function fakeTronWeb(): { tw: InjectedTronWeb; broadcasts: unknown[] } {
  const broadcasts: unknown[] = [];
  const tw = {
    defaultAddress: { base58: 'TTest' },
    transactionBuilder: {
      triggerSmartContract: async () => ({
        transaction: { txID: '11'.repeat(32), raw_data: { contract: [] } },
      }),
    },
    trx: {
      sign: async (transaction: unknown) => ({ ...(transaction as object), signature: ['sig'] }),
      sendRawTransaction: async (signed: unknown) => {
        broadcasts.push(signed);
        return { result: true, txid: '22'.repeat(32) };
      },
    },
    address: { toHex: () => '41' + 'cd'.repeat(20) },
  } as unknown as InjectedTronWeb;
  return { tw, broadcasts };
}

test('sendCallSigned accepts WalletConnect-style wrapped signed transaction', async () => {
  const { tw, broadcasts } = fakeTronWeb();
  const signed = { txID: '11'.repeat(32), raw_data: { contract: [] }, signature: ['sig'] };

  const txid = await sendCallSigned(tw, 'TTest', CALL, async () => ({ transaction: signed }));

  assert.equal(txid, '22'.repeat(32));
  assert.equal(broadcasts[0], signed);
});

test('sendCallSigned rejects unsupported wallet signature result before broadcast', async () => {
  const { tw, broadcasts } = fakeTronWeb();

  await assert.rejects(
    () => sendCallSigned(tw, 'TTest', CALL, async () => ({ txid: 'not-a-signed-transaction' })),
    /expected a signed Tron transaction object/,
  );
  assert.equal(broadcasts.length, 0);
});
