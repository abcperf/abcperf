use sgx_types::{
    error::{SgxResult, SgxStatus},
    types::Key128bit,
};

extern "C" {
    fn maas_tee_to_bft(
        ret_val: *mut SgxStatus,
        client_id: u64,
        request_id: u64,
        payload: *const u8,
        payload_len: usize,
        mac: *const Key128bit,
    ) -> SgxStatus;

    fn maas_tee_to_client(
        ret_val: *mut SgxStatus,
        client_id: u64,
        request_id: u64,
        payload: *const u8,
        payload_len: usize,
        mac: *const Key128bit,
    ) -> SgxStatus;
}

pub(crate) fn to_bft(
    client_id: u64,
    request_id: u64,
    payload: &[u8],
    mac: Key128bit,
) -> SgxResult<SgxStatus> {
    let mut retval = SgxStatus::Success;

    let (payload, payload_len) = sgx_utils::slice::call_param_to_ptr(payload);

    let result = unsafe {
        maas_tee_to_bft(
            &mut retval,
            client_id,
            request_id,
            payload,
            payload_len,
            &mac,
        )
    };

    match result {
        SgxStatus::Success => Ok(retval),
        _ => Err(result),
    }
}

pub(crate) fn to_client(
    client_id: u64,
    request_id: u64,
    payload: &[u8],
    mac: Key128bit,
) -> SgxResult<SgxStatus> {
    let mut retval = SgxStatus::Success;

    let (payload, payload_len) = sgx_utils::slice::call_param_to_ptr(payload);

    let result = unsafe {
        maas_tee_to_client(
            &mut retval,
            client_id,
            request_id,
            payload,
            payload_len,
            &mac,
        )
    };

    match result {
        SgxStatus::Success => Ok(retval),
        _ => Err(result),
    }
}
