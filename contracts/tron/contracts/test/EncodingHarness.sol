// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {
    BridgeConfig,
    ReturnLeaf,
    SourceLockRef,
    PublicValues,
    BridgeEncoding
} from "../BridgeEncoding.sol";

/// @dev Public wrappers over the `internal` {BridgeEncoding} library so the
///      Solidity conformance tests can recompute each derivation directly and
///      assert equality with the `bridge-vectors` fixtures (00 §10).
contract EncodingHarness {
    function configHash(BridgeConfig calldata c) external pure returns (bytes32) {
        return BridgeEncoding.configHash(c);
    }

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
    ) external pure returns (bytes32) {
        return BridgeEncoding.lockDigest(
            sourceChainId, vault, nonce, asset, tokenType, coinId,
            amount, unicityTokenId, recipientCommitment
        );
    }

    function domainTag() external pure returns (bytes32) {
        return BridgeEncoding.domainTag();
    }

    function returnRoot(ReturnLeaf[] calldata leaves) external pure returns (bytes32) {
        return BridgeEncoding.returnRoot(leaves);
    }

    function lockRefRoot(SourceLockRef[] calldata refs) external pure returns (bytes32) {
        return BridgeEncoding.lockRefRoot(refs);
    }

    function encodePublicValues(PublicValues calldata pv) external pure returns (bytes memory) {
        return abi.encode(pv);
    }

    function decodePublicValues(bytes calldata b) external pure returns (PublicValues memory) {
        return abi.decode(b, (PublicValues));
    }
}
