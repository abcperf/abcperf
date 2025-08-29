use sgx_types::{
    error::SgxStatus,
    types::{Ec256PublicKey, EnclaveId, Mac128bit},
};
use sgx_utils::{SgxResult, SgxStatusExt};

extern "C" {
    fn maas_tee_init(eid: EnclaveId, ret_val: *mut SgxStatus, id: usize) -> SgxStatus;

    fn maas_tee_from_bft(
        eid: EnclaveId,
        ret_val: *mut SgxStatus,
        client_id: u64,
        request_id: u64,
        payload: *const u8,
        payload_len: usize,
        mac: *const Mac128bit,
    ) -> SgxStatus;

    fn maas_tee_from_client(
        eid: EnclaveId,
        ret_val: *mut SgxStatus,
        client_id: u64,
        request_id: u64,
        client_pub_key: *const Ec256PublicKey,
        payload: *const u8,
        payload_len: usize,
        mac: *const Mac128bit,
    ) -> SgxStatus;

    // The ECALL for performing an attestation
    fn maas_tee_attest(
        eid: EnclaveId,
        ret_val: *mut SgxStatus,
        local_pub_key: *mut Ec256PublicKey,
        local_ra: *mut u8,
        local_ra_max_len: usize,
        local_ra_len: *mut usize,
    ) -> SgxStatus;

    // The ECALL for performing an attestation
    fn maas_tee_attest_shared_key(
        eid: EnclaveId,
        ret_val: *mut SgxStatus,
        shared_pub_key: *mut Ec256PublicKey,
        local_ra: *mut u8,
        local_ra_max_len: usize,
        local_ra_len: *mut usize,
    ) -> SgxStatus;

    // The ECALL for continuing a ma
    fn maas_tee_ma_continue(
        eid: EnclaveId,
        ret_val: *mut SgxStatus,
        remote_pub_key: *const Ec256PublicKey,
        remote_ra: *const u8,
        remote_ra_len: usize,
        local_proposed_secret: *mut u8,
        local_proposed_secret_max_len: usize,
        local_proposed_secret_len: *mut usize,
        local_proposed_secret_mac: *mut Mac128bit,
    ) -> SgxStatus;

    // The ECALL for finishing a ma
    fn maas_tee_ma_finish(
        eid: EnclaveId,
        ret_val: *mut SgxStatus,
        remote_pub_key: *const Ec256PublicKey,
        remote_ra: *const u8,
        remote_ra_len: usize,
        remote_proposed_secret: *const u8,
        remote_proposed_secret_len: usize,
        remote_proposed_secret_mac: *const Mac128bit,
    ) -> SgxStatus;

    // The ECALL to get info about the shared secret
    #[allow(dead_code)] // Used by tests
    fn maas_tee_shared_secret_info(
        eid: EnclaveId,
        ret_val: *mut SgxStatus,
        k_anonymity_hash: *mut u8,
    ) -> SgxStatus;
}

pub(crate) fn init(eid: EnclaveId, id: usize) -> SgxResult {
    let mut ret_val = SgxStatus::Success;

    unsafe { maas_tee_init(eid, &mut ret_val, id) }.into_result()?;

    ret_val.into_result()?;

    Ok(())
}

pub(crate) fn from_bft(
    eid: EnclaveId,
    client_id: u64,
    request_id: u64,
    payload: &[u8],
    mac: Mac128bit,
) -> SgxResult {
    let mut ret_val = SgxStatus::Success;

    let (payload, payload_len) = sgx_utils::slice::call_param_to_ptr(payload);

    unsafe {
        maas_tee_from_bft(
            eid,
            &mut ret_val,
            client_id,
            request_id,
            payload,
            payload_len,
            &mac,
        )
    }
    .into_result()?;

    ret_val.into_result()?;

    Ok(())
}

