#![cfg_attr(not(target_vendor = "teaclave"), no_std)]
#![cfg_attr(target_vendor = "teaclave", feature(rustc_private))]

#[cfg(not(target_vendor = "teaclave"))]
#[macro_use]
extern crate sgx_tstd as std;
extern crate sgx_types;

use damysus_base::enclave::{
    BlockHash, CommitmentHash, EnclaveAutomata, EnclaveError, EnclaveTrusted, Justification,
    SignedCommitment,
};
use damysus_base::view::View;
use damysus_base::Parent;
use damysus_tee_shared::MyPlatformError;
use damysus_tee_shared::NotInitializedError;
use damysus_tee_shared::{
    AccumulateError, AccumulateResult, AddPeerError, AddPeerResult, GetPublicKeyError,
    GetPublicKeyResult, InitError, InitResult, PrepareError, PrepareResult, ReSignLastPrepareError,
    ReSignLastPrepareResult,
};
use once_cell::sync::OnceCell;
use remote_attestation_shared::types::{PubKey, RawSignature};
use sgx_crypto::ecc::EcKeyPair;
use sgx_crypto::ecc::EcPublicKey;
use sgx_types::types::Ec256PublicKey;
use sgx_types::types::Ec256Signature;
use shared_ids::map::ReplicaMap;
use shared_ids::ReplicaId;
use std::ops::DerefMut;
use std::sync::Mutex;

#[derive(Debug)]
pub struct NoopEnclave {}

type PlatformError = MyPlatformError;
type PublicKey = PubKey;
type Signature = RawSignature;

impl InitializedState {
    fn new(me: ReplicaId, n: u64, quorum: u64) -> Self {
        let key_pair = EcKeyPair::create().unwrap();
        let mut pub_keys = ReplicaMap::new(n);

        Self {
            enclave_automata: EnclaveAutomata::new(
                me,
                n,
                quorum,
                EnclaveTrustedImpl {
                    key_pair,
                    pub_keys,
                    n,
                },
            ),
        }
    }

    fn get_public_key(&self) -> PublicKey {
        self.enclave_automata
            .get_enclave()
            .key_pair
            .public_key()
            .public_key()
            .into()
    }

    fn add_peer(&mut self, id: ReplicaId, key: PublicKey) -> Result<(), AddPeerError> {
        if id.as_u64() >= self.enclave_automata.get_enclave().n {
            return Err(AddPeerError::InvalidPeerId);
        }
        match &mut self.enclave_automata.get_enclave_mut().pub_keys[id] {
            Some(_) => Err(AddPeerError::DoubleKeyReceived),
            entry @ None => {
                *entry = Some(key);
                Ok(())
            }
        }
    }

    fn accumulate(
        &mut self,
        commitments: &[SignedCommitment<Signature>],
    ) -> Result<SignedCommitment<Signature>, AccumulateError> {
        let signed_commitment = self.enclave_automata.accumulate(commitments)?;

        Ok(signed_commitment)
    }

    fn prepare(
        &mut self,
        block_view: View,
        block_hash: BlockHash,
        block_justification: Justification<Signature>,
        block_parent: Parent,
        justification_block_hash: BlockHash,
    ) -> Result<SignedCommitment<Signature>, PrepareError> {
        let signed_commitment = self.enclave_automata.prepare(
            block_view,
            block_hash,
            block_justification,
            block_parent,
            justification_block_hash,
        )?;

        Ok(signed_commitment)
    }

    fn re_sign_last_prepared(
        &mut self,
    ) -> Result<SignedCommitment<Signature>, ReSignLastPrepareError> {
        let signed_commitment = self.enclave_automata.re_sign_last_prepared()?;

        Ok(signed_commitment)
    }
}

#[derive(Debug)]
struct EnclaveTrustedImpl {
    key_pair: EcKeyPair,
    pub_keys: ReplicaMap<Option<PubKey>>,
    n: u64,
}

impl EnclaveTrusted for EnclaveTrustedImpl {
    type PlatformError = PlatformError;
    type Signature = Signature;

