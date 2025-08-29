use core::panic;
use remote_attestation::include_enclave;
use remote_attestation_shared::types::PubKey;
use sgx_crypto::ecc::EcPublicKey;
use sgx_types::types::{Ec256PublicKey, Ec256Signature};
use sgx_urts::enclave::SgxEnclave;
use sgx_utils::{SgxResult, SgxStatusExt};
use shared_ids::ReplicaId;
use tracing::{debug, error};
use types::{Attestation, Signature};

use std::collections::HashMap;

use usig::{SignHalf, Usig, UsigError, VerifyHalf};

mod ecall;
mod types;

include_enclave!("../../usig-tee-enclave/usig_tee.signed.so");

/// The signing half of a split usig service
#[derive(Default, Debug)]
pub struct UsigTEESignHalf {
    enclave: SgxEnclave,
}

impl SignHalf for UsigTEESignHalf {
    type Signature = Signature;
    type Attestation = Attestation;

    /// Sign a message with a USIG signature
    fn sign(&mut self, message: impl AsRef<[u8]>) -> Result<Self::Signature, UsigError> {
        let message = message.as_ref();

        let res_sign = ecall::sign(self.enclave.eid(), message);

        match res_sign {
            Ok((signature, counter)) => Ok(Signature::new(signature, counter)),
            Err(_) => Err(UsigError::SigningFailed),
        }
    }

    /// Get the remote attestation of this USIG
    fn attest(&mut self) -> Result<Self::Attestation, UsigError> {
        let mut pubkey = Ec256PublicKey::default();
        let mut counter = 0;
        let mut cert_der_ffi = [0u8; 4096];
        let mut cert_der_ffi_len = 0;
        let cert_der = cert_der_ffi.as_mut_slice();

        let res_attest = ecall::attest(
            self.enclave.eid(),
            &mut pubkey,
            &mut counter,
            cert_der,
            &mut cert_der_ffi_len,
        );
        let cert_der_ffi = &cert_der_ffi[..cert_der_ffi_len];

        match res_attest {
            Ok(()) => {
                let cert_der_ffi = cert_der_ffi.to_vec();
                Ok(Attestation::new(pubkey, counter, cert_der_ffi))
            }
            Err(err) => {
                error!("[-] ECALL Enclave Failed {}!", err);
                Err(UsigError::RemoteAttestationFailed)
            }
        }
    }
}

/// The verifying half of a split usig service
#[derive(Default, Debug)]
pub struct UsigTEEVerifyHalf {
    pubkeys: HashMap<ReplicaId, PubKey>,
}

impl VerifyHalf for UsigTEEVerifyHalf {
    /// The type of the USIG signature
    ///
    /// The access to the count is provided by the counter trait
    type Signature = Signature;

    /// The type of a remote attestation
    type Attestation = Attestation;

    /// Verify the USIG signature of a message
    ///
    /// Only works if the attestation for the usig was previously loaded
    fn verify(
        &self,
        remote_usig_id: ReplicaId,
        message: impl AsRef<[u8]>,
        signature: &Self::Signature,
    ) -> Result<(), UsigError> {
        let message = message.as_ref();

        let mut data = Vec::<u8>::new();
        data.extend_from_slice(message);
        data.extend_from_slice(&signature.counter.to_be_bytes());

        let pubkey = match self.pubkeys.get(&remote_usig_id) {
            Some(v) => *v,
            None => {
                return Err(UsigError::UnknownId(remote_usig_id));
            }
        };

        let res = EcPublicKey::from(Ec256PublicKey::from(pubkey))
            .verify(
                data.as_slice(),
                &Ec256Signature::from(signature.signature).into(),
            )
            .unwrap();

        if !res {
            return Err(UsigError::InvalidSignature);
        }

        Ok(())
    }

    /// Load a remote attestation of a remote USIG and add the remote party
    fn add_remote_party(
        &mut self,
        remote_usig_id: ReplicaId,
        attestation: Self::Attestation,
    ) -> bool {
        // TODO RA verify
        self.pubkeys.insert(remote_usig_id, attestation.pub_key);
        true
    }
}

/// The struct that defines a UsigTEE
#[derive(Default, Debug)]
pub struct UsigTEE {
    sign_half: UsigTEESignHalf,
    verify_half: UsigTEEVerifyHalf,
}

impl UsigTEE {
    fn init_enc() -> SgxResult<SgxEnclave> {
        let enclave = match init_enclave() {
            Ok(r) => {
                debug!("Init Enclave Successful eid={}", r.eid());
                r
            }
            Err(e) => {
                error!("Init Enclave Failed: {}", e.as_str());
                return Err(e);
            }
        };
        Ok(enclave)
    }

