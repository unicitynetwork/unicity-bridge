use core::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BridgeExtError {
    InvalidTrustBase,
    NetworkMismatch,
    InvalidMintLockScript,
    InclusionCertificateMissing,
    CertificationDataMissing,
    CertificationDataMismatch,
    TransactionHashMismatch,
    PathInvalid,
    ShardMismatch,
    SealNetworkMismatch,
    SealRootMismatch,
    QuorumNotMet,
    NotAuthenticated,
    Genesis,
    Transfer,
    BridgeLockJustificationMalformed,
    BridgeLockAddressLength,
    BridgeLockBytes32Length,
    BridgeLockConfigMismatch,
    BridgeLockTokenTypeMismatch,
    BridgeLockPaymentDataMissing,
    BridgeLockPaymentDataMalformed,
    BridgeLockCoinMissing,
    BridgeLockAmountMismatch,
    SplitMalformed,
    SplitNetworkMismatch,
    SplitBurnTransferMissing,
    SplitManifestMissing,
    SplitManifestMalformed,
    SplitBurnPredicateMismatch,
    SplitTokenTypeMismatch,
    SplitSourcePaymentDataMissing,
    SplitManifestLengthMismatch,
    SplitProofCountMismatch,
    SplitSourceAssetMissing,
    SplitAllocationProofInvalid,
    SplitSourceAmountMismatch,
}

pub type Result<T> = core::result::Result<T, BridgeExtError>;

impl fmt::Display for BridgeExtError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{self:?}")
    }
}
