use damysus_base::enclave::EnclaveError;
use enclave_result::impl_error_and_result;

#[derive(Debug)]
pub enum MyPlatformError {
    InvalidPeerId,
}

pub struct NotInitializedError;

impl_error_and_result! {
    InitError,
    InitResult,
    [AlreadyInitialized]
}

impl_error_and_result! {
    GetPublicKeyError,
    GetPublicKeyResult,
    [NotInitialized]
}

impl From<NotInitializedError> for GetPublicKeyError {
    fn from(NotInitializedError: NotInitializedError) -> Self {
        Self::NotInitialized
    }
}

impl_error_and_result! {
    AddPeerError,
    AddPeerResult,
    [
        InvalidPeerId,
        NotInitialized,
        DoubleKeyReceived
    ]
}

impl From<NotInitializedError> for AddPeerError {
    fn from(NotInitializedError: NotInitializedError) -> Self {
        Self::NotInitialized
    }
}

impl_error_and_result! {
    AccumulateError,
    AccumulateResult,
    [
        Bincode,
        EnclaveError,
        InvalidPeerId,
        NotInitialized,
        WrongBlockHashLength,
        SignedCommitmentBufferToSmall
    ]
}

impl From<NotInitializedError> for AccumulateError {
    fn from(NotInitializedError: NotInitializedError) -> Self {
        Self::NotInitialized
    }
}

impl From<Box<bincode::ErrorKind>> for AccumulateError {
    fn from(_: Box<bincode::ErrorKind>) -> Self {
        Self::Bincode
    }
}

impl From<MyPlatformError> for AccumulateError {
    fn from(error: MyPlatformError) -> Self {
        match error {
            MyPlatformError::InvalidPeerId => Self::InvalidPeerId,
        }
    }
}

impl From<EnclaveError<MyPlatformError>> for AccumulateError {
    fn from(error: EnclaveError<MyPlatformError>) -> Self {
        match error {
            EnclaveError::PlatformError(e) => e.into(),
            _ => Self::EnclaveError,
        }
    }
}

impl_error_and_result! {
    PrepareError,
    PrepareResult,
    [
        Bincode,
        EnclaveError,
        InvalidPeerId,
        NotInitialized,
        WrongBlockHashLength,
        SignedCommitmentBufferToSmall
    ]
}

impl From<NotInitializedError> for PrepareError {
    fn from(NotInitializedError: NotInitializedError) -> Self {
        Self::NotInitialized
    }
}

impl From<Box<bincode::ErrorKind>> for PrepareError {
    fn from(_: Box<bincode::ErrorKind>) -> Self {
        Self::Bincode
    }
}

impl From<MyPlatformError> for PrepareError {
    fn from(error: MyPlatformError) -> Self {
        match error {
            MyPlatformError::InvalidPeerId => Self::InvalidPeerId,
        }
    }
}

impl From<EnclaveError<MyPlatformError>> for PrepareError {
    fn from(error: EnclaveError<MyPlatformError>) -> Self {
        match error {
            EnclaveError::PlatformError(e) => e.into(),
            _ => Self::EnclaveError,
        }
    }
}

impl_error_and_result! {
    ReSignLastPrepareError,
    ReSignLastPrepareResult,
    [
        Bincode,
        EnclaveError,
        InvalidPeerId,
        NotInitialized,
        SignedCommitmentBufferToSmall
    ]
}

impl From<NotInitializedError> for ReSignLastPrepareError {
    fn from(NotInitializedError: NotInitializedError) -> Self {
        Self::NotInitialized
    }
}

impl From<Box<bincode::ErrorKind>> for ReSignLastPrepareError {
    fn from(_: Box<bincode::ErrorKind>) -> Self {
        Self::Bincode
    }
}

impl From<MyPlatformError> for ReSignLastPrepareError {
    fn from(error: MyPlatformError) -> Self {
        match error {
            MyPlatformError::InvalidPeerId => Self::InvalidPeerId,
        }
    }
}

impl From<EnclaveError<MyPlatformError>> for ReSignLastPrepareError {
    fn from(error: EnclaveError<MyPlatformError>) -> Self {
        match error {
            EnclaveError::PlatformError(e) => e.into(),
            _ => Self::EnclaveError,
        }
    }
}
