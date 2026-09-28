import assert from 'node:assert/strict';
import { test } from 'node:test';

import { sendCallSigned, type InjectedTronWeb } from '../src/wallet/index.js';

const CALL = {
  contractHex: 'ab'.repeat(20),
  functionSignature: 'approve(address,uint256)',
  parameters: [
    { type: 'address', value: 'ef'.repeat(20) },
    { type: 'uint256', value: '1' },
  ],
} as const;

function fakeTronWeb(): { tw: InjectedTronWeb; broadcasts: unknown[]; built: unknown[] } {
  const broadcasts: unknown[] = [];
  const built: unknown[] = [];
  const tw = {
    defaultAddress: { base58: 'TTest' },
    transactionBuilder: {
      triggerSmartContract: async (...args: unknown[]) => {
        built.push(args);
        return { transaction: { txID: '11'.repeat(32), raw_data: { contract: [] } } };
      },
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
  return { tw, broadcasts, built };
}

test('sendCallSigned accepts WalletConnect-style wrapped signed transaction', async () => {
  const { tw, broadcasts } = fakeTronWeb();
  const signed = { txID: '11'.repeat(32), raw_data: { contract: [] }, signature: ['sig'] };

  const txid = await sendCallSigned(tw, 'TTest', CALL, async () => ({ transaction: signed }));

  assert.equal(txid, '22'.repeat(32));
  assert.equal(broadcasts[0], signed);
});

test('sendCallSigned hands TronWeb the call in its 41-prefixed address form', async () => {
  const { tw, built } = fakeTronWeb();
  await sendCallSigned(tw, 'TTest', CALL, async (t) => ({ ...(t as object), signature: ['sig'] }));
  const [contractAddress, functionSignature, , parameters] = built[0] as [string, string, unknown, { type: string; value: string }[]];
  assert.equal(contractAddress, '41' + 'ab'.repeat(20));
  assert.equal(functionSignature, 'approve(address,uint256)');
  assert.deepEqual(parameters, [
    { type: 'address', value: '41' + 'ef'.repeat(20) },
    { type: 'uint256', value: '1' },
  ]);
});

test('sendCallSigned rejects unsupported wallet signature result before broadcast', async () => {
  const { tw, broadcasts } = fakeTronWeb();

  await assert.rejects(
    () => sendCallSigned(tw, 'TTest', CALL, async () => ({ txid: 'not-a-signed-transaction' })),
    /expected a signed Tron transaction object/,
  );
  assert.equal(broadcasts.length, 0);
});
