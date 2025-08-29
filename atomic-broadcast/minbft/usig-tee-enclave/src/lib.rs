#![cfg_attr(not(target_vendor = "teaclave"), no_std)]
#![cfg_attr(target_vendor = "teaclave", feature(rustc_private))]

#[cfg(not(target_vendor = "teaclave"))]
#[macro_use]
extern crate sgx_tstd as std;
extern crate sgx_libc;
extern crate sgx_types;

use once_cell::sync::OnceCell;
use sgx_crypto::ecc::EcKeyPair;
use sgx_types::error::SgxStatus;
use sgx_types::types::{Ec256PublicKey, Ec256Signature};

use sgx_utils::{SgxResult, SgxResultExt};

use std::ops::DerefMut;
use std::str;
use std::sync::Mutex;
use std::vec::Vec;

/// The struct that defines the state of the enclave
struct SharedState {
    key_pair: EcKeyPair,
    counter: u64,
}

impl SharedState {
    // Creates a key pair for the enclave and saves it in the state
    fn init() -> Self {
        let key_pair = EcKeyPair::create().unwrap();

        Self {
            key_pair,
            counter: 0,
        }
    }
}

static SHARED_STATE: OnceCell<Mutex<SharedState>> = OnceCell::new();

fn shared_state() -> impl DerefMut<Target = SharedState> {
    SHARED_STATE
        .get_or_init(|| Mutex::new(SharedState::init()))
        .lock()
        .unwrap()
}

/// Creates a signature based on the provided message and the internal counter
///
/// # Safety
/// Converts C array into slice
#[no_mangle]
unsafe extern "C" fn sign(
    message: *const u8,
    len: usize,
    signature: *mut Ec256Signature,
    counter: *mut u64,
) -> SgxStatus {
    let message = sgx_utils::slice::call_param_to_slice(message, len);
    sign_safe(message, &mut *signature, &mut *counter).into_status()
}

/// Creates a signature based on the provided message and the internal counter
fn sign_safe(message: &[u8], signature: &mut Ec256Signature, counter: &mut u64) -> SgxResult {
    let mut state = shared_state();

    *counter = state.counter;
    state.counter += 1;

    let mut data = Vec::<u8>::new();
    data.extend_from_slice(message);
    data.extend_from_slice(&counter.to_be_bytes());

    *signature = state
        .key_pair
        .private_key()
        .sign(data.as_slice())
        .unwrap()
        .signature();

    Ok(())
}

/// Performs an attestation
///
/// # Safety
/// Converts C array into slice
#[no_mangle]
unsafe extern "C" fn attest(
    pubkey: *mut Ec256PublicKey,
    counter: *mut u64,
    cert_der: *mut u8,
    cert_der_max_len: usize,
    cert_der_len: *mut usize,
) -> SgxStatus {
    attest_safe(
        &mut *pubkey,
        &mut *counter,
        sgx_utils::slice::call_param_to_slice_mut(cert_der, cert_der_max_len),
        &mut *cert_der_len,
    )
    .into_status()
}

fn attest_safe(
    pubkey: &mut Ec256PublicKey,
    counter: &mut u64,
    cert_der: &mut [u8],
    cert_der_len: &mut usize,
) -> SgxResult {
    let state = shared_state();

    *pubkey = state.key_pair.public_key().public_key();
    *counter = state.counter;

    // TODO RA
    *cert_der_len = 0;

    Ok(())
}