    /// The init function which has to be called for a correct initialization of a UsigTEE
    pub fn init_panic() -> Self {
        match UsigTEE::init_enc() {
            Ok(e) => {
                let mut usig_tee = UsigTEE::default();
                usig_tee.sign_half.enclave = e;
                usig_tee
            }
            Err(e) => panic!("Initialization failed: {e}"),
        }
    }
}

impl Usig for UsigTEE {
    /// The type of a remote attestation
    type Attestation = Attestation;

    /// The type of the USIG signature
    ///
    /// The access to the count is provided by the counter trait
    type Signature = Signature;

    /// Type of the signing half
    type SignHalf = UsigTEESignHalf;

    /// Type of the verifying half
    type VerifyHalf = UsigTEEVerifyHalf;

    /// Sign a message with a USIG signature
    fn sign(&mut self, message: impl AsRef<[u8]>) -> Result<Self::Signature, UsigError> {
        self.sign_half.sign(message)
    }

    /// Get the remote attestation of this USIG
    fn attest(&mut self) -> Result<Self::Attestation, UsigError> {
        self.sign_half.attest()
    }

    /// Verify the USIG signature of a message
    ///
    /// Only works if the attestation for the usig was previously loaded
    fn verify(
        &self,
        remote_usig_id: ReplicaId,
        message: impl AsRef<[u8]>,
        signature: &Self::Signature,
    ) -> Result<(), UsigError> {
        self.verify_half.verify(remote_usig_id, message, signature)
    }

    /// Load a remote attestation of a remote USIG and add the remote party
    fn add_remote_party(
        &mut self,
        remote_usig_id: ReplicaId,
        attestation: Self::Attestation,
    ) -> bool {
        self.verify_half
            .add_remote_party(remote_usig_id, attestation)
    }

    /// Split USIG into signing and verifying half's
    fn split(self) -> (Self::SignHalf, Self::VerifyHalf) {
        (self.sign_half, self.verify_half)
    }

    fn new() -> Self
    where
        Self: Sized,
    {
        UsigTEE::init_panic()
    }
}

// Initializes the enclave
fn init_enclave() -> SgxResult<SgxEnclave> {
    // call sgx_create_enclave to initialize an enclave instance
    // Debug Support: set 2nd parameter to 1
    let debug = true;
    SgxEnclave::create(ENCLAVE_PATH.as_path(), debug).into_result()
}

#[cfg(test)]
mod tests {
    use sgx_types::types::Ec256Signature;
    use usig::tests;

    use super::*;

    tests!(UsigTEE::init_panic());

    #[test]
    fn complete_once() {
        let mut usig_tee = UsigTEE::init_panic();
        let attestation = usig_tee.attest().unwrap();
        usig_tee.add_remote_party(ReplicaId::from_u64(0), attestation);
        let sig = usig_tee.sign("message").unwrap();
        usig_tee
            .verify(ReplicaId::from_u64(0), "message", &sig)
            .unwrap();
    }

    #[test]
    fn wrong_remote_party() {
        let mut usig_tee = UsigTEE::init_panic();
        let attestation = usig_tee.attest().unwrap();
        usig_tee.add_remote_party(ReplicaId::from_u64(0), attestation);
        let sig = usig_tee.sign("message").unwrap();
        assert!(usig_tee
            .verify(ReplicaId::from_u64(1), "message", &sig)
            .is_err())
    }

    #[test]
    fn signed_msg_differs_from_verified() {
        let mut usig_tee = UsigTEE::init_panic();
        let attestation = usig_tee.attest().unwrap();
        usig_tee.add_remote_party(ReplicaId::from_u64(0), attestation);
        let sig = usig_tee.sign("message1").unwrap();
        assert!(usig_tee
            .verify(ReplicaId::from_u64(0), "message2", &sig)
            .is_err())
    }

    #[test]
    fn wrong_signature() {
        let mut usig_tee = UsigTEE::init_panic();
        let attestation = usig_tee.attest().unwrap();
        usig_tee.add_remote_party(ReplicaId::from_u64(0), attestation);
        usig_tee.sign("message").unwrap(); // correct signature
        let sig2 = Signature {
            signature: Ec256Signature::default().into(),
            counter: 0,
        }; // wrong signature
        assert!(usig_tee
            .verify(ReplicaId::from_u64(0), "message", &sig2)
            .is_err())
    }

    #[test]
    fn no_eq_signatures() {
        let mut usig_tee = UsigTEE::init_panic();
        let attestation = usig_tee.attest().unwrap();
        usig_tee.add_remote_party(ReplicaId::from_u64(0), attestation);
        let sig1 = usig_tee.sign("message").unwrap();
        let sig2 = usig_tee.sign("message").unwrap();
        assert_ne!(sig1, sig2);
    }
}
