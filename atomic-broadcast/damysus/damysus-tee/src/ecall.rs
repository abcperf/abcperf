use damysus_tee_shared::{
    AccumulateError, AccumulateResult, AddPeerError, AddPeerResult, GetPublicKeyError,
    GetPublicKeyResult, InitError, InitResult, PrepareError, PrepareResult, ReSignLastPrepareError,
    ReSignLastPrepareResult,
};
use sgx_types::{
    error::SgxStatus,
    types::{Ec256PublicKey, EnclaveId},
};
use sgx_utils::{SgxResult, SgxStatusExt};

use crate::DamysusEnclave;

extern "C" {
    fn damysus_tee_init(
        eid: EnclaveId,
        ret_val: *mut u8,
        me: u64,
        n: u64,
        quorum: u64,
    ) -> SgxStatus;

    fn damysus_tee_get_public_key(
        eid: EnclaveId,
        ret_val: *mut u8,
        pub_key: *mut Ec256PublicKey,
    ) -> SgxStatus;

    fn damysus_tee_add_peer(
        eid: EnclaveId,
        ret_val: *mut u8,
        id: u64,
        pub_key: *const Ec256PublicKey,
    ) -> SgxStatus;

    fn damysus_tee_accumulate(
        eid: EnclaveId,
        ret_val: *mut u8,
        commitments: *const u8,
        commitments_len: usize,
        signed_commitment: *mut u8,
        signed_commitment_max_len: usize,
        signed_commitment_len: *mut usize,
    ) -> SgxStatus;

    fn damysus_tee_prepare(
        eid: EnclaveId,
        ret_val: *mut u8,
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
    ) -> SgxStatus;

    fn damysus_tee_re_sign_last_prepared(
        eid: EnclaveId,
        ret_val: *mut u8,
        signed_commitment: *mut u8,
        signed_commitment_max_len: usize,
        signed_commitment_len: *mut usize,
    ) -> SgxStatus;
}

macro_rules! enclave_fn {
    ($self:ident.$fn:ident($($arg:expr),*) -> $ret:ty) => {{
        let mut ret_val = u8::MAX;

        unsafe { $fn($self.eid(), &mut ret_val $(, $arg)*) }.into_result()?;

        Ok(<$ret>::from_repr(ret_val).expect("enclave should only return valid results").into())
    }};
}

impl DamysusEnclave {
    pub(crate) fn init(&self, me: u64, n: u64, quorum: u64) -> SgxResult<Result<(), InitError>> {
        enclave_fn!(self.damysus_tee_init(me, n, quorum) -> InitResult)
    }

    pub(crate) fn get_public_key(&self) -> SgxResult<Result<Ec256PublicKey, GetPublicKeyError>> {
        let mut pub_key = Ec256PublicKey::default();

        let result = self.inner_get_public_key(&mut pub_key)?;

        Ok(result.map(|()| (pub_key)))
    }

    fn inner_get_public_key(
        &self,
        pub_key: &mut Ec256PublicKey,
    ) -> SgxResult<Result<(), GetPublicKeyError>> {
        enclave_fn!(self.damysus_tee_get_public_key(pub_key) -> GetPublicKeyResult)
    }

    pub(crate) fn add_peer(
        &self,
        id: u64,
        pub_key: Ec256PublicKey,
    ) -> SgxResult<Result<(), AddPeerError>> {
        self.inner_add_peer(id, &pub_key)
    }

    fn inner_add_peer(
        &self,
        id: u64,
        pub_key: &Ec256PublicKey,
    ) -> SgxResult<Result<(), AddPeerError>> {
        enclave_fn!(self.damysus_tee_add_peer(id, pub_key) -> AddPeerResult)
    }

    pub(crate) fn accumulate(
        &self,
        commitments: &[u8],
        signed_commitment_max_len: usize,
    ) -> SgxResult<Result<Box<[u8]>, AccumulateError>> {
        let mut signed_commitment = vec![0; signed_commitment_max_len];
        let mut signed_commitment_len = 0;

        let result = self.inner_accumulate(
            commitments,
            &mut signed_commitment,
            &mut signed_commitment_len,
        )?;

        signed_commitment.truncate(signed_commitment_len);

        Ok(result.map(|()| signed_commitment.into_boxed_slice()))
    }

