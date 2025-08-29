use enclave_result::{impl_error_and_result, impl_result};
use nxbft_base::enclave::EnclaveAutomataError;
use thiserror::Error;

impl_error_and_result! {
    InitPeerHandshakeError,
    InitPeerHandshakeResult,
    [
        DuplicateHello,
        InvalidAttestation,
        InvalidPeerId,
        NotReady,
        NotEnoughValidVertices,
        InvalidVertexRound,
        AesInitFailed,
        AesEncryptFailed,
        RngInitFailed,
        SigningFailed,
        EcVerifyFailed,
        InvalidHandshake,
        InvalidHandshakeLength
    ]
}

impl From<EnclaveAutomataError<SgxEnclaveTrustedError>> for InitPeerHandshakeError {
    fn from(e: EnclaveAutomataError<SgxEnclaveTrustedError>) -> Self {
        match e {
            EnclaveAutomataError::InvalidPeerId => Self::InvalidPeerId,
            EnclaveAutomataError::NotReady => Self::NotReady,
            EnclaveAutomataError::NotEnoughValidVertices => Self::NotEnoughValidVertices,
            EnclaveAutomataError::InvalidVertexRound => Self::InvalidVertexRound,
            EnclaveAutomataError::EnclaveTrusted(e) => match e {
                SgxEnclaveTrustedError::AesInitFailed => Self::AesInitFailed,
                SgxEnclaveTrustedError::AesEncryptFailed => Self::AesEncryptFailed,
                SgxEnclaveTrustedError::RngInitFailed => Self::RngInitFailed,
                SgxEnclaveTrustedError::SigningFailed => Self::SigningFailed,
                SgxEnclaveTrustedError::EcVerifyFailed => Self::EcVerifyFailed,
                SgxEnclaveTrustedError::InvalidHandshake => Self::InvalidHandshake,
                SgxEnclaveTrustedError::InvalidHandshakeLength => Self::InvalidHandshakeLength,
            },
        }
    }
}

impl_error_and_result! {
    CompletePeerHandshakeError,
    CompletePeerHandshakeResult,
    [
        MissingAttestation,
        DuplicateReply,
        InvalidHandshakeEncryption,
        InvalidPeerId,
        NotReady,
        NotEnoughValidVertices,
        InvalidVertexRound,
        AesInitFailed,
        AesEncryptFailed,
        RngInitFailed,
        SigningFailed,
        EcVerifyFailed,
        InvalidHandshake,
        InvalidHandshakeLength
    ]
}
impl From<EnclaveAutomataError<SgxEnclaveTrustedError>> for CompletePeerHandshakeError {
    fn from(e: EnclaveAutomataError<SgxEnclaveTrustedError>) -> Self {
        match e {
            EnclaveAutomataError::InvalidPeerId => Self::InvalidPeerId,
            EnclaveAutomataError::NotReady => Self::NotReady,
            EnclaveAutomataError::NotEnoughValidVertices => Self::NotEnoughValidVertices,
            EnclaveAutomataError::InvalidVertexRound => Self::InvalidVertexRound,
            EnclaveAutomataError::EnclaveTrusted(e) => match e {
                SgxEnclaveTrustedError::AesInitFailed => Self::AesInitFailed,
                SgxEnclaveTrustedError::AesEncryptFailed => Self::AesEncryptFailed,
                SgxEnclaveTrustedError::RngInitFailed => Self::RngInitFailed,
                SgxEnclaveTrustedError::SigningFailed => Self::SigningFailed,
                SgxEnclaveTrustedError::EcVerifyFailed => Self::EcVerifyFailed,
                SgxEnclaveTrustedError::InvalidHandshake => Self::InvalidHandshake,
                SgxEnclaveTrustedError::InvalidHandshakeLength => Self::InvalidHandshakeLength,
            },
        }
    }
}

impl_result! {
    IsReadyResult,
    [
        Ready,
        NotReady
    ]
}

impl_error_and_result! {
    EnclaveAutomataFFIError,
    EnclaveAutomataFFIResult,
    [
        InvalidPeerId,
        NotReady,
        NotEnoughValidVertices,
        InvalidVertexRound,
        InvalidHandshakeEncryption,
        AesInitFailed,
        AesEncryptFailed,
        RngInitFailed,
        SigningFailed,
        EcVerifyFailed,
        Bincode,
        ExportBufferToSmall,
        NotInitialized,
        AlreadyInitialized,
        KeyGenFailed,
        InvalidHandshake,
        InvalidHandshakeLength
    ]
}

impl From<Box<bincode::ErrorKind>> for EnclaveAutomataFFIError {
    fn from(_: Box<bincode::ErrorKind>) -> Self {
        Self::Bincode
    }
}

impl From<EnclaveAutomataError<SgxEnclaveTrustedError>> for EnclaveAutomataFFIError {
    fn from(e: EnclaveAutomataError<SgxEnclaveTrustedError>) -> Self {
        match e {
            EnclaveAutomataError::InvalidPeerId => Self::InvalidPeerId,
            EnclaveAutomataError::NotReady => Self::NotReady,
            EnclaveAutomataError::NotEnoughValidVertices => Self::NotEnoughValidVertices,
            EnclaveAutomataError::InvalidVertexRound => Self::InvalidVertexRound,
            EnclaveAutomataError::EnclaveTrusted(e) => match e {
                SgxEnclaveTrustedError::AesInitFailed => Self::AesInitFailed,
                SgxEnclaveTrustedError::AesEncryptFailed => Self::AesEncryptFailed,
                SgxEnclaveTrustedError::RngInitFailed => Self::RngInitFailed,
                SgxEnclaveTrustedError::SigningFailed => Self::SigningFailed,
                SgxEnclaveTrustedError::EcVerifyFailed => Self::EcVerifyFailed,
                SgxEnclaveTrustedError::InvalidHandshake => Self::InvalidHandshake,
                SgxEnclaveTrustedError::InvalidHandshakeLength => Self::InvalidHandshakeLength,
            },
        }
    }
}

#[derive(Debug, Error)]
pub enum SgxEnclaveTrustedError {
    #[error("invalid handshake")]
    InvalidHandshake,
    #[error("invalid handshake length")]
    InvalidHandshakeLength,
    #[error("AES initialization failed")]
    AesInitFailed,
    #[error("AES encryption failed")]
    AesEncryptFailed,
    #[error("Rng initialization failed")]
    RngInitFailed,
    #[error("creating signature failed")]
    SigningFailed,
    #[error("ec verify failed")]
    EcVerifyFailed,
}