    fn sign(
        &self,
        hash: &CommitmentHash,
    ) -> Result<Self::Signature, EnclaveError<Self::PlatformError>> {
        let sig = self
            .key_pair
            .private_key()
            .sign(hash.as_ref())
            .unwrap()
            .signature();

        Ok(sig.into())
    }

    fn verify(
        &self,
        creator: ReplicaId,
        hash: &CommitmentHash,
        signature: &Self::Signature,
    ) -> Result<bool, EnclaveError<Self::PlatformError>> {
        if creator.as_u64() >= self.n {
            return Err(EnclaveError::PlatformError(MyPlatformError::InvalidPeerId));
        }

        let pub_key = self.pub_keys[creator]
            .ok_or(EnclaveError::PlatformError(MyPlatformError::InvalidPeerId))?;

        let valid = EcPublicKey::from(Ec256PublicKey::from(pub_key))
            .verify(hash.as_ref(), &Ec256Signature::from(*signature).into())
            .unwrap();

        Ok(valid)
    }
}

struct InitializedState {
    enclave_automata: EnclaveAutomata<EnclaveTrustedImpl>,
}

enum SharedState {
    Uninitialized,
    Initialized(InitializedState),
}

impl SharedState {
    fn init(&mut self, me: ReplicaId, n: u64, quorum: u64) -> Result<(), InitError> {
        match self {
            Self::Initialized(_) => Err(InitError::AlreadyInitialized),
            Self::Uninitialized => {
                *self = Self::Initialized(InitializedState::new(me, n, quorum));
                Ok(())
            }
        }
    }

    fn initialized(&mut self) -> Result<&mut InitializedState, NotInitializedError> {
        match self {
            Self::Initialized(init) => Ok(init),
            Self::Uninitialized => Err(NotInitializedError),
        }
    }
}

static SHARED_STATE: OnceCell<Mutex<SharedState>> = OnceCell::new();

fn shared_state() -> impl DerefMut<Target = SharedState> {
    SHARED_STATE
        .get_or_init(|| Mutex::new(SharedState::Uninitialized))
        .lock()
        .unwrap()
}

#[no_mangle]
unsafe extern "C" fn init(me: u64, n: u64, quorum: u64) -> u8 {
    InitResult::from(init_safe(me, n, quorum)).into()
}

fn init_safe(me: u64, n: u64, quorum: u64) -> Result<(), InitError> {
    shared_state().init(me.into(), n, quorum)?;

    Ok(())
}

#[no_mangle]
unsafe extern "C" fn get_public_key(pub_key: *mut Ec256PublicKey) -> u8 {
    GetPublicKeyResult::from(get_public_key_safe(&mut *pub_key)).into()
}

fn get_public_key_safe(pub_key: &mut Ec256PublicKey) -> Result<(), GetPublicKeyError> {
    *pub_key = shared_state().initialized()?.get_public_key().into();

    Ok(())
}

#[no_mangle]
unsafe extern "C" fn add_peer(id: u64, pub_key: *const Ec256PublicKey) -> u8 {
    AddPeerResult::from(add_peer_safe(id, &*pub_key)).into()
}

fn add_peer_safe(id: u64, pub_key: &Ec256PublicKey) -> Result<(), AddPeerError> {
    shared_state()
        .initialized()?
        .add_peer(id.into(), pub_key.clone().into())?;

    Ok(())
}

#[no_mangle]
unsafe extern "C" fn accumulate(
    commitments: *const u8,
    commitments_len: usize,
    signed_commitment: *mut u8,
    signed_commitment_max_len: usize,
    signed_commitment_len: *mut usize,
) -> u8 {
    let commitments = sgx_utils::slice::call_param_to_slice(commitments, commitments_len);
    let signed_commitment =
        sgx_utils::slice::call_param_to_slice_mut(signed_commitment, signed_commitment_max_len);

    AccumulateResult::from(accumulate_safe(
        commitments,
        signed_commitment,
        &mut *signed_commitment_len,
    ))
    .into()
}

