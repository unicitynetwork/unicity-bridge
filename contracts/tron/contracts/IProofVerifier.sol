// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

/// @title IProofVerifier
/// @notice On-chain succinct-proof verifier the vault calls before settling a
///         batch (01-source-chain-contracts.md "Proof verification on-chain").
///         Shaped after SP1's `ISP1Verifier`: the vault commits to one immutable
///         `VKEY` (circuit version) and one `publicValues` blob (00 §7), and the
///         verifier checks the Groth16 proof over bn254.
/// @dev    MUST revert on an invalid proof (it does not return a bool). A circuit
///         upgrade is a new verifier address + `VKEY`, i.e. a new vault (or a
///         governed swap behind a timelock at M5).
interface IProofVerifier {
    /// @param vkey         The verification key for the committed circuit version.
    /// @param publicValues The ABI-encoded `PublicValues` the circuit committed.
    /// @param proof        The Groth16 proof bytes.
    function verifyProof(
        bytes32 vkey,
        bytes calldata publicValues,
        bytes calldata proof
    ) external view;
}
