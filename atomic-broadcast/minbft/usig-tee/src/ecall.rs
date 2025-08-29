use sgx_types::{
    error::SgxStatus,
    types::{Ec256PublicKey, Ec256Signature, EnclaveId},
};
use sgx_utils::{SgxResult, SgxStatusExt};

extern "C" {

    // The ECALL for creating a signature
    fn usig_tee_sign(
        eid: EnclaveId,
        retval: *mut SgxStatus,
        message: *const u8,
        message_len: usize,
        signature: *mut Ec256Signature,
        counter: *mut u64,
    ) -> SgxStatus;

    // The ECALL for performing an attestation
    fn usig_tee_attest(
        eid: EnclaveId,
        retval: *mut SgxStatus,
        pubkey: *mut Ec256PublicKey,
        counter: *mut u64,
        cert_der: *mut u8,
        cert_der_max_len: usize,
        cert_der_len: *mut usize,
    ) -> SgxStatus;
}

pub(crate) fn sign(eid: EnclaveId, message: &[u8]) -> SgxResult<(Ec256Signature, u64)> {
    let mut retval = SgxStatus::Success;

    let (message, message_len) = sgx_utils::slice::call_param_to_ptr(message);

    let mut signature = Ec256Signature::default();
    let mut counter = 0;

    unsafe {
        usig_tee_sign(
            eid,
            &mut retval,
            message,
            message_len,
            &mut signature,
            &mut counter,
        )
    }
    .into_result()?;

    retval.into_result()?;

    Ok((signature, counter))
}

pub(crate) fn attest(
    eid: EnclaveId,
    pubkey: &mut Ec256PublicKey,
    counter: &mut u64,
    cert_der: &mut [u8],
    cert_der_len: &mut usize,
) -> SgxResult {
    let mut retval = SgxStatus::Success;

    let (cert_der, cert_der_max_len) = sgx_utils::slice::call_param_to_ptr_mut(cert_der);

    unsafe {
        usig_tee_attest(
            eid,
            &mut retval,
            pubkey,
            counter,
            cert_der,
            cert_der_max_len,
            cert_der_len,
        )
    }
    .into_result()?;

    retval.into_result()?;

    Ok(())
}
