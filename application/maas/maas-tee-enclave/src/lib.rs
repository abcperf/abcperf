#![cfg_attr(not(target_vendor = "teaclave"), no_std)]
#![cfg_attr(target_vendor = "teaclave", feature(rustc_private))]

#[cfg(not(target_vendor = "teaclave"))]
#[macro_use]
extern crate sgx_tstd as std;
extern crate sgx_libc;
extern crate sgx_types;

use std::sync::Mutex;

use db::{Db, InnerModifyMsg};
use maas_types::{ClientRequest, InnerClientRequest};
use once_cell::sync::OnceCell;
use sgx_crypto::{
    aes::gcm::{Aad, AesGcm, Nonce},
    ecc::{EcKeyPair, EcPrivateKey, EcPublicKey},
    sha::Sha256,
};
use sgx_rand::Rng;
use sgx_types::{
    error::SgxStatus,
    types::{Ec256PublicKey, Key128bit, Mac128bit},
};
use sgx_utils::{SgxResult, SgxResultExt, SgxStatusExt};

use crate::db::ModifyMsg;

mod db;
mod ocall;

struct MutableState {
    db: Db,
    id: usize,
    used_key: Key128bit,
    used_key_pair: EcKeyPair,
}

struct SharedState {
    key_pair: EcKeyPair,
    proposed_key: Key128bit,
    mutable: Mutex<MutableState>,
}

impl SharedState {
    // Creates a key pair for the enclave and saves it in the state
    fn init() -> Self {
        let key_pair = EcKeyPair::create().unwrap();

        let mut proposed_key = Key128bit::default();
        sgx_rand::StdRng::new()
            .unwrap()
            .fill_bytes(&mut proposed_key);

        let used_key_pair = EcKeyPair::create_with_seed(&proposed_key).unwrap();

        Self {
            key_pair,
            proposed_key,
            mutable: Mutex::new(MutableState {
                used_key: proposed_key,
                used_key_pair,
                id: 0,
                db: Db::default(),
            }),
        }
    }

    fn get_shared_key_pair(&self) -> EcKeyPair {
        let lock = self.mutable.lock().unwrap();
        lock.used_key_pair
    }
}

static SHARED_STATE: OnceCell<SharedState> = OnceCell::new();

fn shared_state() -> &'static SharedState {
    SHARED_STATE.get_or_init(SharedState::init)
}

#[no_mangle]
pub extern "C" fn init(id: usize) -> SgxStatus {
    let shared_state = shared_state();
    shared_state.mutable.lock().unwrap().id = id;

    SgxStatus::Success
}

/// # Safety
/// TODO
#[no_mangle]
pub unsafe extern "C" fn from_bft(
    client_id: u64,
    request_id: u64,
    payload: *const u8,
    payload_len: usize,
    mac: *const Mac128bit,
) -> SgxStatus {
    let payload = sgx_utils::slice::call_param_to_slice(payload, payload_len);

    from_bft_safe(client_id, request_id, payload, *mac).into_status()
}

fn from_bft_safe(
    client_id: u64,
    request_id: u64,
    encrypted_payload: &[u8],
    mac: Mac128bit,
) -> SgxResult {
    let shared_state = shared_state();

    let key = shared_state.mutable.lock().unwrap().used_key;

    let mut payload_bytes = vec![0; encrypted_payload.len()];

    AesGcm::new(&key, Nonce::zeroed() /* TODO random */, Aad::from(&[]))
        .into_result()?
        .decrypt(encrypted_payload, payload_bytes.as_mut_slice(), &mac)
        .into_result()?;

    let payload: ModifyMsg = bincode::deserialize(payload_bytes.as_slice()).unwrap(); //TODO error handling

    let enc_key = payload.enc_key;

    let _ = shared_state.mutable.lock().unwrap().db.modify(payload); // TODO return value handling

    send_to_client(client_id, request_id, &enc_key, Ok(()))?;

    Ok(())
}

/// # Safety
/// TODO
#[no_mangle]
pub unsafe extern "C" fn from_client(
    client_id: u64,
    request_id: u64,
    client_pub_key: *const Ec256PublicKey,
    payload: *const u8,
    payload_len: usize,
    mac: *const Mac128bit,
) -> SgxStatus {
    let payload = sgx_utils::slice::call_param_to_slice(payload, payload_len);

    from_client_safe(client_id, request_id, *client_pub_key, payload, *mac).into_status()
}

