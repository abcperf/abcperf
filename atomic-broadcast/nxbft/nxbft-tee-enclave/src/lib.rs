#![cfg_attr(not(target_vendor = "teaclave"), no_std)]
#![cfg_attr(target_vendor = "teaclave", feature(rustc_private))]

#[cfg(not(target_vendor = "teaclave"))]
#[macro_use]
extern crate sgx_tstd as std;
extern crate sgx_libc;
extern crate sgx_types;

use nxbft_base::enclave::EnclaveAutomata;
use nxbft_base::enclave::EnclaveAutomataExport;
use nxbft_base::enclave::EnclaveTrusted;
use nxbft_base::enclave::HandshakeResult;
use nxbft_base::enclave::InitHandshakeResult;
use nxbft_base::vertex::SignedVertex;
use nxbft_base::Round;
use nxbft_tee_shared::{
    CompletePeerHandshakeError, CompletePeerHandshakeResult, EnclaveAutomataFFIError,
    EnclaveAutomataFFIResult, InitPeerHandshakeError, InitPeerHandshakeResult, IsReadyResult,
    SgxEnclaveTrustedError,
};
use once_cell::sync::OnceCell;
use remote_attestation_shared::types::PubKey;
use remote_attestation_shared::types::RawSignature;
use sgx_crypto::aes::gcm::{Aad, AesGcm, Nonce};
use sgx_crypto::ecc::EcKeyPair;
use sgx_crypto::ecc::{EcPrivateKey, EcPublicKey};
use sgx_rand::Rng;
use sgx_types::types::Key128bit;
use sgx_types::types::{Ec256PublicKey, Ec256Signature, Mac128bit};
use shared_ids::ReplicaId;
use std::boxed::Box;

use sgx_utils::SgxStatusExt;

use std::ops::DerefMut;
use std::sync::Mutex;
use std::vec::Vec;

type Export = EnclaveAutomataExport<(PubKey, Box<[u8]>)>;

struct SgxEnclaveTrusted {
    key_pair: EcKeyPair,
}