fn accumulate_safe(
    commitments: &[u8],
    signed_commitment: &mut [u8],
    signed_commitment_len: &mut usize,
) -> Result<(), AccumulateError> {
    let commitments: Vec<_> = bincode::deserialize(commitments)?;

    let sc = shared_state().initialized()?.accumulate(&commitments)?;

    let sc = bincode::serialize(&sc)?;
    if signed_commitment.len() < sc.len() {
        return Err(AccumulateError::SignedCommitmentBufferToSmall);
    }
    let signed_commitment = &mut signed_commitment[..sc.len()];
    signed_commitment.copy_from_slice(&sc);
    *signed_commitment_len = signed_commitment.len();

    Ok(())
}

#[no_mangle]
unsafe extern "C" fn prepare(
    block_view: u64,
    block_hash: *const u8,
    block_hash_len: usize,
    block_justification: *const u8,
    block_justification_len: usize,
    block_parent: *const u8,
    block_parent_len: usize,
    justification_block_hash: *const u8,
    justification_block_hash_len: usize,
    signed_commitment: *mut u8,
    signed_commitment_max_len: usize,
    signed_commitment_len: *mut usize,
) -> u8 {
    let block_hash = sgx_utils::slice::call_param_to_slice(block_hash, block_hash_len);
    let block_justification =
        sgx_utils::slice::call_param_to_slice(block_justification, block_justification_len);
    let block_parent = sgx_utils::slice::call_param_to_slice(block_parent, block_parent_len);
    let justification_block_hash = sgx_utils::slice::call_param_to_slice(
        justification_block_hash,
        justification_block_hash_len,
    );
    let signed_commitment =
        sgx_utils::slice::call_param_to_slice_mut(signed_commitment, signed_commitment_max_len);

    PrepareResult::from(prepare_safe(
        block_view,
        block_hash,
        block_justification,
        block_parent,
        justification_block_hash,
        signed_commitment,
        &mut *signed_commitment_len,
    ))
    .into()
}

fn prepare_safe(
    block_view: u64,
    block_hash: &[u8],
    block_justification: &[u8],
    block_parent: &[u8],
    justification_block_hash: &[u8],
    signed_commitment: &mut [u8],
    signed_commitment_len: &mut usize,
) -> Result<(), PrepareError> {
    let block_view = View::new(block_view);
    let block_hash = <[u8; 32]>::try_from(block_hash)
        .map_err(|_| PrepareError::WrongBlockHashLength)?
        .into();
    let block_justification = bincode::deserialize(block_justification)?;
    let block_parent = bincode::deserialize(block_parent)?;
    let justification_block_hash = <[u8; 32]>::try_from(justification_block_hash)
        .map_err(|_| PrepareError::WrongBlockHashLength)?
        .into();

    let sc = shared_state().initialized()?.prepare(
        block_view,
        block_hash,
        block_justification,
        block_parent,
        justification_block_hash,
    )?;

    let sc = bincode::serialize(&sc)?;
    if signed_commitment.len() < sc.len() {
        return Err(PrepareError::SignedCommitmentBufferToSmall);
    }
    let signed_commitment = &mut signed_commitment[..sc.len()];
    signed_commitment.copy_from_slice(&sc);
    *signed_commitment_len = signed_commitment.len();

    Ok(())
}

#[no_mangle]
unsafe extern "C" fn re_sign_last_prepared(
    signed_commitment: *mut u8,
    signed_commitment_max_len: usize,
    signed_commitment_len: *mut usize,
) -> u8 {
    let signed_commitment =
        sgx_utils::slice::call_param_to_slice_mut(signed_commitment, signed_commitment_max_len);

    ReSignLastPrepareResult::from(re_sign_last_prepared_safe(
        signed_commitment,
        &mut *signed_commitment_len,
    ))
    .into()
}

fn re_sign_last_prepared_safe(
    signed_commitment: &mut [u8],
    signed_commitment_len: &mut usize,
) -> Result<(), ReSignLastPrepareError> {
    let sc = shared_state().initialized()?.re_sign_last_prepared()?;

    let sc = bincode::serialize(&sc)?;
    if signed_commitment.len() < sc.len() {
        return Err(ReSignLastPrepareError::SignedCommitmentBufferToSmall);
    }
    let signed_commitment = &mut signed_commitment[..sc.len()];
    signed_commitment.copy_from_slice(&sc);
    *signed_commitment_len = signed_commitment.len();

    Ok(())
}
