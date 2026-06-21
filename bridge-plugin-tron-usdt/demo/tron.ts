import { readFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

import { TronWeb } from 'tronweb';

import type { DemoEnv } from './env.js';

const HERE = dirname(fileURLToPath(import.meta.url));
const ARTIFACTS = join(HERE, '..', '..', 'contracts', 'tron', 'artifacts', 'contracts');

interface HardhatArtifact {
  abi: unknown[];
  bytecode: string;
}

export function loadArtifact(rel: string): HardhatArtifact {
  return JSON.parse(readFileSync(join(ARTIFACTS, rel), 'utf8')) as HardhatArtifact;
}

export const MOCK_TRC20 = (): HardhatArtifact => loadArtifact('test/MockTRC20.sol/MockTRC20.json');
export const UNICITY_LOCK = (): HardhatArtifact => loadArtifact('UnicityLock.sol/UnicityLock.json');

const FEE_LIMIT = 1_500_000_000; // 1500 TRX cap; Nile deploys cost far less.

export function makeTronWeb(env: DemoEnv, privateKey?: string): TronWeb {
  return new TronWeb({
    fullHost: env.tronRpc,
    headers: env.tronApiKey ? { 'TRON-PRO-API-KEY': env.tronApiKey } : undefined,
    privateKey: privateKey ?? undefined,
  });
}

/** Lowercase 20-byte EVM-form hex (strips the Tron `41` prefix), no `0x`. */
export function toEvmHex(tronWeb: TronWeb, addr: string): string {
  const hex = tronWeb.address.toHex(addr); // 41 + 40 hex chars
  return hex.replace(/^0x/i, '').replace(/^41/, '').toLowerCase();
}

function strip0x(h: string): string {
  return h.startsWith('0x') || h.startsWith('0X') ? h.slice(2) : h;
}

export interface Deployed {
  base58: string;
  evmHex: string;
  txid: string;
}

/** Deploy a Hardhat-compiled contract to Tron and wait for it to confirm. */
export async function deployContract(
  tronWeb: TronWeb,
  artifact: HardhatArtifact,
  name: string,
  parameters: unknown[],
): Promise<Deployed> {
  const issuer = tronWeb.defaultAddress.base58 as string;
  const unsigned = await tronWeb.transactionBuilder.createSmartContract(
    {
      abi: artifact.abi as never,
      bytecode: strip0x(artifact.bytecode),
      feeLimit: FEE_LIMIT,
      callValue: 0,
      userFeePercentage: 100,
      originEnergyLimit: 10_000_000,
      parameters,
      name,
    },
    issuer,
  );
  const signed = await tronWeb.trx.sign(unsigned);
  const receipt = (await tronWeb.trx.sendRawTransaction(signed)) as { result?: boolean; txid?: string };
  const txid = receipt.txid ?? (unsigned as { txID: string }).txID;
  if (!txid) {
    throw new Error(`Deploy of ${name} returned no txid: ${JSON.stringify(receipt)}`);
  }
  const contractHex = (unsigned as { contract_address: string }).contract_address; // 41...
  await waitForReceipt(tronWeb, txid);
  return {
    base58: tronWeb.address.fromHex(contractHex),
    evmHex: contractHex.replace(/^41/, '').toLowerCase(),
    txid,
  };
}

/** Poll until a transaction has a confirmed receipt; returns its info. */
export async function waitForReceipt(
  tronWeb: TronWeb,
  txid: string,
  timeoutMs = 120_000,
): Promise<{ blockNumber: number; log: TronInfoLog[]; receiptResult: string }> {
  const deadline = Date.now() + timeoutMs;
  for (;;) {
    const info = (await tronWeb.trx.getTransactionInfo(txid)) as TronTransactionInfo;
    if (info && info.id && typeof info.blockNumber === 'number') {
      const result = info.receipt?.result ?? 'SUCCESS'; // deploys report no explicit result on success
      if (result !== 'SUCCESS' && result !== 'OK' && result !== undefined) {
        throw new Error(`Tx ${txid} failed on Tron: ${result} (${info.resMessage ?? ''})`);
      }
      return { blockNumber: info.blockNumber, log: info.log ?? [], receiptResult: result };
    }
    if (Date.now() > deadline) {
      throw new Error(`Timed out waiting for Tron tx ${txid} to confirm.`);
    }
    await sleep(3000);
  }
}

interface TronInfoLog {
  address: string;
  topics: string[];
  data: string;
}
interface TronTransactionInfo {
  id?: string;
  blockNumber?: number;
  log?: TronInfoLog[];
  resMessage?: string;
  receipt?: { result?: string };
}

/** Call a state-changing contract method and return its txid. */
export async function sendMethod(
  tronWeb: TronWeb,
  abi: unknown[],
  addressBase58: string,
  method: string,
  args: unknown[],
): Promise<string> {
  const contract = tronWeb.contract(abi as never, addressBase58);
  const txid = (await contract[method](...args).send({ feeLimit: FEE_LIMIT, callValue: 0 })) as string;
  return txid;
}

export async function getNowBlock(tronWeb: TronWeb): Promise<number> {
  const block = (await tronWeb.trx.getCurrentBlock()) as { block_header?: { raw_data?: { number?: number } } };
  return block.block_header?.raw_data?.number ?? 0;
}

export function sleep(ms: number): Promise<void> {
  return new Promise((r) => setTimeout(r, ms));
}