impl EnclaveTrusted for SgxEnclaveTrusted {
    type Attestation = (PubKey, Box<[u8]>);
    type Error = SgxEnclaveTrustedError;
    type Handshake = (Box<[u8]>, Mac128bit);
    type Signature = RawSignature;
    fn gen_seed_share(&self) -> Result<[u8; 32], Self::Error> {
        let mut seed_share = [0; 32];

        if let Ok(mut rng) = sgx_rand::StdRng::new() {
            rng.fill_bytes(&mut seed_share);
            return Ok(seed_share);
        } else {
            return Err(SgxEnclaveTrustedError::RngInitFailed);
        }
    }
    fn seal_seed_share(
        &self,
        attestation: &Self::Attestation,
        share: [u8; 32],
    ) -> Result<Self::Handshake, Self::Error> {
        let (remote_pub_key, _) = attestation;
        // TODO RA

        let shared_key = gen_shared_key(
            &self.key_pair.private_key(),
            &EcPublicKey::from(Ec256PublicKey::from(*remote_pub_key)),
        );

        let mut aes = AesGcm::new(
            &shared_key,
            Nonce::zeroed(), /* TODO random */
            Aad::from(&[]),
        )
        .into_result()
        .map_err(|_| SgxEnclaveTrustedError::AesInitFailed)?;

        let mut encrypted_share = vec![0; share.len()];
        let encrypted_share_mac = aes
            .encrypt(&share, &mut encrypted_share)
            .into_result()
            .map_err(|_| SgxEnclaveTrustedError::AesEncryptFailed)?;

        Ok((encrypted_share.into(), encrypted_share_mac))
    }
    fn unseal_seed_share(
        &self,
        share: &Self::Handshake,
        attestation: &Self::Attestation,
    ) -> Result<[u8; 32], Self::Error> {
        let (remote_pub_key, _) = attestation;
        let shared_key = gen_shared_key(
            &self.key_pair.private_key(),
            &EcPublicKey::from(Ec256PublicKey::from(*remote_pub_key)),
        );
        let mut aes = AesGcm::new(
            &shared_key,
            Nonce::zeroed(), /* TODO random */
            Aad::from(&[]),
        )
        .into_result()
        .map_err(|_| SgxEnclaveTrustedError::AesInitFailed)?;

        let (remote_proposed_secret, remote_proposed_secret_mac) = share;

        if remote_proposed_secret.len() != 32 {
            return Err(SgxEnclaveTrustedError::InvalidHandshakeLength);
        }

        let mut handshake = [0; 32];

        aes.decrypt(
            remote_proposed_secret,
            &mut handshake,
            &remote_proposed_secret_mac,
        )
        .into_result()
        .map_err(|_| SgxEnclaveTrustedError::InvalidHandshake)?;

        Ok(handshake)
    }
    fn sign(&mut self, message_hash: &[u8], counter: u64) -> Result<Self::Signature, Self::Error> {
        let mut data = Vec::<u8>::new();
        data.extend_from_slice(message_hash);
        data.extend_from_slice(&counter.to_be_bytes());

        let signature = self
            .key_pair
            .private_key()
            .sign(data.as_slice())
            .map_err(|_| SgxEnclaveTrustedError::SigningFailed)?
            .signature();

        Ok(signature.into())
    }
    fn verify(
        attestation: &Self::Attestation,
        counter: u64,
        message_hash: &[u8],
        signature: &Self::Signature,
    ) -> Result<bool, Self::Error> {
        let mut data = Vec::new();
        data.extend_from_slice(message_hash);
        data.extend_from_slice(&counter.to_be_bytes());

        let (pubkey, _) = attestation;

        let valid = EcPublicKey::from(Ec256PublicKey::from(*pubkey))
            .verify(
                data.as_slice(),
                &Ec256Signature::from(signature.clone()).into(),
            )
            .into_result()
            .map_err(|_| SgxEnclaveTrustedError::EcVerifyFailed)?;

        Ok(valid)
    }
    fn get_attestation_certificate(&mut self) -> Result<Self::Attestation, Self::Error> {
        Ok((self.key_pair.public_key().public_key().into(), Box::new([])))
    }
}

struct SharedState {
    automata: EnclaveAutomata<SgxEnclaveTrusted>,
}

// TODO: dedup
fn gen_shared_key(priv_key: &EcPrivateKey, pub_key: &EcPublicKey) -> Key128bit {
    priv_key
        .shared_key(pub_key)
        .unwrap()
        .derive_key(b"some label")
        .unwrap()
        .key
}

impl SharedState {
    // Creates a key pair for the enclave and saves it in the state
    fn init(n: u64) -> Result<Self, EnclaveAutomataFFIError> {
        let key_pair = if let Ok(key_pair) = EcKeyPair::create() {
            key_pair
        } else {
            return Err(EnclaveAutomataFFIError::KeyGenFailed);
        };

        let mut automata = EnclaveAutomata::new(n, SgxEnclaveTrusted { key_pair });
        automata.init()?;
        Ok(Self { automata })
    }

    fn init_peer_handshake(
        &mut self,
        peer_id: ReplicaId,
        remote_pub_key: Ec256PublicKey,
        remote_ra: &[u8],
        local_proposed_secret: &mut [u8],
        local_proposed_secret_len: &mut usize,
        local_proposed_secret_mac: &mut Mac128bit,
    ) -> Result<(), InitPeerHandshakeError> {
        match self.automata.init_peer_handshake(
            peer_id,
            (remote_pub_key.into(), remote_ra.iter().copied().collect()),
        ) {
            InitHandshakeResult::Ok((secret, mac)) => {
                local_proposed_secret[..secret.len()].copy_from_slice(&secret);
                *local_proposed_secret_len = secret.len();
                *local_proposed_secret_mac = mac;
                Ok(())
            }
            InitHandshakeResult::DuplicateHello => Err(InitPeerHandshakeError::DuplicateHello),
            InitHandshakeResult::InvalidAttestation => {
                Err(InitPeerHandshakeError::InvalidAttestation)
            }
            InitHandshakeResult::EnclaveError(e) => Err(e.into()),
        }
    }

