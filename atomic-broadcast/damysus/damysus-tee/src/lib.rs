use std::ops::Deref;

use damysus_base::{
    enclave::{BlockHash, CommitmentHash, Enclave, EnclaveError, Justification, SignedCommitment},
    view::View,
    Parent,
};
use damysus_tee_shared::{
    AccumulateError, AddPeerError, GetPublicKeyError, PrepareError, ReSignLastPrepareError,
};
use remote_attestation::include_enclave;
use remote_attestation_shared::types::{PubKey, RawSignature};
use sgx_crypto::ecc::EcPublicKey;
use sgx_types::{
    error::SgxResult,
    types::{Ec256PublicKey, Ec256Signature},
};
use sgx_urts::enclave::SgxEnclave;
use sgx_utils::{SgxError, SgxStatusExt};
use shared_ids::{map::ReplicaMap, ReplicaId};
use thiserror::Error;

mod ecall;

include_enclave!("../../damysus-tee-enclave/damysus_tee.signed.so");

#[derive(Debug)]
pub(crate) struct DamysusEnclave {
    enclave: SgxEnclave,
}

impl Deref for DamysusEnclave {
    type Target = SgxEnclave;

    fn deref(&self) -> &Self::Target {
        &self.enclave
    }
}

impl DamysusEnclave {
    fn new() -> SgxResult<Self> {
        // call sgx_create_enclave to initialize an enclave instance
        // Debug Support: set 2nd parameter to 1
        let debug = true;
        let enclave = SgxEnclave::create(ENCLAVE_PATH.as_path(), debug).into_result()?;

        Ok(Self { enclave })
    }
}

#[derive(Debug)]
pub struct DamysusTEE {
    enclave: DamysusEnclave,
    pub_keys: ReplicaMap<Option<PubKey>>,
    my_pub_key: PubKey,
}

#[derive(Debug, Error)]
pub enum MyError {
    #[error("sgx error: {0}")]
    Sgx(#[from] SgxError),
    #[error("get_public_key error: {0}")]
    GetPublicKey(#[from] GetPublicKeyError),
    #[error("add_peer error: {0}")]
    AddPeer(#[from] AddPeerError),
    #[error("accumulate error: {0}")]
    Accumulate(#[from] AccumulateError),
    #[error("prepare error: {0}")]
    Prepare(#[from] PrepareError),
    #[error("re_sign_last_prepared error: {0}")]
    ReSignLastPrepare(#[from] ReSignLastPrepareError),
    #[error("bincode error: {0}")]
    Bincode(#[from] bincode::Error),
}

impl Enclave for DamysusTEE {
    type PlatformError = MyError;
    type PublicKey = PubKey;
    type Signature = RawSignature;

    fn new(me: ReplicaId, n: u64, quorum: u64) -> Self {
        let enclave = DamysusEnclave::new().unwrap();

        enclave.init(me.as_u64(), n, quorum).unwrap().unwrap();

        let mut pub_keys = ReplicaMap::new(n);

        let my_pub_key = enclave.get_public_key().unwrap().unwrap().into();

        pub_keys[me] = Some(my_pub_key);

        Self {
            enclave,
            pub_keys,
            my_pub_key,
        }
    }

    fn get_public_key(&self) -> Result<Self::PublicKey, EnclaveError<Self::PlatformError>> {
        Ok(self.my_pub_key)
    }

    fn add_peer(
        &mut self,
        id: ReplicaId,
        key: Self::PublicKey,
    ) -> Result<(), EnclaveError<Self::PlatformError>> {
        let mut inner = || {
            self.enclave.add_peer(id.as_u64(), key.into())??;

            self.pub_keys[id] = Some(key);

            Ok(())
        };

        inner().map_err(EnclaveError::PlatformError)
    }

    fn accumulate(
        &mut self,
        commitments: &[SignedCommitment<Self::Signature>],
    ) -> Result<SignedCommitment<Self::Signature>, EnclaveError<Self::PlatformError>> {
        let inner = || {
            let commitments = bincode::serialize(commitments)?;

            let signed_commitment = self.enclave.accumulate(&commitments, 4096)??;

            let signed_commitment = bincode::deserialize(&signed_commitment)?;

            Ok(signed_commitment)
        };

        inner().map_err(EnclaveError::PlatformError)
    }

    fn prepare(
        &mut self,
        block_view: View,
        block_hash: BlockHash,
        block_justification: Justification<Self::Signature>,
        block_parent: Parent,
        justification_block_hash: BlockHash,
    ) -> Result<SignedCommitment<Self::Signature>, EnclaveError<Self::PlatformError>> {
        let inner = || {
            let block_justification = bincode::serialize(&block_justification)?;
            let block_parent = bincode::serialize(&block_parent)?;

            let signed_commitment = self.enclave.prepare(
                block_view.as_u64(),
                block_hash.as_ref(),
                &block_justification,
                &block_parent,
                justification_block_hash.as_ref(),
                4096,
            )??;

            let signed_commitment = bincode::deserialize(&signed_commitment)?;

            Ok(signed_commitment)
        };

        inner().map_err(EnclaveError::PlatformError)
    }

    fn re_sign_last_prepared(
        &mut self,
    ) -> Result<SignedCommitment<Self::Signature>, EnclaveError<Self::PlatformError>> {
        let inner = || {
            let signed_commitment = self.enclave.re_sign_last_prepared(4096)??;

            let signed_commitment = bincode::deserialize(&signed_commitment)?;

            Ok(signed_commitment)
        };

        inner().map_err(EnclaveError::PlatformError)
    }

    fn verify_untrusted(
        &self,
        creator: ReplicaId,
        hash: &CommitmentHash,
        signature: &Self::Signature,
    ) -> bool {
        let pub_key = self.pub_keys[creator].unwrap();

        let valid = EcPublicKey::from(Ec256PublicKey::from(pub_key))
            .verify(hash.as_ref(), &Ec256Signature::from(*signature).into())
            .unwrap();

        valid
    }
}
