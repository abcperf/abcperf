use once_cell::sync::OnceCell;

mod ecall;
pub mod enclave;

pub use remote_attestation_shared::types::PubKey;
use sgx_types::{error::SgxStatus, types::Mac128bit};
use sgx_utils::{SgxResult, SgxResultExt};
use shared_ids::{ClientId, RequestId};

type ToBftCell = OnceCell<Box<dyn Fn(ClientId, RequestId, Box<[u8]>, [u8; 16]) + Sync + Send>>;
pub static TO_BFT_SAFE: ToBftCell = OnceCell::new();

/// # Safety
/// TODO
#[no_mangle]
pub unsafe extern "C" fn maas_tee_to_bft(
    client_id: u64,
    request_id: u64,
    payload: *const u8,
    payload_len: usize,
    mac: *const Mac128bit,
) -> SgxStatus {
    let payload = sgx_utils::slice::call_param_to_slice(payload, payload_len);

    to_bft_safe(client_id, request_id, payload, *mac).into_status()
}

fn to_bft_safe(client_id: u64, request_id: u64, payload: &[u8], mac: Mac128bit) -> SgxResult {
    if let Some(func) = TO_BFT_SAFE.get() {
        let payload = payload.iter().copied().collect();
        func(
            ClientId::from_u64(client_id),
            RequestId::from_u64(request_id),
            payload,
            mac,
        );
    }
    Ok(())
}

type ToClientCell = OnceCell<Box<dyn Fn(ClientId, RequestId, Vec<u8>, Mac128bit) + Sync + Send>>;
pub static TO_CLIENT_SAFE: ToClientCell = OnceCell::new();

/// # Safety
/// TODO
#[no_mangle]
pub unsafe extern "C" fn maas_tee_to_client(
    client_id: u64,
    request_id: u64,
    payload: *const u8,
    payload_len: usize,
    mac: *const Mac128bit,
) -> SgxStatus {
    let payload = sgx_utils::slice::call_param_to_slice(payload, payload_len);

    to_client_safe(
        ClientId::from_u64(client_id),
        RequestId::from_u64(request_id),
        payload,
        *mac,
    )
    .into_status()
}

fn to_client_safe(
    client_id: ClientId,
    request_id: RequestId,
    payload: &[u8],
    mac: Mac128bit,
) -> SgxResult {
    if let Some(func) = TO_CLIENT_SAFE.get() {
        let payload = payload.to_vec();
        func(client_id, request_id, payload, mac);
    }
    Ok(())
}