    fn complete_peer_handshake(
        &mut self,
        peer_id: ReplicaId,
        remote_proposed_secret: &[u8],
        remote_proposed_secret_mac: Mac128bit,
    ) -> Result<(), CompletePeerHandshakeError> {
        match self.automata.complete_peer_handshake(
            peer_id,
            &(
                remote_proposed_secret.iter().copied().collect(),
                remote_proposed_secret_mac,
            ),
        ) {
            HandshakeResult::MissingAttestation => {
                Err(CompletePeerHandshakeError::MissingAttestation)
            }
            HandshakeResult::DuplicateReply => Err(CompletePeerHandshakeError::DuplicateReply),
            HandshakeResult::InvalidHandshakeEncryption => {
                Err(CompletePeerHandshakeError::InvalidHandshakeEncryption)
            }
            HandshakeResult::Ok => Ok(()),
            HandshakeResult::EnclaveError(e) => Err(e.into()),
        }
    }

    fn toss(&mut self, proof: &[u8]) -> Result<ReplicaId, EnclaveAutomataFFIError> {
        let proof: Vec<SignedVertex<RawSignature>> = bincode::deserialize(proof)?;
        let id = self.automata.toss(proof)?;
        Ok(id)
    }

    fn attest(&mut self) -> Result<EcPublicKey, EnclaveAutomataFFIError> {
        let (key, _) = self.automata.get_attestation_certificate()?;

        Ok(EcPublicKey::from(Ec256PublicKey::from(key)))
    }

    fn is_ready(&self) -> IsReadyResult {
        match self.automata.is_ready() {
            true => IsReadyResult::Ready,
            false => IsReadyResult::NotReady,
        }
    }

    fn sign(&mut self, message: &[u8]) -> Result<(Ec256Signature, u64), EnclaveAutomataFFIError> {
        let (sig, count) = self.automata.sign(message)?;
        Ok((sig.into(), count))
    }

    fn export(&mut self) -> Result<Export, EnclaveAutomataFFIError> {
        let export = self.automata.export()?;

        Ok(export)
    }

    fn recover(&mut self, export: Export) -> Result<(), EnclaveAutomataFFIError> {
        self.automata.recover(export)?;
        Ok(())
    }

    fn replace_peer(
        &mut self,
        peer_id: ReplicaId,
        remote_pub_key: Ec256PublicKey,
        remote_ra: &[u8],
        max_round: Round,
    ) -> Result<(), EnclaveAutomataFFIError> {
        self.automata.replace_peer(
            peer_id,
            &(
                PubKey::from(remote_pub_key),
                remote_ra.iter().copied().collect(),
            ),
            max_round,
        )?;
        Ok(())
    }
}

static SHARED_STATE: OnceCell<Mutex<SharedState>> = OnceCell::new();

fn shared_state() -> impl DerefMut<Target = SharedState> {
    SHARED_STATE.get().unwrap().lock().unwrap()
}

fn try_shared_state() -> Option<impl DerefMut<Target = SharedState>> {
    Some(SHARED_STATE.get()?.lock().ok()?)
}

#[no_mangle]
extern "C" fn init(n: u64) -> u8 {
    EnclaveAutomataFFIResult::from(init_safe(n)).into()
}

fn init_safe(n: u64) -> Result<(), EnclaveAutomataFFIError> {
    SHARED_STATE
        .set(Mutex::new(SharedState::init(n)?))
        .map_err(|_| EnclaveAutomataFFIError::AlreadyInitialized)?;

    Ok(())
}

