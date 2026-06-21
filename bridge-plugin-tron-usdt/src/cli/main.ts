#!/usr/bin/env node
import { CertifiedMintTransaction } from '@unicitylabs/state-transition-sdk/lib/transaction/CertifiedMintTransaction.js';
import { MintJustificationVerifierService } from '@unicitylabs/state-transition-sdk/lib/transaction/verification/MintJustificationVerifierService.js';
import { VerificationStatus } from '@unicitylabs/state-transition-sdk/lib/verification/VerificationStatus.js';

import {
  createTronUsdtBridgePlugin,
  TRON_MAINNET_CHAIN_ID,
  TRON_NILE_CHAIN_ID,
  TRON_MAINNET_USDT,
  type TronUsdtBridgeConfig,
} from '../index.js';
import { buildDemo, hexToBytes } from './scenario.js';

const AMOUNT = 1_000_000n; // 1 USDT (6 decimals)
const CONFIRMATIONS = 20;

// Nile demo config (addresses are illustrative for the offline demo).
const DEMO_CONFIG: TronUsdtBridgeConfig = {
  chainId: TRON_NILE_CHAIN_ID,
  lockContract: '41' + 'ab'.repeat(20),
  assetContract: '41' + 'de'.repeat(20),
  confirmations: CONFIRMATIONS,
  decimals: 6,
};

const service = () => new MintJustificationVerifierService();

interface Check {
  name: string;
  expect: VerificationStatus;
  build: () => Promise<{ status: VerificationStatus; message: string }>;
}

async function run(build: Awaited<ReturnType<typeof buildDemo>>): Promise<{ status: VerificationStatus; message: string }> {
  const r = await build.plugin.verifier.verify(build.certifiedTx, service());
  return { status: r.status, message: r.message };
}

async function demo(): Promise<number> {
  console.log('Unicity ⇽ Tron USDT bridge — offline security demo');
  console.log('(mock Tron RPC; mint reason = self-contained lock proof)\n');

  const checks: Check[] = [
    {
      name: 'Valid bridged mint (lock finalized, bound to this token+recipient)',
      expect: VerificationStatus.OK,
      build: () => buildDemo(DEMO_CONFIG, AMOUNT, CONFIRMATIONS).then(run),
    },
    {
      name: 'Attack: inflate token value above locked amount',
      expect: VerificationStatus.FAIL,
      build: () => buildDemo(DEMO_CONFIG, AMOUNT, CONFIRMATIONS, { tokenValueAmount: AMOUNT * 1000n }).then(run),
    },
    {
      name: 'Attack: tamper justification amount',
      expect: VerificationStatus.FAIL,
      build: () =>
        buildDemo(DEMO_CONFIG, AMOUNT, CONFIRMATIONS, {
          justification: (d) => ({ ...d, amount: AMOUNT + 1n }),
        }).then(run),
    },
    {
      name: 'Attack: replay lock for a different token id',
      expect: VerificationStatus.FAIL,
      build: () =>
        buildDemo(DEMO_CONFIG, AMOUNT, CONFIRMATIONS, {
          logs: (def) => [{ ...def, data: def.data.slice(0, 64) + '99'.repeat(32) + def.data.slice(128) }],
        }).then(run),
    },
    {
      name: 'Attack: steal lock by swapping recipient',
      expect: VerificationStatus.FAIL,
      build: () =>
        buildDemo(DEMO_CONFIG, AMOUNT, CONFIRMATIONS, {
          logs: (def) => [{ ...def, data: def.data.slice(0, 128) + '88'.repeat(32) }],
        }).then(run),
    },
    {
      name: 'Attack: forged lock contract emits the event',
      expect: VerificationStatus.FAIL,
      build: () =>
        buildDemo(DEMO_CONFIG, AMOUNT, CONFIRMATIONS, {
          logs: (def) => [{ ...def, address: 'ff'.repeat(20) }],
        }).then(run),
    },
    {
      name: 'Attack: spend lock before finality (insufficient confirmations)',
      expect: VerificationStatus.FAIL,
      build: () =>
        buildDemo(DEMO_CONFIG, AMOUNT, CONFIRMATIONS, {
          blockNumber: 1000n,
          tip: 1000n + BigInt(CONFIRMATIONS) - 1n,
        }).then(run),
    },
  ];

  let failures = 0;
  for (const check of checks) {
    const { status, message } = await check.build();
    const ok = status === check.expect;
    if (!ok) {
      failures++;
    }
    const tag = ok ? '✔' : '✘ UNEXPECTED';
    const verdict = status === VerificationStatus.OK ? 'OK  ' : 'FAIL';
    console.log(`${tag}  [${verdict}] ${check.name}`);
    if (status === VerificationStatus.FAIL && message) {
      console.log(`         ↳ ${message}`);
    }
  }

  console.log('');
  if (failures === 0) {
    console.log('All checks behaved as expected: valid token accepted, every attack rejected.');
    return 0;
  }
  console.error(`${failures} check(s) did not behave as expected.`);
  return 1;
}

async function verifyOnChain(args: Map<string, string>): Promise<number> {
  const tokenHex = args.get('token');
  const lockContract = args.get('lock');
  const rpcUrl = args.get('rpc');
  if (!tokenHex || !lockContract || !rpcUrl) {
    console.error('Usage: tron-usdt-bridge verify --token <hex> --lock <addr> --rpc <url> [--asset <addr>] [--chain mainnet|nile] [--api-key <key>] [--confirmations N]');
    return 2;
  }
  const chain = (args.get('chain') ?? 'mainnet') === 'nile' ? TRON_NILE_CHAIN_ID : TRON_MAINNET_CHAIN_ID;
  const config: TronUsdtBridgeConfig = {
    chainId: chain,
    lockContract,
    assetContract: args.get('asset') ?? TRON_MAINNET_USDT,
    confirmations: Number(args.get('confirmations') ?? CONFIRMATIONS),
    decimals: 6,
    rpcUrl,
    apiKey: args.get('api-key'),
  };
  // NOTE: real Sphere tokens carry value as SpherePaymentData; pass a matching
  // extractAmount via the sphere-sdk wiring. This CLI uses the simple envelope.
  const plugin = createTronUsdtBridgePlugin(config);
  const tx = await CertifiedMintTransaction.fromCBOR(hexToBytes(tokenHex));
  const result = await plugin.verifier.verify(tx, service());
  console.log(`Token type expected for this asset: ${plugin.tokenTypeHex}`);
  console.log(`Verification: ${result.status}${result.message ? ` — ${result.message}` : ''}`);
  return result.status === VerificationStatus.OK ? 0 : 1;
}

function parseArgs(argv: string[]): Map<string, string> {
  const map = new Map<string, string>();
  for (let i = 0; i < argv.length; i++) {
    if (argv[i].startsWith('--')) {
      const key = argv[i].slice(2);
      const val = argv[i + 1] && !argv[i + 1].startsWith('--') ? argv[(i += 1)] : 'true';
      map.set(key, val);
    }
  }
  return map;
}

async function main(): Promise<void> {
  const [cmd, ...rest] = process.argv.slice(2);
  let code: number;
  switch (cmd ?? 'demo') {
    case 'demo':
      code = await demo();
      break;
    case 'verify':
      code = await verifyOnChain(parseArgs(rest));
      break;
    default:
      console.error(`Unknown command: ${cmd}. Use "demo" or "verify".`);
      code = 2;
  }
  process.exit(code);
}

void main();