fn from_client_safe(
    client_id: u64,
    request_id: u64,
    client_pub_key: Ec256PublicKey,
    encrypted_payload: &[u8],
    mac: Mac128bit,
) -> SgxResult {
    let mut payload_bytes = vec![0; encrypted_payload.len()];

    let state = shared_state();

    let enc_key = gen_shared_key(
        &state.get_shared_key_pair().private_key(),
        &client_pub_key.into(),
    );

    AesGcm::new(
        &enc_key,
        Nonce::zeroed(), /* TODO random */
        Aad::from(&[]),
    )
    .into_result()?
    .decrypt(encrypted_payload, payload_bytes.as_mut_slice(), &mac)
    .into_result()?;

    let ClientRequest {
        inner,
        response_key,
    } = bincode::deserialize(payload_bytes.as_slice()).unwrap(); //TODO error handling

    match inner {
        InnerClientRequest::PreRegister => todo!("reply to client"),
        InnerClientRequest::Register(r) => send_to_bft(
            client_id,
            request_id,
            response_key,
            InnerModifyMsg::Register(r),
        )?,
        InnerClientRequest::Checkin(i) => send_to_bft(
            client_id,
            request_id,
            response_key,
            InnerModifyMsg::Checkin(i),
        )?,
        InnerClientRequest::Checkout(o) => send_to_bft(
            client_id,
            request_id,
            response_key,
            InnerModifyMsg::Checkout(o),
        )?,
    }

    Ok(())
}

// TODO: dedup
fn gen_shared_key(priv_key: &EcPrivateKey, pub_key: &EcPublicKey) -> Key128bit {
    let shared_secret = priv_key.shared_key(pub_key).unwrap();

    shared_secret.derive_key(b"some label").unwrap().key
}

fn send_to_bft(
    client_id: u64,
    request_id: u64,
    enc_key: Key128bit,
    inner: InnerModifyMsg,
) -> SgxResult {
    let payload = ModifyMsg { inner, enc_key };

    let payload_bytes = bincode::serialize(&payload).unwrap(); // TODO error handling

    let mut encrypted_bytes = vec![0; payload_bytes.len()];

    let key = shared_state().mutable.lock().unwrap().used_key;

    let mac = AesGcm::new(&key, Nonce::zeroed() /* TODO random */, Aad::from(&[]))
        .into_result()?
        .encrypt(payload_bytes.as_slice(), encrypted_bytes.as_mut_slice())
        .into_result()?;

    ocall::to_bft(client_id, request_id, encrypted_bytes.as_slice(), mac).into_result()?;

    Ok(())
}

fn send_to_client(
    client_id: u64,
    request_id: u64,
    enc_key: &Key128bit,
    payload: Result<(), ()>,
) -> SgxResult {
    let payload_bytes = bincode::serialize(&payload).unwrap(); // TODO error handling

    let mut encrypted_bytes = vec![0; payload_bytes.len()];

    let mac = AesGcm::new(
        enc_key,
        Nonce::zeroed(), /* TODO random */
        Aad::from(&[]),
    )
    .into_result()?
    .encrypt(payload_bytes.as_slice(), encrypted_bytes.as_mut_slice())
    .into_result()?;

    ocall::to_client(client_id, request_id, encrypted_bytes.as_slice(), mac).into_result()?;

    Ok(())
}

/// Performs an attestation
///
/// # Safety
/// Converts C array into slice
#[no_mangle]
pub unsafe extern "C" fn attest(
    local_pub_key: *mut Ec256PublicKey,
    local_ra: *mut u8,
    local_ra_max_len: usize,
    local_ra_len: *mut usize,
) -> SgxStatus {
    attest_safe(
        &mut *local_pub_key,
        sgx_utils::slice::call_param_to_slice_mut(local_ra, local_ra_max_len),
        &mut *local_ra_len,
    )
    .into_status()
}

fn attest_safe(
    local_pub_key: &mut Ec256PublicKey,
    local_ra: &mut [u8],
    local_ra_len: &mut usize,
) -> SgxResult {
    let state = shared_state();
    *local_pub_key = state.key_pair.public_key().public_key();

    // TODO RA
    *local_ra_len = 0;

    Ok(())
}

/// Performs an attestation
///
/// # Safety
/// Converts C array into slice
#[no_mangle]
pub unsafe extern "C" fn attest_shared_key(
    shared_pub_key: *mut Ec256PublicKey,
    local_ra: *mut u8,
    local_ra_max_len: usize,
    local_ra_len: *mut usize,
) -> SgxStatus {
    attest_shared_key_safe(
        &mut *shared_pub_key,
        sgx_utils::slice::call_param_to_slice_mut(local_ra, local_ra_max_len),
        &mut *local_ra_len,
    )
    .into_status()
}

