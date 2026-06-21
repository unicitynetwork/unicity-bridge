import type { CertifiedMintTransaction } from '@unicitylabs/state-transition-sdk/lib/transaction/CertifiedMintTransaction.js';
import type { IMintJustificationVerifier } from '@unicitylabs/state-transition-sdk/lib/transaction/verification/IMintJustificationVerifier.js';
import type { MintJustificationVerifierService } from '@unicitylabs/state-transition-sdk/lib/transaction/verification/MintJustificationVerifierService.js';
import { VerificationResult } from '@unicitylabs/state-transition-sdk/lib/verification/VerificationResult.js';
import { VerificationStatus } from '@unicitylabs/state-transition-sdk/lib/verification/VerificationStatus.js';
import type { TronRpc } from './TronRpcClient.js';
import { type BridgedAmountExtractor } from './value.js';
/** Trust anchors + identifiers a verifier instance is bound to. */
export interface ResolvedTronUsdtConfig {
    readonly chainId: number;
    /** 20-byte EVM-form address, lowercase hex. */
    readonly lockContractHex: string;
    /** 20-byte EVM-form address, lowercase hex. */
    readonly assetContractHex: string;
    readonly confirmations: number;
    /** 32-byte Unicity TokenType for this asset. */
    readonly tokenType: Uint8Array;
    /** 32-byte Sphere coinId for this asset. */
    readonly coinId: Uint8Array;
}
export interface TronUsdtVerifierDeps {
    readonly rpc: TronRpc;
    /** Defaults to the simple CLI value envelope decoder. */
    readonly extractAmount?: BridgedAmountExtractor;
}
/**
 * Validates a bridged USDT-on-Tron token by re-checking its mint reason against
 * a Tron node: the lock exists, is final, has the right amount, and commits to
 * exactly this token's id + recipient. See docs/bridge/MINT_REASON.md.
 */
export declare class TronUsdtMintJustificationVerifier implements IMintJustificationVerifier {
    private readonly config;
    private readonly rpc;
    private readonly extractAmount;
    constructor(config: ResolvedTronUsdtConfig, deps: TronUsdtVerifierDeps);
    get tag(): bigint;
    verify(transaction: CertifiedMintTransaction, _service: MintJustificationVerifierService): Promise<VerificationResult<VerificationStatus>>;
}