pub(crate) fn from_client(
    eid: EnclaveId,
    client_id: u64,
    request_id: u64,
    client_pub_key: Ec256PublicKey,
    payload: &[u8],
    mac: Mac128bit,
) -> SgxResult {
    let mut ret_val = SgxStatus::Success;

    let (payload, payload_len) = sgx_utils::slice::call_param_to_ptr(payload);

    unsafe {
        maas_tee_from_client(
            eid,
            &mut ret_val,
            client_id,
            request_id,
            &client_pub_key,
            payload,
            payload_len,
            &mac,
        )
    }
    .into_result()?;

    ret_val.into_result()?;

    Ok(())
}

pub(crate) fn attest(
    eid: EnclaveId,
    local_pub_key: &mut Ec256PublicKey,
    local_ra: &mut [u8],
    local_ra_len: &mut usize,
) -> SgxResult {
    let mut ret_val = SgxStatus::Success;

    let (local_ra, local_ra_max_len) = sgx_utils::slice::call_param_to_ptr_mut(local_ra);

    unsafe {
        maas_tee_attest(
            eid,
            &mut ret_val,
            local_pub_key,
            local_ra,
            local_ra_max_len,
            local_ra_len,
        )
    }
    .into_result()?;

    ret_val.into_result()?;

    Ok(())
}

pub(crate) fn ma_continue(
    eid: EnclaveId,
    remote_pub_key: Ec256PublicKey,
    remote_ra: &[u8],
    local_proposed_secret: &mut [u8],
    local_proposed_secret_len: &mut usize,
    local_proposed_secret_mac: &mut Mac128bit,
) -> SgxResult {
    let mut ret_val = SgxStatus::Success;

    let (remote_ra, remote_ra_len) = sgx_utils::slice::call_param_to_ptr(remote_ra);
    let (local_proposed_secret, local_proposed_secret_max_len) =
        sgx_utils::slice::call_param_to_ptr_mut(local_proposed_secret);

    unsafe {
        maas_tee_ma_continue(
            eid,
            &mut ret_val,
            &remote_pub_key,
            remote_ra,
            remote_ra_len,
            local_proposed_secret,
            local_proposed_secret_max_len,
            local_proposed_secret_len,
            local_proposed_secret_mac,
        )
    }
    .into_result()?;

    ret_val.into_result()?;

    Ok(())
}

pub(crate) fn ma_finish(
    eid: EnclaveId,
    remote_pub_key: Ec256PublicKey,
    remote_ra: &[u8],
    remote_proposed_secret: &[u8],
    remote_proposed_secret_mac: Mac128bit,
) -> SgxResult {
    let mut ret_val = SgxStatus::Success;

    let (remote_ra, remote_ra_len) = sgx_utils::slice::call_param_to_ptr(remote_ra);
    let (remote_proposed_secret, remote_proposed_secret_len) =
        sgx_utils::slice::call_param_to_ptr(remote_proposed_secret);

    unsafe {
        maas_tee_ma_finish(
            eid,
            &mut ret_val,
            &remote_pub_key,
            remote_ra,
            remote_ra_len,
            remote_proposed_secret,
            remote_proposed_secret_len,
            &remote_proposed_secret_mac,
        )
    }
    .into_result()?;

    ret_val.into_result()?;

    Ok(())
}

#[allow(dead_code)] // Used by tests
pub(crate) fn shared_secret_info(eid: EnclaveId, k_anonymity_hash: &mut u8) -> SgxResult {
    let mut ret_val = SgxStatus::Success;

    unsafe { maas_tee_shared_secret_info(eid, &mut ret_val, k_anonymity_hash) }.into_result()?;

    ret_val.into_result()?;

    Ok(())
}

pub(crate) fn attest_shared_key(
    eid: EnclaveId,
    shared_pub_key: &mut Ec256PublicKey,
    local_ra: &mut [u8],
    local_ra_len: &mut usize,
) -> SgxResult {
    let mut ret_val = SgxStatus::Success;

    let (local_ra, local_ra_max_len) = sgx_utils::slice::call_param_to_ptr_mut(local_ra);

    unsafe {
        maas_tee_attest_shared_key(
            eid,
            &mut ret_val,
            shared_pub_key,
            local_ra,
            local_ra_max_len,
            local_ra_len,
        )
    }
    .into_result()?;

    ret_val.into_result()?;

    Ok(())
}
