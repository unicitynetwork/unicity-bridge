import { NetworkId } from '@unicitylabs/state-transition-sdk/lib/api/NetworkId.js';
import { SigningService } from '@unicitylabs/state-transition-sdk/lib/crypto/secp256k1/SigningService.js';
import { EncodedPredicate } from '@unicitylabs/state-transition-sdk/lib/predicate/EncodedPredicate.js';
import { SignaturePredicate } from '@unicitylabs/state-transition-sdk/lib/predicate/builtin/SignaturePredicate.js';
import { MintTransaction } from '@unicitylabs/state-transition-sdk/lib/transaction/MintTransaction.js';
import { TokenId } from '@unicitylabs/state-transition-sdk/lib/transaction/TokenId.js';
import { TokenSalt } from '@unicitylabs/state-transition-sdk/lib/transaction/TokenSalt.js';
import { TokenType } from '@unicitylabs/state-transition-sdk/lib/transaction/TokenType.js';
import { createTronUsdtBridgePlugin, encodeBridgedValue, LOCK_EVENT_TOPIC0, recipientCommitment, toHex, TronUsdtLockJustification, } from '../index.js';
export class MockTronRpc {
    txInfo;
    tip;
    constructor(txInfo, tip) {
        this.txInfo = txInfo;
        this.tip = tip;
    }
    async getTransactionInfo() {
        return this.txInfo;
    }
    async getNowBlockNumber() {
        return this.tip;
    }
}
function word(v) {
    if (v instanceof Uint8Array) {
        return toHex(v).padStart(64, '0');
    }
    return v.toString(16).padStart(64, '0');
}
export function makeLockLog(addressEvmHex, e) {
    return {
        address: addressEvmHex.toLowerCase(),
        topics: [LOCK_EVENT_TOPIC0, word(e.nonce), e.fromEvmHex.toLowerCase().padStart(64, '0')],
        data: word(e.amount) + word(e.unicityTokenId) + word(e.recipientCommitment),
    };
}
export const DEMO_NETWORK = NetworkId.fromId(4);
export const DEMO_TXID = 'a1'.repeat(32);
export const DEMO_NONCE = 7n;
/** Builds a valid lock + matching bridged mint, then applies optional tampering. */
export async function buildDemo(config, amount, confirmations, overrides = {}) {
    const recipient = SignaturePredicate.create(SigningService.generate().publicKey);
    const recipientCommitmentBytes = recipientCommitment(EncodedPredicate.fromPredicate(recipient).toCBOR());
    const salt = TokenSalt.generate();
    const tokenId = await TokenId.fromSalt(DEMO_NETWORK, salt);
    const blockNumber = overrides.blockNumber ?? 1000n;
    const tip = overrides.tip ?? blockNumber + BigInt(confirmations);
    // Build a probe plugin only to learn the normalized lock-contract hex.
    const probe = createTronUsdtBridgePlugin(config, { rpc: new MockTronRpc(null, 0n) });
    const defaultLog = makeLockLog(probe.resolvedConfig.lockContractHex, {
        nonce: DEMO_NONCE,
        fromEvmHex: 'cc'.repeat(20),
        amount,
        unicityTokenId: tokenId.bytes,
        recipientCommitment: recipientCommitmentBytes,
    });
    const logs = overrides.logs ? overrides.logs(defaultLog) : [defaultLog];
    const rpc = new MockTronRpc({ blockNumber, success: true, logs }, tip);
    const plugin = createTronUsdtBridgePlugin(config, { rpc });
    let jData = {
        chainId: config.chainId,
        lockContract: hexToBytes(plugin.resolvedConfig.lockContractHex),
        assetContract: hexToBytes(plugin.resolvedConfig.assetContractHex),
        txid: hexToBytes(DEMO_TXID),
        logIndex: 0,
        amount,
        nonce: DEMO_NONCE,
    };
    if (overrides.justification) {
        jData = overrides.justification(jData);
    }
    const valueData = encodeBridgedValue(plugin.resolvedConfig.coinId, overrides.tokenValueAmount ?? amount);
    const tokenType = new TokenType(plugin.resolvedConfig.tokenType);
    const mint = await MintTransaction.create(DEMO_NETWORK, recipient, valueData, tokenType, salt, new TronUsdtLockJustification(jData).toCBOR());
    return {
        plugin,
        certifiedTx: mint,
        rpc,
        tokenIdHex: toHex(tokenId.bytes),
    };
}
export function hexToBytes(hex) {
    const h = hex.startsWith('0x') ? hex.slice(2) : hex;
    const out = new Uint8Array(h.length / 2);
    for (let i = 0; i < out.length; i++) {
        out[i] = parseInt(h.slice(i * 2, i * 2 + 2), 16);
    }
    return out;
}
