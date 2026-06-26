// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {IProofVerifier} from "../IProofVerifier.sol";

/// @dev Always accepts. Lets the M2 settlement logic (root transition, lock-ref
///      checks, fee/deadline, atomicity, events) be tested without proving.
contract MockProofVerifier is IProofVerifier {
    function verifyProof(bytes32, bytes calldata, bytes calldata) external view {}
}

/// @dev Always rejects — for the "proof rejected" path.
contract RevertingProofVerifier is IProofVerifier {
    function verifyProof(bytes32, bytes calldata, bytes calldata) external view {
        revert("mock: proof rejected");
    }
}