/// Performs an attestation
///
/// # Safety
/// Converts C array into slice
#[no_mangle]
unsafe extern "C" fn attest(
    local_pub_key: *mut Ec256PublicKey,
    local_ra: *mut u8,
    local_ra_max_len: usize,
    local_ra_len: *mut usize,
) -> u8 {
    EnclaveAutomataFFIResult::from(attest_safe(
        &mut *local_pub_key,
        sgx_utils::slice::call_param_to_slice_mut(local_ra, local_ra_max_len),
        &mut *local_ra_len,
    ))
    .into()
}

fn attest_safe(
    local_pub_key: &mut Ec256PublicKey,
    _local_ra: &mut [u8],
    local_ra_len: &mut usize,
) -> Result<(), EnclaveAutomataFFIError> {
    let mut state = shared_state();

    let key_pair = state.attest()?;

    *local_pub_key = key_pair.public_key();

    // TODO RA
    *local_ra_len = 0;

    Ok(())
}

/// # Safety
/// Converts C array into slice
#[no_mangle]
unsafe extern "C" fn init_peer_handshake(
    peer_id: u64,
    remote_pub_key: *const Ec256PublicKey,
    remote_ra: *const u8,
    remote_ra_len: usize,
    local_proposed_secret: *mut u8,
    local_proposed_secret_max_len: usize,
    local_proposed_secret_len: *mut usize,
    local_proposed_secret_mac: *mut Mac128bit,
) -> u8 {
    InitPeerHandshakeResult::from(init_peer_handshake_safe(
        peer_id,
        *remote_pub_key,
        sgx_utils::slice::call_param_to_slice(remote_ra, remote_ra_len),
        sgx_utils::slice::call_param_to_slice_mut(
            local_proposed_secret,
            local_proposed_secret_max_len,
        ),
        &mut *local_proposed_secret_len,
        &mut *local_proposed_secret_mac,
    ))
    .into()
}

fn init_peer_handshake_safe(
    peer_id: u64,
    remote_pub_key: Ec256PublicKey,
    remote_ra: &[u8],
    local_proposed_secret: &mut [u8],
    local_proposed_secret_len: &mut usize,
    local_proposed_secret_mac: &mut Mac128bit,
) -> Result<(), InitPeerHandshakeError> {
    let mut state = shared_state();

    let peer_id = ReplicaId::from_u64(peer_id);

    state.init_peer_handshake(
        peer_id,
        remote_pub_key,
        remote_ra,
        local_proposed_secret,
        local_proposed_secret_len,
        local_proposed_secret_mac,
    )
}

/// # Safety
/// Converts C array into slice
#[no_mangle]
unsafe extern "C" fn complete_peer_handshake(
    peer_id: u64,
    remote_proposed_secret: *const u8,
    remote_proposed_secret_len: usize,
    remote_proposed_secret_mac: *const Mac128bit,
) -> u8 {
    CompletePeerHandshakeResult::from(complete_peer_handshake_safe(
        peer_id,
        sgx_utils::slice::call_param_to_slice(remote_proposed_secret, remote_proposed_secret_len),
        *remote_proposed_secret_mac,
    ))
    .into()
}

fn complete_peer_handshake_safe(
    peer_id: u64,
    remote_proposed_secret: &[u8],
    remote_proposed_secret_mac: Mac128bit,
) -> Result<(), CompletePeerHandshakeError> {
    let mut state = shared_state();

    let peer_id = ReplicaId::from_u64(peer_id);

    state.complete_peer_handshake(peer_id, remote_proposed_secret, remote_proposed_secret_mac)
}

#[no_mangle]
extern "C" fn is_ready() -> u8 {
    is_ready_safe().into()
}

fn is_ready_safe() -> IsReadyResult {
    let shared_state = try_shared_state();

    if let Some(shared_state) = shared_state {
        shared_state.is_ready()
    } else {
        IsReadyResult::NotReady
    }
}

