require("@nomicfoundation/hardhat-toolbox");

/**
 * Tron's TVM is EVM-compatible at the Solidity level for this contract, so we
 * compile/test with Hardhat (EVM) here. For on-chain deployment to Tron Nile /
 * mainnet, compile the same source with tronbox or solc and deploy via TronWeb;
 * the bytecode/ABI are equivalent for the logic exercised by these tests.
 *
 * @type import('hardhat/config').HardhatUserConfig
 */
const optimizer = { optimizer: { enabled: true, runs: 200 } };
// viaIR resolves "stack too deep" in UnicityBridgeVault (the explicit-gas
// transfer stipend pushed it over Solidity's stack-slot limit) without
// changing contract semantics. Scoped to 0.8.24 (the vault); the 0.8.20
// verifier subtree is unaffected and not being redeployed.
const optimizerViaIR = { optimizer: { enabled: true, runs: 200 }, viaIR: true };

module.exports = {
  solidity: {
    // 0.8.24 for the bridge contracts; 0.8.20 for the vendored SP1 v6.1.0
    // Groth16 verifier (its generated source pins `pragma solidity 0.8.20`).
    // The verifier subtree is self-contained (the vault depends only on
    // IProofVerifier), so the two compilers never link together.
    compilers: [
      { version: "0.8.24", settings: optimizerViaIR },
      { version: "0.8.20", settings: optimizer },
    ],
    overrides: {
      "contracts/verifier/ISP1Verifier.sol": { version: "0.8.20", settings: optimizer },
      "contracts/verifier/v6.1.0/Groth16Verifier.sol": { version: "0.8.20", settings: optimizer },
      "contracts/verifier/v6.1.0/SP1VerifierGroth16.sol": { version: "0.8.20", settings: optimizer },
    },
  },
};
