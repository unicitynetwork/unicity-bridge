// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

/// @dev Minimal TRC20/ERC20 surface used by the bridge. Tether USDT on Tron
///      returns false even on success; we require only no-revert in {_safeTransferFrom}.
interface ITRC20 {
    function transferFrom(address from, address to, uint256 value) external returns (bool);
    function transfer(address to, uint256 value) external returns (bool);
}

/// @title UnicityLock
/// @notice Locks USDT (TRC20) on Tron to back a bridged token mint on Unicity.
///         Each lock commits to the exact Unicity `tokenId` and to a
///         `recipientCommitment = SHA256(recipient predicate)`, so the resulting
///         lock proof (see docs/bridge/MINT_REASON.md) can fund exactly one
///         Unicity token, owned only by the designated recipient — even though
///         minting on Unicity is permissionless.
/// @dev    Verifiers read the emitted {Lock} event over a Tron RPC node; nothing
///         here trusts an operator. The `unlock` (bridge-back) path is wired but
///         inert until a burn-proof verifier is set (see docs/bridge/BRIDGE_BACK.md).
contract UnicityLock {
    struct LockRecord {
        address from;
        uint256 amount;
        bytes32 unicityTokenId;
        bytes32 recipientCommitment;
        bool withdrawn;
    }

    /// @notice The TRC20 asset this contract bridges (USDT).
    ITRC20 public immutable asset;

    /// @notice Admin able to configure the bridge-back verifier and rotate it.
    address public admin;

    /// @notice Contract authorised to settle bridge-back withdrawals. address(0)
    ///         => bridge-back disabled (deposits/bridge-in are unaffected).
    address public burnProofVerifier;

    /// @notice Auto-incrementing id for each lock; also emitted in {Lock}.
    uint256 public nextNonce;

    /// @notice nonce => lock record.
    mapping(uint256 => LockRecord) public locks;

    /// @notice Guards against funding two Unicity tokens from one deposit.
    ///         (Unicity's one-genesis-per-tokenId rule is the primary guard;
    ///         this is defence-in-depth so the same tokenId can't even be locked twice.)
    mapping(bytes32 => bool) public tokenIdUsed;

    uint256 private _entered;

    event Lock(
        uint256 indexed nonce,
        address indexed from,
        uint256 amount,
        bytes32 unicityTokenId,
        bytes32 recipientCommitment
    );
    event Unlock(uint256 indexed nonce, address indexed to, uint256 amount);
    event BurnProofVerifierUpdated(address indexed verifier);
    event AdminTransferred(address indexed previousAdmin, address indexed newAdmin);

    modifier onlyAdmin() {
        require(msg.sender == admin, "UnicityLock: not admin");
        _;
    }

    modifier nonReentrant() {
        require(_entered == 0, "UnicityLock: reentrancy");
        _entered = 1;
        _;
        _entered = 0;
    }

    constructor(ITRC20 asset_, address admin_) {
        require(address(asset_) != address(0), "UnicityLock: zero asset");
        require(admin_ != address(0), "UnicityLock: zero admin");
        asset = asset_;
        admin = admin_;
    }

    // ---------------------------------------------------------------------
    // Bridge-in: lock USDT, commit to the Unicity token it will fund
    // ---------------------------------------------------------------------

    /// @notice Lock `amount` USDT and bind it to a future Unicity mint.
    /// @param amount               USDT amount (6 decimals). Must be > 0.
    /// @param unicityTokenId       The Unicity TokenId (32 bytes) that may be minted.
    /// @param recipientCommitment  SHA256 of the recipient's encoded predicate.
    /// @return nonce               The lock id, echoed in the {Lock} event.
    /// @dev Caller must `approve` this contract for `amount` first. Uses
    ///      checks-effects-interactions; state is finalised before the token pull.
    function lock(
        uint256 amount,
        bytes32 unicityTokenId,
        bytes32 recipientCommitment
    ) external nonReentrant returns (uint256 nonce) {
        require(amount > 0, "UnicityLock: zero amount");
        require(unicityTokenId != bytes32(0), "UnicityLock: zero tokenId");
        require(recipientCommitment != bytes32(0), "UnicityLock: zero recipient");
        require(!tokenIdUsed[unicityTokenId], "UnicityLock: tokenId already locked");

        tokenIdUsed[unicityTokenId] = true;
        nonce = nextNonce++;
        locks[nonce] = LockRecord({
            from: msg.sender,
            amount: amount,
            unicityTokenId: unicityTokenId,
            recipientCommitment: recipientCommitment,
            withdrawn: false
        });

        emit Lock(nonce, msg.sender, amount, unicityTokenId, recipientCommitment);

        _safeTransferFrom(msg.sender, address(this), amount);
    }

    // ---------------------------------------------------------------------
    // Bridge-back: release USDT against a proven Unicity burn (wired, inert)
    // ---------------------------------------------------------------------

    /// @notice Settle a bridge-back withdrawal. Callable only by the configured
    ///         burn-proof verifier, which is responsible for proving that the
    ///         Unicity token for `nonce` was burned with a reason committing to
    ///         `to`/`amount`. Inert until a verifier is set.
    /// @dev    The proof system itself (committee multisig / aggregation / zk)
    ///         lives in `burnProofVerifier`; see docs/bridge/BRIDGE_BACK.md.
    function unlock(uint256 nonce, address to, uint256 amount) external nonReentrant {
        require(burnProofVerifier != address(0), "UnicityLock: bridge-back disabled");
        require(msg.sender == burnProofVerifier, "UnicityLock: not verifier");
        LockRecord storage rec = locks[nonce];
        require(rec.amount != 0, "UnicityLock: unknown nonce");
        require(!rec.withdrawn, "UnicityLock: already withdrawn");
        require(amount == rec.amount, "UnicityLock: amount mismatch");

        rec.withdrawn = true;
        emit Unlock(nonce, to, amount);
        _safeTransfer(to, amount);
    }

    // ---------------------------------------------------------------------
    // Admin
    // ---------------------------------------------------------------------

    function setBurnProofVerifier(address verifier) external onlyAdmin {
        burnProofVerifier = verifier;
        emit BurnProofVerifierUpdated(verifier);
    }

    function transferAdmin(address newAdmin) external onlyAdmin {
        require(newAdmin != address(0), "UnicityLock: zero admin");
        emit AdminTransferred(admin, newAdmin);
        admin = newAdmin;
    }

    // ---------------------------------------------------------------------
    // Token transfer helpers (tolerate no-return and false-returning TRC20s)
    // ---------------------------------------------------------------------

    function _safeTransferFrom(address from, address to, uint256 value) private {
        (bool ok,) =
            address(asset).call(abi.encodeWithSelector(ITRC20.transferFrom.selector, from, to, value));
        require(ok, "UnicityLock: transferFrom failed");
    }

    function _safeTransfer(address to, uint256 value) private {
        (bool ok,) =
            address(asset).call(abi.encodeWithSelector(ITRC20.transfer.selector, to, value));
        // Tether USDT on Tron returns false even on success; failures revert.
        require(ok, "UnicityLock: transfer failed");
    }
}
