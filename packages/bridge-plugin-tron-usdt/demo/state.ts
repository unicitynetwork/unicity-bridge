import { readFileSync, writeFileSync, existsSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const HERE = dirname(fileURLToPath(import.meta.url));
export const STATE_PATH = join(HERE, '.demo-state.json');

/**
 * Persistent state shared across the e2e demo steps. Each subcommand reads the
 * fields it needs and writes the ones it produces, so the whole flow can be run
 * one command at a time from a shell (see ../DEMO.md). Everything here is public
 * testnet data — addresses, tx hashes, and throwaway demo keys.
 */
export interface DemoState {
  // --- Tron (Nile) deployment ---
  tron?: {
    chainId: number;
    rpcUrl: string;
    deployerBase58: string;
    deployerEvmHex: string;
    /** MockTRC20 standing in for USDT. */
    assetBase58: string;
    assetEvmHex: string;
    /** UnicityLock contract. */
    lockBase58: string;
    lockEvmHex: string;
    deployTxids: { asset: string; lock: string; mint: string };
  };

  // --- The bridge intent: which Unicity token this lock will fund ---
  intent?: {
    /** 32-byte Unicity TokenId (hex). */
    tokenIdHex: string;
    /** TokenSalt bytes (hex) used to derive tokenId. */
    saltHex: string;
    /** SHA256(recipient predicate CBOR), committed on Tron (hex). */
    recipientCommitmentHex: string;
    /** Throwaway secp256k1 key of the bridge recipient on Unicity (hex). */
    recipientPrivKeyHex: string;
    /** Locked USDT amount (6 decimals), as a decimal string. */
    amount: string;
  };

  // --- The on-chain Tron lock that backs the mint ---
  lock?: {
    txid: string;
    blockNumber: number;
    logIndex: number;
    nonce: number;
  };

  // --- Unicity (testnet2) minted token ---
  mint?: {
    networkId: number;
    aggregatorUrl: string;
    tokenTypeHex: string;
    coinIdHex: string;
    /** Owner == bridge recipient after mint (hex priv key). */
    ownerPrivKeyHex: string;
    /** Serialized Token (CBOR hex). */
    tokenCborHex: string;
  };

  // --- Unicity transfer to a second party ---
  transfer?: {
    /** New owner's throwaway key (hex). */
    recipientPrivKeyHex: string;
    /** Serialized transferred Token (CBOR hex). */
    tokenCborHex: string;
  };
}

export function loadState(): DemoState {
  if (!existsSync(STATE_PATH)) {
    return {};
  }
  return JSON.parse(readFileSync(STATE_PATH, 'utf8')) as DemoState;
}

export function saveState(state: DemoState): void {
  writeFileSync(STATE_PATH, JSON.stringify(state, null, 2) + '\n');
}

export function requireState<K extends keyof DemoState>(state: DemoState, key: K, step: string): NonNullable<DemoState[K]> {
  const v = state[key];
  if (v == null) {
    throw new Error(`Missing "${key}" in demo state — run \`${step}\` first.`);
  }
  return v as NonNullable<DemoState[K]>;
}
