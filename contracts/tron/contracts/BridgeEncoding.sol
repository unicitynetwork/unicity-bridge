// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

// ---------------------------------------------------------------------------
// Cross-stack types and encodings for the return path.
//
// Every value below is recomputed on-chain by the vault and MUST be byte-
// identical to the prover (Rust) and the TS SDK. The single source of truth is
// protocol/interop.md (§1 hash policy, §2 config, §3
// lock, §7 public statement). The keccak/ABI layouts here mirror the reference
// generator in protocol/vectors/gen (src/derive.rs, src/abi.rs); the conformance
// tests assert equality against protocol/vectors/{config,lock,public}.
//
// Hash policy (00 §1): everything the vault recomputes uses keccak256 over
// abi.encode (`K`). SHA-256/CBOR values (nullifier, accumulator) are never
// recomputed on-chain — they only ride through as opaque bytes32.
// ---------------------------------------------------------------------------

/// Deployment configuration (00 §2). The vault derives CONFIG_HASH and ASSET
/// from this one struct in its constructor so they cannot diverge.
struct BridgeConfig {
    uint64  sourceChainId;
    address vault;            // must equal the deployed vault address
    address asset;            // the TRC20/ERC20 under custody; = ASSET
    bytes32 tokenType;        // Unicity bridged-asset token type
    bytes32 coinId;           // value id in token payment data
    uint64  reasonTag;        // CBOR tag of BridgeBackReason
    bytes32 lockDomain;       // domain separator for lockDigest
    bytes32 nullifierDomain;  // domain separator for the nullifier
}

/// One settlement leaf, committed by `returnRoot` in submission order (00 §7).
struct ReturnLeaf {
    bytes32 nullifier;
    address recipient;
    uint256 amount;
    address feeRecipient;
    uint256 feeAmount;
    uint64  deadline;
}

/// One source-lock reference, committed by `lockRefRoot` sorted by nonce (00 §7).
struct SourceLockRef {
    uint256 nonce;
    bytes32 digest;
}

/// The public statement the circuit commits and the vault decodes (00 §7).
/// All-static ABI layout, so the vault recovers it with a single abi.decode.
struct PublicValues {
    bytes32 domainTag;       // K("unicity-bridge-return:v1")
    bytes32 configHash;
    bytes32 trustBaseHash;   // H(RootTrustBase) — allow-listed, not recomputed
    bytes32 spentRootOld;
    bytes32 spentRootNew;
    bytes32 returnRoot;
    bytes32 lockRefRoot;
    uint32  batchSize;
    uint256 totalAmount;
}

/// @title BridgeEncoding
/// @notice Pure keccak/ABI derivations the vault recomputes (00 §1). Exposed as
///         a library so the Solidity conformance harness can exercise each
///         derivation directly against the `protocol/vectors` fixtures.
library BridgeEncoding {
    // Domain separators (00 §2, §3, §7). `configHash`/`lockDigest` carry the
    // domain as a dynamic ABI `string` (matches protocol/vectors/gen abi.rs);
    // `domainTag` is keccak over the raw UTF-8 bytes.
    string internal constant DOMAIN_CONFIG = "unicity-bridge-return-config:v1";
    string internal constant DOMAIN_LOCK   = "unicity-bridge-lock:v1";
    string internal constant DOMAIN_RETURN = "unicity-bridge-return:v1";

    /// `configHash = K(abi.encode("...config:v1", fields...))` (00 §2).
    function configHash(BridgeConfig memory c) internal pure returns (bytes32) {
        return keccak256(abi.encode(
            DOMAIN_CONFIG,
            c.sourceChainId,
            c.vault,
            c.asset,
            c.tokenType,
            c.coinId,
            c.reasonTag,
            c.lockDomain,
            c.nullifierDomain
        ));
    }

    /// `lockDigest = K(abi.encode("...lock:v1", fields...))` (00 §3). `nonce` and
    /// `amount` are uint256; the deployment-constant fields come from the config.
    function lockDigest(
        uint64  sourceChainId,
        address vault,
        uint256 nonce,
        address asset,
        bytes32 tokenType,
        bytes32 coinId,
        uint256 amount,
        bytes32 unicityTokenId,
        bytes32 recipientCommitment
    ) internal pure returns (bytes32) {
        return keccak256(abi.encode(
            DOMAIN_LOCK,
            sourceChainId,
            vault,
            nonce,
            asset,
            tokenType,
            coinId,
            amount,
            unicityTokenId,
            recipientCommitment
        ));
    }

    /// `domainTag = K("unicity-bridge-return:v1")` over raw bytes (00 §7).
    function domainTag() internal pure returns (bytes32) {
        return keccak256(bytes(DOMAIN_RETURN));
    }

    /// `returnRoot = K( concat of fixed-width abi.encode(leaf) )` in the given
    /// (submission) order (00 §7). Each leaf encodes to 6 static 32-byte words.
    function returnRoot(ReturnLeaf[] calldata leaves) internal pure returns (bytes32) {
        bytes memory buf;
        for (uint256 i = 0; i < leaves.length; i++) {
            ReturnLeaf calldata l = leaves[i];
            buf = bytes.concat(buf, abi.encode(
                l.nullifier, l.recipient, l.amount, l.feeRecipient, l.feeAmount, l.deadline
            ));
        }
        return keccak256(buf);
    }

    /// `lockRefRoot = K( concat of fixed-width abi.encode(ref) )` (00 §7). The
    /// refs must already be sorted by nonce with duplicates removed; the vault
    /// enforces that ordering independently before calling this.
    function lockRefRoot(SourceLockRef[] calldata refs) internal pure returns (bytes32) {
        bytes memory buf;
        for (uint256 i = 0; i < refs.length; i++) {
            buf = bytes.concat(buf, abi.encode(refs[i].nonce, refs[i].digest));
        }
        return keccak256(buf);
    }
}
