pub mod noop;
pub mod signature;
pub mod test;

use core::fmt;
use std::{
    fmt::Debug,
    ops::{Add, AddAssign},
};

use serde::{Deserialize, Serialize};
use shared_ids::hashbar::Hashbar;
pub use shared_ids::ReplicaId;
use thiserror::Error;

/// A USIG signature counter value
#[repr(transparent)]
#[derive(
    Serialize, Deserialize, Debug, Clone, Copy, Ord, Eq, PartialEq, PartialOrd, Default, Hash,
)]
pub struct Count(pub u64);

impl fmt::Display for Count {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "({0})", self.0)
    }
}

impl Hashbar for Count {
    fn hash<H: hashbar::Hasher>(&self, hasher: &mut H) {
        hasher.update(&self.0.to_le_bytes());
    }
}

#[derive(Error, Debug, Clone)]
pub enum UsigError {
    #[error("unknown id '{0:?}'")]
    UnknownId(ReplicaId),

    #[error("invalid signature")]
    InvalidSignature,

    #[error("remote attestation failed")]
    RemoteAttestationFailed,

    #[error("signing failed")]
    SigningFailed,
}

impl Add<u64> for Count {
    type Output = Count;

    fn add(self, rhs: u64) -> Self::Output {
        Self(self.0 + rhs)
    }
}

impl AddAssign<u64> for Count {
    fn add_assign(&mut self, rhs: u64) {
        self.0 += rhs
    }
}

/// This trait allows the retrieval of the counter value from a USIG signature
pub trait Counter {
    /// Get the counter value of this USIG signature
    fn counter(&self) -> Count;
}

/// The main trait that defines a usig service
pub trait Usig {
    /// The type of the USIG signature
    ///
    /// The access to the count is provided by the counter trait
    type Signature: Debug + Counter;

    /// The type of a remote attestation
    type Attestation: Debug + Send + 'static;

    fn new() -> Self
    where
        Self: Sized;

    /// Sign a message with a USIG signature
    fn sign(&mut self, message: impl AsRef<[u8]>) -> Result<Self::Signature, UsigError>;

    /// Get the remote attestation of this USIG
    fn attest(&mut self) -> Result<Self::Attestation, UsigError>;

    /// Verify the USIG signature of a message
    ///
    /// Only work if the attestation for the usig is was previously loaded
    fn verify(
        &self,
        remote_usig_id: ReplicaId,
        message: impl AsRef<[u8]>,
        signature: &Self::Signature,
    ) -> Result<(), UsigError>;

    /// Load a remote attestation of a remote USIG and add the remote party
    fn add_remote_party(
        &mut self,
        remote_usig_id: ReplicaId,
        attestation: Self::Attestation,
    ) -> bool;

    /// Type of the signing half
    type SignHalf: SignHalf<Signature = Self::Signature, Attestation = Self::Attestation>;

    /// Type of the verifying half
    type VerifyHalf: VerifyHalf<Signature = Self::Signature, Attestation = Self::Attestation>;

    /// Split USIG into signing and verifying half's
    fn split(self) -> (Self::SignHalf, Self::VerifyHalf);
}

/// The signing half of a split usig service
pub trait SignHalf {
    /// The type of the USIG signature
    ///
    /// The access to the count is provided by the counter trait
    type Signature: Counter;

    /// The type of a remote attestation
    type Attestation;

    /// Sign a message with a USIG signature
    fn sign(&mut self, message: impl AsRef<[u8]>) -> Result<Self::Signature, UsigError>;

    /// Get the remote attestation of this USIG
    fn attest(&mut self) -> Result<Self::Attestation, UsigError>;
}

/// The verifying half of a split usig service
pub trait VerifyHalf {
    /// The type of the USIG signature
    ///
    /// The access to the count is provided by the counter trait
    type Signature: Counter;

    /// The type of a remote attestation
    type Attestation;

    /// Verify the USIG signature of a message
    ///
    /// Only work if the attestation for the usig is was previously loaded
    fn verify(
        &self,
        remote_usig_id: ReplicaId,
        message: impl AsRef<[u8]>,
        signature: &Self::Signature,
    ) -> Result<(), UsigError>;

    /// Load a remote attestation of a remote USIG and add the remote party
    fn add_remote_party(
        &mut self,
        remote_usig_id: ReplicaId,
        attestation: Self::Attestation,
    ) -> bool;
}
