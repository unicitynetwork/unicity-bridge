// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {BridgeConfig, ReturnLeaf, SourceLockRef, PublicValues, BridgeEncoding} from "./BridgeEncoding.sol";
import {IProofVerifier} from "./IProofVerifier.sol";

/// @dev Minimal TRC20/ERC20 surface. Tether USDT on Tron (mainnet and Nile
///      testnet) returns false even on success. We require only that the call
///      does not revert; see {_safeTransfer}/{_safeTransferFrom}.
interface ITRC20 {
    function transferFrom(address from, address to, uint256 value) external returns (bool);
    function transfer(address to, uint256 value) external returns (bool);
}

/// @title UnicityBridgeVault
/// @notice The single greenfield bridge vault per ZK_BACK3: it holds custody of
///         the external asset, records the bridge-in `lockDigest`, and settles
///         the bridge-back return path (proof verification + one replay
///         accumulator root + fee/deadline settlement).
/// @dev    Supersedes `UnicityLock`'s `unlock`/`withdrawn` model. Releases are
///         keyed by **nullifier**, not lock nonce (a deposit split on Unicity
///         returns as many independent tokens). Conforms to
///         docs/bridge/dev-plan/00-interop-contract.md §1–3, §7, §9 and
///         01-source-chain-contracts.md. The proof system is pluggable behind
///         {IProofVerifier} (a mock at M2, real SP1 Groth16 at M3).
contract UnicityBridgeVault {
    // ----- immutables, all derived from one BridgeConfig (00 §2, ZK_BACK3 §2.3)
    IProofVerifier public immutable verifier;
    bytes32 public immutable VKEY;
    bytes32 public immutable DOMAIN_TAG;   // K("unicity-bridge-return:v1")
    bytes32 public immutable CONFIG_HASH;  // K(abi.encode(config)) (00 §2)
    ITRC20  public immutable ASSET;        // = config.asset; cannot diverge from CONFIG_HASH

    // config fields retained to recompute lockDigest at lock time (00 §3)
    uint64  public immutable sourceChainId;
    address public immutable assetAddr;
    bytes32 public immutable tokenType;
    bytes32 public immutable coinId;

    /// @notice Empty replay-accumulator root (00 §6). All-zero placeholder; the
    ///         real empty-tree root is pinned by `bridge-vectors/accumulator` at
    ///         M2 and must equal this constant.
    bytes32 public constant EMPTY_TREE_ROOT = bytes32(0);

    /// @notice Admin able to manage the trust-base allow-list (timelock at M5).
    address public admin;

    /// @notice Bridge-in: nonce => lockDigest (00 §3), stored at lock time.
    mapping(uint256 => bytes32) public lockDigest;
    /// @notice Auto-incrementing lock id, echoed in {Lock}.
    uint256 public nextNonce;
    /// @notice Defence-in-depth: the same Unicity tokenId can't be locked twice.
    mapping(bytes32 => bool) public tokenIdUsed;

    /// @notice Allow-listed `trustBaseHash` set (validator-set epochs; 00 §8).
    mapping(bytes32 => bool) public trustBaseAllowed;
    /// @notice The replay accumulator root; advances once per settled batch.
    bytes32 public spentRoot;

    /// @notice Pull-payment mode (set at deploy). When true, {fulfillBatch}
    ///         credits {owed} instead of transferring, so a single reverting or
    ///         blocklisted recipient can't brick the whole batch (ZK_BACK3 §9
    ///         batch-atomicity note). Recipients claim via {withdraw}. Recommended
    ///         for assets with transfer hooks/blocklists (e.g. USDT); push mode is
    ///         fine for plain assets and keeps single-tx UX.
    bool public immutable PULL_PAYMENTS;
    /// @notice Accrued pull-payment balances (used only when PULL_PAYMENTS).
    mapping(address => uint256) public owed;

    uint256 private _entered;

    event Lock(
        uint256 indexed nonce,
        address indexed from,
        uint256 amount,
        bytes32 unicityTokenId,
        bytes32 recipientCommitment
    );
    event BatchFulfilled(
        bytes32 indexed spentRootOld,
        bytes32 indexed spentRootNew,
        uint32 batchSize,
        uint256 totalAmount
    );
    event Released(
        bytes32 indexed nullifier,
        address indexed recipient,
        uint256 amount,
        address feeRecipient,
        uint256 feeAmount,
        uint64 deadline
    );
    event TrustBaseAllowedUpdated(bytes32 indexed trustBaseHash, bool allowed);
    event AdminTransferred(address indexed previousAdmin, address indexed newAdmin);
    /// @notice Emitted when an account claims its accrued pull-payment balance.
    event Withdrawn(address indexed account, uint256 amount);

    modifier onlyAdmin() {
        require(msg.sender == admin, "vault: not admin");
        _;
    }

    modifier nonReentrant() {
        require(_entered == 0, "vault: reentrancy");
        _entered = 1;
        _;
        _entered = 0;
    }

    /// @param cfg       Deployment config. `cfg.vault` is IGNORED and overwritten
    ///                  with `address(this)`: the vault stamps its own address into
    ///                  CONFIG_HASH so the binding holds without predicting the
    ///                  deploy address. This is required on Tron, where the contract
    ///                  address derives from the deployment txID (which covers the
    ///                  constructor args), making a `cfg.vault == address(this)`
    ///                  precondition circular/unsatisfiable. On EVM the stamped
    ///                  value equals the CREATE address, so CONFIG_HASH is unchanged.
    ///                  The off-chain prover/wallet MUST set its `BridgeConfig.vault`
    ///                  to the deployed address so its `configHash` matches.
    /// @param verifier_ The proof verifier (mock at M2, SP1 Groth16 at M3).
    /// @param vkey      The circuit verification key (placeholder until M3).
    /// @param admin_    Trust-base allow-list manager.
    /// @param pullPayments If true, settle by crediting {owed} (claim via
    ///                  {withdraw}) instead of pushing transfers — see {PULL_PAYMENTS}.
    constructor(
        BridgeConfig memory cfg,
        IProofVerifier verifier_,
        bytes32 vkey,
        address admin_,
        bool pullPayments
    ) {
        cfg.vault = address(this);
        require(cfg.asset != address(0), "vault: zero asset");
        require(address(verifier_) != address(0), "vault: zero verifier");
        require(admin_ != address(0), "vault: zero admin");

        verifier = verifier_;
        VKEY = vkey;
        CONFIG_HASH = BridgeEncoding.configHash(cfg);
        DOMAIN_TAG = BridgeEncoding.domainTag();
        ASSET = ITRC20(cfg.asset);
        sourceChainId = cfg.sourceChainId;
        assetAddr = cfg.asset;
        tokenType = cfg.tokenType;
        coinId = cfg.coinId;

        admin = admin_;
        spentRoot = EMPTY_TREE_ROOT;
        PULL_PAYMENTS = pullPayments;
    }

    // ---------------------------------------------------------------------
    // Bridge-in: lock the asset, store lockDigest, emit the full fields
    // ---------------------------------------------------------------------

    /// @notice Lock `amount` of the asset and bind it to a future Unicity mint.
    /// @param amount               Asset amount (token decimals). Must be > 0.
    /// @param unicityTokenId       The Unicity TokenId (32 bytes) this funds.
    /// @param recipientCommitment  SHA256 of the recipient's encoded predicate.
    /// @return nonce               The lock id, echoed in {Lock}.
    /// @dev Caller must `approve` this contract first. Checks-effects-interactions:
    ///      `lockDigest[nonce]` is finalised before the token pull. The `Lock`
    ///      event keeps emitting full fields for the TS verifier and explorers.
    function lock(
        uint256 amount,
        bytes32 unicityTokenId,
        bytes32 recipientCommitment
    ) external nonReentrant returns (uint256 nonce) {
        require(amount > 0, "vault: zero amount");
        require(unicityTokenId != bytes32(0), "vault: zero tokenId");
        require(recipientCommitment != bytes32(0), "vault: zero recipient");
        require(!tokenIdUsed[unicityTokenId], "vault: tokenId already locked");

        tokenIdUsed[unicityTokenId] = true;
        nonce = nextNonce++;
        lockDigest[nonce] = BridgeEncoding.lockDigest(
            sourceChainId,
            address(this),
            nonce,
            assetAddr,
            tokenType,
            coinId,
            amount,
            unicityTokenId,
            recipientCommitment
        );

        emit Lock(nonce, msg.sender, amount, unicityTokenId, recipientCommitment);
        _safeTransferFrom(msg.sender, address(this), amount);
    }

    // ---------------------------------------------------------------------
    // Bridge-back: verify one batch proof and settle every leaf
    // ---------------------------------------------------------------------

    /// @notice Verify a return-batch proof and release the asset for each leaf.
    /// @dev ZK_BACK3 §9 / 01 §"The vault". All validation precedes any transfer;
    ///      the accumulator root advances (replay guard consumed) before payouts.
    /// @param publicValues ABI-encoded {PublicValues} the circuit committed (00 §7).
    /// @param proof        The Groth16 proof over `publicValues`.
    /// @param leaves       The settlement leaves, in the batch's submission order.
    /// @param lockRefs     The deduplicated source-lock refs, sorted by nonce.
    function fulfillBatch(
        bytes calldata publicValues,
        bytes calldata proof,
        ReturnLeaf[] calldata leaves,
        SourceLockRef[] calldata lockRefs
    ) external nonReentrant {
        // 1. proof (reverts on failure)
        verifier.verifyProof(VKEY, publicValues, proof);

        // 2. decode the public statement and check it against this vault
        PublicValues memory pv = abi.decode(publicValues, (PublicValues));
        require(pv.domainTag == DOMAIN_TAG, "vault: bad domain");
        require(pv.configHash == CONFIG_HASH, "vault: bad config");
        require(trustBaseAllowed[pv.trustBaseHash], "vault: trust base not allowed");
        require(pv.spentRootOld == spentRoot, "vault: stale root");
        require(leaves.length > 0, "vault: empty batch");
        require(pv.batchSize == leaves.length, "vault: batch size mismatch");
        require(pv.returnRoot == BridgeEncoding.returnRoot(leaves), "vault: return root mismatch");
        require(pv.lockRefRoot == BridgeEncoding.lockRefRoot(lockRefs), "vault: lock ref root mismatch");

        // 3. lock refs sorted-unique + bound to stored digests (one SLOAD each)
        for (uint256 i = 0; i < lockRefs.length; i++) {
            if (i > 0) {
                require(lockRefs[i].nonce > lockRefs[i - 1].nonce, "vault: lock refs unsorted");
            }
            require(lockDigest[lockRefs[i].nonce] == lockRefs[i].digest, "vault: lock digest mismatch");
        }

        // 4. value conservation: per-leaf fee <= amount, and sum == totalAmount
        uint256 total;
        for (uint256 i = 0; i < leaves.length; i++) {
            require(leaves[i].feeAmount <= leaves[i].amount, "vault: fee exceeds amount");
            total += leaves[i].amount;
        }
        require(total == pv.totalAmount, "vault: total amount mismatch");

        // 5. consume the replay guard (effects before interactions)
        spentRoot = pv.spentRootNew;
        emit BatchFulfilled(pv.spentRootOld, pv.spentRootNew, pv.batchSize, pv.totalAmount);

        // 6. settle each leaf; deadline gates the FEE only, principal is always paid.
        //    In pull mode we only credit {owed} (no external calls), so a hostile
        //    recipient can revert at most its own {withdraw}, never the batch.
        for (uint256 i = 0; i < leaves.length; i++) {
            ReturnLeaf calldata l = leaves[i];
            uint256 fee =
                (l.feeRecipient != address(0) && block.timestamp <= l.deadline) ? l.feeAmount : 0;
            uint256 principal = l.amount - fee;
            if (PULL_PAYMENTS) {
                owed[l.recipient] += principal;
                if (fee > 0) {
                    owed[l.feeRecipient] += fee;
                }
            } else {
                _safeTransfer(l.recipient, principal);
                if (fee > 0) {
                    _safeTransfer(l.feeRecipient, fee);
                }
            }
            emit Released(l.nullifier, l.recipient, l.amount, l.feeRecipient, fee, l.deadline);
        }
    }

    /// @notice Claim the caller's accrued pull-payment balance (PULL_PAYMENTS mode).
    /// @dev Checks-effects-interactions: zero the balance before the transfer, and
    ///      nonReentrant for defence in depth. A recipient that can't receive the
    ///      asset only blocks its own withdrawal — never others' settlements.
    /// @return amount The amount transferred to the caller.
    function withdraw() external nonReentrant returns (uint256 amount) {
        amount = owed[msg.sender];
        require(amount > 0, "vault: nothing owed");
        owed[msg.sender] = 0;
        emit Withdrawn(msg.sender, amount);
        _safeTransfer(msg.sender, amount);
    }

    // ---------------------------------------------------------------------
    // Admin (trust-base allow-list; timelocked governance at M5)
    // ---------------------------------------------------------------------

    function setTrustBaseAllowed(bytes32 trustBaseHash, bool allowed) external onlyAdmin {
        trustBaseAllowed[trustBaseHash] = allowed;
        emit TrustBaseAllowedUpdated(trustBaseHash, allowed);
    }

    function transferAdmin(address newAdmin) external onlyAdmin {
        require(newAdmin != address(0), "vault: zero admin");
        emit AdminTransferred(admin, newAdmin);
        admin = newAdmin;
    }

    // ---------------------------------------------------------------------
    // Token transfer helpers (tolerate no-return + false-returning TRC20s)
    // ---------------------------------------------------------------------

    // TVM (unlike EVM) does not reliably forward "all remaining energy" to a
    // bare `.call(...)` with no explicit gas — nested contract-to-contract
    // calls made this way can starve even when the outer transaction has
    // plenty of energy headroom (observed: fulfillBatch's transfer to USDT
    // failed with ~268k energy free, while the identical transfer succeeds
    // standalone on 14.6k). Pin an explicit, generous stipend.
    uint256 private constant TRANSFER_GAS_STIPEND = 200_000;

    function _safeTransferFrom(address from, address to, uint256 value) private {
        (bool ok,) = address(ASSET).call{gas: TRANSFER_GAS_STIPEND}(
            abi.encodeWithSelector(ITRC20.transferFrom.selector, from, to, value)
        );
        require(ok, "vault: transferFrom failed");
    }

    function _safeTransfer(address to, uint256 value) private {
        (bool ok,) = address(ASSET).call{gas: TRANSFER_GAS_STIPEND}(
            abi.encodeWithSelector(ITRC20.transfer.selector, to, value)
        );
        // Tether USDT on Tron (mainnet + Nile faucet TXYZopYRdj2D9XRtbG411XZZ3kM5VkAeBf)
        // returns false even on success; real failures revert. Require only no-revert.
        require(ok, "vault: transfer failed");
    }
}