    fn inner_accumulate(
        &self,
        commitments: &[u8],
        signed_commitment: &mut [u8],
        signed_commitment_len: &mut usize,
    ) -> SgxResult<Result<(), AccumulateError>> {
        let (commitments, commitments_len) = sgx_utils::slice::call_param_to_ptr(commitments);

        let (signed_commitment, signed_commitment_max_len) =
            sgx_utils::slice::call_param_to_ptr_mut(signed_commitment);

        enclave_fn!(self.damysus_tee_accumulate(
            commitments,
            commitments_len,
            signed_commitment,
            signed_commitment_max_len,
            signed_commitment_len) -> AccumulateResult)
    }

    pub(crate) fn prepare(
        &self,
        block_view: u64,
        block_hash: &[u8],
        block_justification: &[u8],
        block_parent: &[u8],
        justification_block_hash: &[u8],
        signed_commitment_max_len: usize,
    ) -> SgxResult<Result<Box<[u8]>, PrepareError>> {
        let mut signed_commitment = vec![0; signed_commitment_max_len];
        let mut signed_commitment_len = 0;

        let result = self.inner_prepare(
            block_view,
            block_hash,
            block_justification,
            block_parent,
            justification_block_hash,
            &mut signed_commitment,
            &mut signed_commitment_len,
        )?;

        signed_commitment.truncate(signed_commitment_len);

        Ok(result.map(|()| signed_commitment.into_boxed_slice()))
    }

    #[allow(clippy::too_many_arguments)]
    fn inner_prepare(
        &self,
        block_view: u64,
        block_hash: &[u8],
        block_justification: &[u8],
        block_parent: &[u8],
        justification_block_hash: &[u8],
        signed_commitment: &mut [u8],
        signed_commitment_len: &mut usize,
    ) -> SgxResult<Result<(), PrepareError>> {
        let (block_hash, block_hash_len) = sgx_utils::slice::call_param_to_ptr(block_hash);
        let (block_justification, block_justification_len) =
            sgx_utils::slice::call_param_to_ptr(block_justification);
        let (block_parent, block_parent_len) = sgx_utils::slice::call_param_to_ptr(block_parent);
        let (justification_block_hash, justification_block_hash_len) =
            sgx_utils::slice::call_param_to_ptr(justification_block_hash);
        let (signed_commitment, signed_commitment_max_len) =
            sgx_utils::slice::call_param_to_ptr_mut(signed_commitment);

        enclave_fn!(self.damysus_tee_prepare(        block_view,
            block_hash,
            block_hash_len,
            block_justification,
            block_justification_len,
            block_parent,
            block_parent_len,
            justification_block_hash,
            justification_block_hash_len,
            signed_commitment,
            signed_commitment_max_len,
            signed_commitment_len) -> PrepareResult)
    }

    pub(crate) fn re_sign_last_prepared(
        &self,
        signed_commitment_max_len: usize,
    ) -> SgxResult<Result<Box<[u8]>, ReSignLastPrepareError>> {
        let mut signed_commitment = vec![0; signed_commitment_max_len];
        let mut signed_commitment_len = 0;

        let result =
            self.inner_re_sign_last_prepared(&mut signed_commitment, &mut signed_commitment_len)?;

        signed_commitment.truncate(signed_commitment_len);

        Ok(result.map(|()| signed_commitment.into_boxed_slice()))
    }

    fn inner_re_sign_last_prepared(
        &self,
        signed_commitment: &mut [u8],
        signed_commitment_len: &mut usize,
    ) -> SgxResult<Result<(), ReSignLastPrepareError>> {
        let (signed_commitment, signed_commitment_max_len) =
            sgx_utils::slice::call_param_to_ptr_mut(signed_commitment);

        enclave_fn!(self.damysus_tee_re_sign_last_prepared(signed_commitment, signed_commitment_max_len, signed_commitment_len) -> ReSignLastPrepareResult)
    }
}
