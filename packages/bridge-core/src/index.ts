/**
 * `@unicitylabs/bridge-core` — the chain-neutral bridge contracts (08 Phase 4,
 * the three-boundary abstraction). A wallet's bridge-in orchestration runs on
 * these interfaces alone; each source chain (Tron today, EVM next) ships a plugin
 * that *implements* them. The orchestrator therefore never imports a chain
 * package — only this one — so adding a chain is additive, never a fork.
 *
 * The three boundaries:
 *   - {ChainWallet}          connect · live account/network (no reads, no sends)
 *   - {ReceiptReader}        node reads: tx receipts (no signing)
 *   - {BridgeSourceAdapter}  "deposit X for recipient" -> opaque {DepositStep}[],
 *                            decode the commit, build the Unicity mint request
 * plus {BridgePresentation} (explorer link + address validation) for the UI.
 */
import type { MintJustificationVerifierService } from '@unicitylabs/state-transition-sdk/lib/transaction/verification/MintJustificationVerifierService.js';

// ── ChainWallet boundary ────────────────────────────────────────────────────

/**
 * The wallet capabilities the orchestrator needs: connect once, then read the
 * **live** account/network before every signature. No signing here — the deposit
 * steps sign via the wallet the adapter closed over.
 */
export interface ChainWallet {
  connect(): Promise<string>;
  getAddress(): Promise<string>;
  getNetwork(): Promise<number>;
}

// ── ReceiptReader boundary ──────────────────────────────────────────────────

/**
 * A committing/approval tx receipt the orchestrator inspects only for revert; the
 * rest is opaque and handed back to the adapter's `decodeCommit`. `null` until the
 * tx is mined.
 */
export interface TxReceipt {
  readonly success: boolean;
}

/** Node-read surface the orchestrator needs: a tx receipt by id. */
export interface ReceiptReader {
  getReceipt(txid: string): Promise<TxReceipt | null>;
}

// ── BridgeSourceAdapter boundary ────────────────────────────────────────────

/** One opaque step the orchestrator runs blindly (sign+broadcast → txid). */
export interface DepositStep {
  /** Progress label shown while the step runs. */
  readonly label: string;
  /** Whether the orchestrator must wait for this tx to succeed before the next step. */
  readonly awaitReceipt: boolean;
  /** Sign + broadcast; resolves to the txid. The wallet + call are the adapter's concern. */
  send(): Promise<string>;
}

/** Recovery material the orchestrator persists before the committing step. */
export interface DepositRecovery {
  readonly tokenIdHex: string;
  readonly saltHex: string;
  readonly recipientCommitmentHex: string;
  readonly coinIdHex: string;
  readonly tokenTypeHex: string;
  readonly chainId: number;
}

export interface PreparedDeposit {
  readonly recovery: DepositRecovery;
  /** Ordered steps; the one at {commitIndex} carries the commit (lock) event. */
  readonly steps: readonly DepositStep[];
  readonly commitIndex: number;
}

/** Decoded commit (lock) facts the mint justification binds to. */
export interface CommitInfo {
  readonly nonce: bigint;
  readonly blockNumber: bigint;
  readonly logIndex: number;
}

/** A chain-neutral Unicity mint request (the orchestrator hands this to the SDK). */
export interface MintRequest {
  readonly coinIdHex: string;
  readonly amount: bigint;
  /** Raw bridge-owned `MintTransaction.data` bytes. For Tron-USDT this is bare SDK `PaymentAssetCollection` CBOR. */
  readonly mintData: Uint8Array;
  readonly tokenType: Uint8Array;
  readonly salt: Uint8Array;
  readonly genesisReason: Uint8Array;
  readonly mintJustificationVerifierOverride: MintJustificationVerifierService;
}

export interface DepositParams {
  readonly amount: bigint;
  readonly networkId: number;
  readonly recipientPubkey?: Uint8Array;
  readonly ownerPredicateCbor?: Uint8Array;
  readonly approveAmount?: bigint;
}

export interface MintRequestArgs {
  readonly saltHex: string;
  readonly amount: bigint;
  readonly commit: CommitInfo;
  readonly commitTxid: string;
}

/** The chain-neutral bridge-in source the orchestrator drives. */
export interface BridgeSourceAdapter {
  /** Derive the recovery material + the ordered opaque deposit steps. */
  prepareDeposit(params: DepositParams): Promise<PreparedDeposit>;
  /** Decode a committing tx's raw receipt into {CommitInfo}; null if the event isn't present yet. */
  decodeCommit(rawReceipt: unknown): CommitInfo | null;
  /** Build the Unicity mint request for a (recovered) committed deposit. */
  buildMintRequest(args: MintRequestArgs): MintRequest;
}

// ── UI presentation ─────────────────────────────────────────────────────────

/**
 * The chain-specific UI presentation a bridge needs: a source-chain explorer link
 * and destination-address validation. The wallet UI holds one per bridge and never
 * keys on a numeric chainId or hardcodes a chain's URL / address shape — the bridge
 * supplies it.
 */
export interface BridgePresentation {
  /** Block-explorer URL for a source-chain transaction. */
  explorerTxUrl(txid: string): string;
  /** Structural validity of a destination address on this bridge's source chain. */
  validateAddress(addr: string): boolean;
}

// ── Manifest (chain-neutral fields) ─────────────────────────────────────────

/**
 * Chain-neutral fields every bridged-asset manifest carries, whatever the source
 * chain family. Integrity-pinned; all byte fields are lowercase hex, no `0x`. The
 * chain is identified by {chainRef} — a CAIP-2-style string/hex reference
 * (`tron:0x…`, `eip155:1`) — not a JavaScript number; a family's native numeric id
 * (if any) lives inside that family's plugin manifest variant.
 */
export interface BridgeManifestBase {
  /** Human label for the bridged asset, e.g. "USDT (bridged · Tron)". */
  readonly label: string;
  /** Short ticker for the primary balance display, e.g. "USDT". */
  readonly symbol: string;
  /** CAIP-2-style chain reference (e.g. `tron:0xcd8690dc`) — the generic chain identity. */
  readonly chainRef: string;
  /** Deployed vault (lock) address, in any of the chain's address forms. */
  readonly vault: string;
  /** Bridged asset (token) address, same forms. */
  readonly asset: string;
  /** Source-finality threshold an independent receiver enforces (K). */
  readonly confirmations: number;
  /** Token decimals. */
  readonly decimals: number;
  /** Part-B return-service base URL (bridge-back handoff). */
  readonly returnServiceUrl: string;
  /** `BridgeBackReason` CBOR tag the vault/prover bind (frozen config). */
  readonly reasonTag: number;
  /** 32-byte lock domain separator the deployed vault was constructed with (hex). */
  readonly lockDomain: string;
  /** 32-byte nullifier domain separator (hex). */
  readonly nullifierDomain: string;
  /** Groth16 verification key fingerprint the vault enforces (`0x…`); display/ops. */
  readonly vkey: string;
  /** 32-byte `configHash` the deployed vault self-derives (hex). Cross-checked at load. */
  readonly configHash: string;
  /** Optional explicit `tokenTypeHex`; derived + cross-checked when present. */
  readonly tokenTypeHex?: string;
  /** Optional explicit `coinIdHex`; derived + cross-checked when present. */
  readonly coinIdHex?: string;
}
