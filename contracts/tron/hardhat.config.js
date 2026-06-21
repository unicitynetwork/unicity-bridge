require("@nomicfoundation/hardhat-toolbox");

/**
 * Tron's TVM is EVM-compatible at the Solidity level for this contract, so we
 * compile/test with Hardhat (EVM) here. For on-chain deployment to Tron Nile /
 * mainnet, compile the same source with tronbox or solc and deploy via TronWeb;
 * the bytecode/ABI are equivalent for the logic exercised by these tests.
 *
 * @type import('hardhat/config').HardhatUserConfig
 */
module.exports = {
  solidity: {
    version: "0.8.24",
    settings: {
      optimizer: { enabled: true, runs: 200 },
    },
  },
};