/// Creates a signature based on the provided message and the internal counter
///
/// # Safety
/// Converts C array into slice
#[no_mangle]
unsafe extern "C" fn sign(
    payload: *const u8,
    payload_len: usize,
    signature: *mut Ec256Signature,
    counter: *mut u64,
) -> u8 {
    let payload = sgx_utils::slice::call_param_to_slice(payload, payload_len);
    EnclaveAutomataFFIResult::from(sign_safe(payload, &mut *signature, &mut *counter)).into()
}

/// Creates a signature based on the provided message and the internal counter
fn sign_safe(
    message: &[u8],
    signature: &mut Ec256Signature,
    counter: &mut u64,
) -> Result<(), EnclaveAutomataFFIError> {
    let mut shared_state = shared_state();

    let (sig, ctr) = shared_state.sign(message)?;

    *signature = sig;
    *counter = ctr;

    Ok(())
}

/// # Safety
/// Converts C array into slice
#[no_mangle]
unsafe extern "C" fn toss(proof: *const u8, proof_len: usize, peer_id: *mut u64) -> u8 {
    let proof = sgx_utils::slice::call_param_to_slice(proof, proof_len);

    EnclaveAutomataFFIResult::from(toss_safe(proof, &mut *peer_id)).into()
}

fn toss_safe(proof: &[u8], peer_id: &mut u64) -> Result<(), EnclaveAutomataFFIError> {
    let mut shared_state = shared_state();

    *peer_id = shared_state.toss(proof)?.as_u64();

    Ok(())
}

#[no_mangle]
unsafe extern "C" fn export(export: *mut u8, export_max_len: usize, export_len: *mut usize) -> u8 {
    let export = sgx_utils::slice::call_param_to_slice_mut(export, export_max_len);

    EnclaveAutomataFFIResult::from(export_safe(export, &mut *export_len)).into()
}

fn export_safe(export: &mut [u8], export_len: &mut usize) -> Result<(), EnclaveAutomataFFIError> {
    let ex: Export = shared_state().export()?;

    let ex = bincode::serialize(&ex)?;
    if export.len() < ex.len() {
        return Err(EnclaveAutomataFFIError::ExportBufferToSmall);
    }
    let export = &mut export[..ex.len()];
    export.copy_from_slice(&ex);
    *export_len = export.len();

    Ok(())
}

#[no_mangle]
unsafe extern "C" fn recover(export: *const u8, export_len: usize) -> u8 {
    let export = sgx_utils::slice::call_param_to_slice(export, export_len);

    EnclaveAutomataFFIResult::from(recover_safe(export)).into()
}

fn recover_safe(export: &[u8]) -> Result<(), EnclaveAutomataFFIError> {
    let export: Export = bincode::deserialize(export)?;

    init_safe(export.n)?;

    shared_state().recover(export)?;

    Ok(())
}

#[no_mangle]
unsafe extern "C" fn replace_peer(
    peer_id: u64,
    remote_pub_key: *const Ec256PublicKey,
    remote_ra: *const u8,
    remote_ra_len: usize,
    max_round: u64,
) -> u8 {
    let remote_ra = sgx_utils::slice::call_param_to_slice(remote_ra, remote_ra_len);

    EnclaveAutomataFFIResult::from(replace_peer_safe(
        peer_id,
        *remote_pub_key,
        remote_ra,
        max_round,
    ))
    .into()
}

fn replace_peer_safe(
    peer_id: u64,
    remote_pub_key: Ec256PublicKey,
    remote_ra: &[u8],
    max_round: u64,
) -> Result<(), EnclaveAutomataFFIError> {
    let mut shared_state = shared_state();

    let max_round = Round::from_u64(max_round);

    shared_state.replace_peer(peer_id.into(), remote_pub_key, remote_ra, max_round)?;

    Ok(())
}
