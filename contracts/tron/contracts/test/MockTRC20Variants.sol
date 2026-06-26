// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

/// @dev A TRC20 that returns NOTHING from transfer/transferFrom (like real USDT
///      on Tron). Exercises the vault's no-return safe-transfer path.
contract MockNoReturnTRC20 {
    mapping(address => uint256) public balanceOf;
    mapping(address => mapping(address => uint256)) public allowance;

    function mint(address to, uint256 value) external {
        balanceOf[to] += value;
    }

    function approve(address spender, uint256 value) external {
        allowance[msg.sender][spender] = value;
    }

    function transfer(address to, uint256 value) external {
        _transfer(msg.sender, to, value);
    }

    function transferFrom(address from, address to, uint256 value) external {
        uint256 allowed = allowance[from][msg.sender];
        require(allowed >= value, "no-return: allowance");
        if (allowed != type(uint256).max) allowance[from][msg.sender] = allowed - value;
        _transfer(from, to, value);
    }

    function _transfer(address from, address to, uint256 value) private {
        require(balanceOf[from] >= value, "no-return: balance");
        balanceOf[from] -= value;
        balanceOf[to] += value;
    }
}

interface ILock {
    function lock(uint256 amount, bytes32 unicityTokenId, bytes32 recipientCommitment)
        external
        returns (uint256);
}

/// @dev A TRC20 whose first `transfer` reenters the vault's guarded `lock()`.
///      `lock` shares the same reentrancy guard as `fulfillBatch`, so the nested
///      call MUST revert with "vault: reentrancy". We catch that revert and set
///      a flag (so the outer settlement still completes) — the test asserts the
///      flag, proving the guard tripped. `lock`'s args are static and always
///      decode, so the call reaches the guard before any other check.
contract MockReentrantTRC20 {
    mapping(address => uint256) public balanceOf;
    mapping(address => mapping(address => uint256)) public allowance;
    address public vault;
    bool public reentryBlocked;
    bool private attacking;

    function setVault(address v) external {
        vault = v;
    }

    function mint(address to, uint256 value) external {
        balanceOf[to] += value;
    }

    function approve(address spender, uint256 value) external returns (bool) {
        allowance[msg.sender][spender] = value;
        return true;
    }

    function transfer(address to, uint256 value) external returns (bool) {
        if (vault != address(0) && !attacking) {
            attacking = true;
            try ILock(vault).lock(1, bytes32(uint256(1)), bytes32(uint256(1))) returns (uint256) {
                reentryBlocked = false;
            } catch {
                reentryBlocked = true;
            }
            attacking = false;
        }
        _transfer(msg.sender, to, value);
        return true;
    }

    function transferFrom(address from, address to, uint256 value) external returns (bool) {
        uint256 allowed = allowance[from][msg.sender];
        require(allowed >= value, "reentrant: allowance");
        if (allowed != type(uint256).max) allowance[from][msg.sender] = allowed - value;
        _transfer(from, to, value);
        return true;
    }

    function _transfer(address from, address to, uint256 value) private {
        require(balanceOf[from] >= value, "reentrant: balance");
        balanceOf[from] -= value;
        balanceOf[to] += value;
    }
}
