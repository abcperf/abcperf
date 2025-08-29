use nxbft_base::enclave::{Enclave, HandshakeResult, InitHandshakeResult};
use nxbft_base::vertex::{SignableVertex, SignedVertex};
use nxbft_base::Round;
use nxbft_tee_shared::{
    CompletePeerHandshakeError, EnclaveAutomataFFIError, InitPeerHandshakeError, IsReadyResult,
};
use rangemap::RangeInclusiveMap;
use remote_attestation::include_enclave;
use remote_attestation_shared::types::PubKey;
use sgx_crypto::ecc::EcPublicKey;
use sgx_types::types::{Ec256PublicKey, Ec256Signature};
use sgx_urts::enclave::SgxEnclave;
use sgx_utils::{SgxError, SgxResult, SgxStatusExt};
use shared_ids::map::ReplicaMap;
use shared_ids::ReplicaId;
use thiserror::Error;
use tracing::error;
use types::Signature;
pub use types::{Attestation, Handshake};

use std::mem;
use std::ops::Deref;

#[derive(Error, Debug)]
pub enum EnclaveError {
    #[error("sgx error: {0}")]
    SgxError(#[from] SgxError),
    #[error("enclave automata ffi error: {0}")]
    EnclaveAutomataFFI(#[from] EnclaveAutomataFFIError),
    #[error("init peer handshake error: {0}")]
    InitPeerHandshake(#[from] InitPeerHandshakeError),
    #[error("complete peer handshake error: {0}")]
    CompletePeerHandshake(#[from] CompletePeerHandshakeError),
    #[error("bincode error: {0}")]
    Bincode(#[from] Box<bincode::ErrorKind>),
    #[error("unknown peer id: {0}")]
    UnknownPeerId(ReplicaId),
}

mod ecall;
#[cfg(test)]
mod tests;
mod types;

include_enclave!("../../nxbft-tee-enclave/nxbft_tee.signed.so");

#[derive(Debug)]
pub(crate) struct NxbftEnclave {
    enclave: SgxEnclave,
}

impl Deref for NxbftEnclave {
    type Target = SgxEnclave;

    fn deref(&self) -> &Self::Target {
        &self.enclave
    }
}

impl NxbftEnclave {
    fn new() -> SgxResult<Self> {
        // call sgx_create_enclave to initialize an enclave instance
        // Debug Support: set 2nd parameter to 1
        let debug = true;
        let enclave = SgxEnclave::create(ENCLAVE_PATH.as_path(), debug).into_result()?;

        Ok(Self { enclave })
    }
}

/// The struct that defines a UsigTEE
#[derive(Debug)]
pub struct NxbftTEE {
    enclave: NxbftEnclave,
    pubkeys: ReplicaMap<Option<(PubKey, RangeInclusiveMap<u64, PubKey>)>>,

    n: u64,
}

impl Enclave for NxbftTEE {
    type Signature = Signature;

    type EnclaveError = EnclaveError;

    type Attestation = Attestation;

    type Export = (
        Box<[u8]>,
        ReplicaMap<Option<(PubKey, RangeInclusiveMap<u64, PubKey>)>>,
    );

    type Handshake = Handshake;

    fn new(n: u64) -> Self {
        assert!(n >= 3, "Minimum number of peers is three");

        let enclave = NxbftEnclave::new().unwrap();

        Self {
            enclave,
            pubkeys: ReplicaMap::new(n),
            n,
        }
    }

    fn get_n(&self) -> u64 {
        self.n
    }

    fn init(&mut self) -> Result<(), Self::EnclaveError> {
        self.enclave.init(self.n)??;

        Ok(())
    }

    fn get_attestation_certificate(&mut self) -> Result<Self::Attestation, Self::EnclaveError> {
        let (pubkey, local_ra) = self.enclave.attest(4096)??;

        Ok(Attestation::new(pubkey, local_ra))
    }

    fn init_peer_handshake(
        &mut self,
        peer_id: ReplicaId,
        attestation: Self::Attestation,
    ) -> InitHandshakeResult<Self::Handshake, Self::EnclaveError> {
        let result = self.enclave.init_peer_handshake(
            peer_id.as_u64(),
            attestation.pub_key.into(),
            &attestation.intel_report,
            4096,
        );

        let result = match result {
            Ok(r) => r,
            Err(e) => return InitHandshakeResult::EnclaveError(e.into()),
        };

        let (proposed_secret, proposed_secret_mac) = match result {
            Ok(t) => t,
            Err(InitPeerHandshakeError::DuplicateHello) => {
                return InitHandshakeResult::DuplicateHello
            }
            Err(InitPeerHandshakeError::InvalidAttestation) => {
                return InitHandshakeResult::InvalidAttestation
            }
            Err(e) => return InitHandshakeResult::EnclaveError(e.into()),
        };

        let x = &mut self.pubkeys[peer_id];

        assert!(x.is_none());
        *x = Some((attestation.pub_key, RangeInclusiveMap::new()));

        InitHandshakeResult::Ok(Handshake {
            proposed_secret,
            proposed_secret_mac,
        })
    }

    fn complete_peer_handshake(
        &mut self,
        peer_id: ReplicaId,
        handshake: &Self::Handshake,
    ) -> HandshakeResult<Self::EnclaveError> {
        let Handshake {
            proposed_secret,
            proposed_secret_mac,
        } = handshake;

        let result = self.enclave.complete_peer_handshake(
            peer_id.as_u64(),
            proposed_secret,
            *proposed_secret_mac,
        );

        let result = match result {
            Ok(r) => r,
            Err(e) => return HandshakeResult::EnclaveError(e.into()),
        };

        match result {
            Ok(()) => HandshakeResult::Ok,
            Err(CompletePeerHandshakeError::MissingAttestation) => {
                HandshakeResult::MissingAttestation
            }
            Err(CompletePeerHandshakeError::DuplicateReply) => HandshakeResult::DuplicateReply,
            Err(CompletePeerHandshakeError::InvalidHandshakeEncryption) => {
                HandshakeResult::InvalidHandshakeEncryption
            }
            Err(e) => HandshakeResult::EnclaveError(e.into()),
        }
    }

    fn sign(
        &mut self,
        payload: SignableVertex,
    ) -> Result<(Self::Signature, u64), Self::EnclaveError> {
        let message = payload.to_hash();

        let (signature, counter) = self.enclave.sign(message.as_ref())??;

        Ok((Signature(signature.into()), counter))
    }

    fn toss(
        &mut self,
        proof: Vec<SignedVertex<Self::Signature>>,
    ) -> Result<ReplicaId, Self::EnclaveError> {
        let proof = bincode::serialize(&proof)?;

        let peer_id = self.enclave.toss(&proof)??;

        Ok(ReplicaId::from_u64(peer_id))
    }

    fn verify(
        &self,
        peer_id: ReplicaId,
        counter: u64,
        payload: SignableVertex,
        signature: &Self::Signature,
    ) -> Result<bool, Self::EnclaveError> {
        let signature = signature.0;
        let message = payload.to_hash();

        let mut data = Vec::new();
        data.extend_from_slice(message.as_ref());
        data.extend_from_slice(&counter.to_be_bytes());

        let (key, old_keys) = self.pubkeys[peer_id]
            .as_ref()
            .ok_or(EnclaveError::UnknownPeerId(peer_id))?;

        let pubkey = *old_keys
            .get(&payload.address().round().as_u64())
            .unwrap_or(key);

        let valid = EcPublicKey::from(Ec256PublicKey::from(pubkey))
            .verify(data.as_slice(), &Ec256Signature::from(signature).into())
            .into_result()?;

        Ok(valid)
    }

    fn is_ready(&self) -> bool {
        match self.enclave.is_ready().unwrap() {
            IsReadyResult::Ready => true,
            IsReadyResult::NotReady => false,
        }
    }

    fn verify_attestation(
        &self,
        _attestation: &Self::Attestation,
    ) -> Result<bool, Self::EnclaveError> {
        // TODO RA
        Ok(true)
    }

    fn export(&self) -> Result<Self::Export, Self::EnclaveError> {
        let export = self.enclave.export(4096)??;

        Ok((export, self.pubkeys.clone()))
    }

    fn recover(&mut self, export: Self::Export) -> Result<(), Self::EnclaveError> {
        self.enclave.recover(&export.0)??;

        self.pubkeys = export.1;

        Ok(())
    }

    fn replace_peer(
        &mut self,
        peer_id: ReplicaId,
        attestation: &Self::Attestation,
        max_round: Round,
    ) -> Result<bool, Self::EnclaveError> {
        let err = self.enclave.replace_peer(
            peer_id.as_u64(),
            attestation.pub_key.into(),
            &attestation.intel_report,
            max_round.as_u64(),
        )?;

        if err == Err(EnclaveAutomataFFIError::NotReady) {
            return Ok(false);
        }

        err?;

        let (key, old_keys) = self.pubkeys[peer_id].as_mut().unwrap();

        let old = mem::replace(key, attestation.pub_key);
        old_keys.insert(
            old_keys
                .last_range_value()
                .map(|(r, _)| *r.end())
                .unwrap_or(0)..=max_round.as_u64(),
            old,
        );

        Ok(true)
    }
}