fn attest_shared_key_safe(
    shared_pub_key: &mut Ec256PublicKey,
    local_ra: &mut [u8],
    local_ra_len: &mut usize,
) -> SgxResult {
    let state = shared_state();
    *shared_pub_key = state.get_shared_key_pair().public_key().public_key();

    // TODO RA
    *local_ra_len = 0;

    Ok(())
}

/// # Safety
/// Converts C array into slice
#[no_mangle]
pub unsafe extern "C" fn ma_continue(
    remote_pub_key: *const Ec256PublicKey,
    remote_ra: *const u8,
    remote_ra_len: usize,
    local_proposed_secret: *mut u8,
    local_proposed_secret_max_len: usize,
    local_proposed_secret_len: *mut usize,
    local_proposed_secret_mac: *mut Mac128bit,
) -> SgxStatus {
    ma_continue_safe(
        *remote_pub_key,
        sgx_utils::slice::call_param_to_slice(remote_ra, remote_ra_len),
        sgx_utils::slice::call_param_to_slice_mut(
            local_proposed_secret,
            local_proposed_secret_max_len,
        ),
        &mut *local_proposed_secret_len,
        &mut *local_proposed_secret_mac,
    )
    .into_status()
}

fn ma_continue_safe(
    remote_pub_key: Ec256PublicKey,
    remote_ra: &[u8],
    local_proposed_secret: &mut [u8],
    local_proposed_secret_len: &mut usize,
    local_proposed_secret_mac: &mut Mac128bit,
) -> SgxResult {
    // TODO RA verify

    let state = shared_state();

    // send shared secret
    let enc_key = gen_shared_key(&state.key_pair.private_key(), &remote_pub_key.into());
    *local_proposed_secret_len = state.proposed_key.len();
    *local_proposed_secret_mac = AesGcm::new(
        &enc_key,
        Nonce::zeroed(), /* TODO random */
        Aad::from(&[]),
    )
    .into_result()?
    .encrypt(
        &state.proposed_key,
        &mut local_proposed_secret[0..*local_proposed_secret_len],
    )
    .into_result()?;

    Ok(())
}

/// # Safety
/// Converts C array into slice
#[no_mangle]
pub unsafe extern "C" fn ma_finish(
    remote_pub_key: *const Ec256PublicKey,
    remote_ra: *const u8,
    remote_ra_len: usize,
    remote_proposed_secret: *const u8,
    remote_proposed_secret_len: usize,
    remote_proposed_secret_mac: *const Mac128bit,
) -> SgxStatus {
    ma_finish_safe(
        *remote_pub_key,
        sgx_utils::slice::call_param_to_slice(remote_ra, remote_ra_len),
        sgx_utils::slice::call_param_to_slice(remote_proposed_secret, remote_proposed_secret_len),
        *remote_proposed_secret_mac,
    )
    .into_status()
}

fn ma_finish_safe(
    remote_pub_key: Ec256PublicKey,
    remote_ra: &[u8],
    remote_proposed_secret: &[u8],
    remote_proposed_secret_mac: Mac128bit,
) -> SgxResult {
    // TODO RA verify

    let state = shared_state();

    let enc_key = gen_shared_key(&state.key_pair.private_key(), &remote_pub_key.into());

    let mut new_key = Key128bit::default();

    AesGcm::new(
        &enc_key,
        Nonce::zeroed(), /* TODO random */
        Aad::from(&[]),
    )
    .into_result()?
    .decrypt(
        remote_proposed_secret,
        &mut new_key,
        &remote_proposed_secret_mac,
    )
    .into_result()?;

    let mut mutex = state.mutable.lock().unwrap();
    if mutex.used_key > new_key {
        mutex.used_key = new_key;
        mutex.used_key_pair = EcKeyPair::create_with_seed(&mutex.used_key).unwrap();
    }

    Ok(())
}

/// # Safety
/// Converts C array into slice
#[no_mangle]
pub unsafe extern "C" fn shared_secret_info(k_anonymity_hash: *mut u8) -> SgxStatus {
    shared_secret_info_safe(&mut *k_anonymity_hash).into_status()
}

fn shared_secret_info_safe(k_anonymity_hash: &mut u8) -> SgxResult {
    let state = shared_state();
    let mutex = state.mutable.lock().unwrap();

    let mut sha256 = Sha256::new().unwrap();
    sha256.update(&mutex.used_key).unwrap();
    let sha256 = sha256.finalize().unwrap();

    *k_anonymity_hash = sha256[0];

    Ok(())
}
