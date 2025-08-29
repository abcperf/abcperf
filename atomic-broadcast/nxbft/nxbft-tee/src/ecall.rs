use crate::NxbftEnclave;
use nxbft_tee_shared::{
    CompletePeerHandshakeError, CompletePeerHandshakeResult, EnclaveAutomataFFIError,
    EnclaveAutomataFFIResult, InitPeerHandshakeError, InitPeerHandshakeResult, IsReadyResult,
};
use sgx_types::{
    error::SgxStatus,
    types::{Ec256PublicKey, Ec256Signature, EnclaveId, Mac128bit},
};
use sgx_utils::{SgxResult, SgxStatusExt};

extern "C" {
    fn nxbft_tee_init(eid: EnclaveId, ret_val: *mut u8, n: u64) -> SgxStatus;

    // The ECALL for performing an attestation
    fn nxbft_tee_attest(
        eid: EnclaveId,
        ret_val: *mut u8,
        local_pub_key: *mut Ec256PublicKey,
        local_ra: *mut u8,
        local_ra_max_len: usize,
        local_ra_len: *mut usize,
    ) -> SgxStatus;

    // The ECALL for continuing a ma
    fn nxbft_tee_init_peer_handshake(
        eid: EnclaveId,
        ret_val: *mut u8,
        peer_id: u64,
        remote_pub_key: *const Ec256PublicKey,
        remote_ra: *const u8,
        remote_ra_len: usize,
        local_proposed_secret: *mut u8,
        local_proposed_secret_max_len: usize,
        local_proposed_secret_len: *mut usize,
        local_proposed_secret_mac: *mut Mac128bit,
    ) -> SgxStatus;

    // The ECALL for finishing a ma
    fn nxbft_tee_complete_peer_handshake(
        eid: EnclaveId,
        ret_val: *mut u8,
        peer_id: u64,
        remote_proposed_secret: *const u8,
        remote_proposed_secret_len: usize,
        remote_proposed_secret_mac: *const Mac128bit,
    ) -> SgxStatus;

    fn nxbft_tee_is_ready(eid: EnclaveId, ret_val: *mut u8) -> SgxStatus;

    // The ECALL for creating a signature
    fn nxbft_tee_sign(
        eid: EnclaveId,
        ret_val: *mut u8,
        payload: *const u8,
        payload_len: usize,
        signature: *mut Ec256Signature,
        counter: *mut u64,
    ) -> SgxStatus;

    fn nxbft_tee_toss(
        eid: EnclaveId,
        ret_val: *mut u8,
        proof: *const u8,
        proof_len: usize,
        peer_id: *mut u64,
    ) -> SgxStatus;

    fn nxbft_tee_export(
        eid: EnclaveId,
        ret_val: *mut u8,
        export: *mut u8,
        export_max_len: usize,
        export_len: *mut usize,
    ) -> SgxStatus;

    fn nxbft_tee_recover(
        eid: EnclaveId,
        ret_val: *mut u8,
        export: *const u8,
        export_len: usize,
    ) -> SgxStatus;

    fn nxbft_tee_replace_peer(
        eid: EnclaveId,
        ret_val: *mut u8,
        peer_id: u64,
        remote_pub_key: *const Ec256PublicKey,
        remote_ra: *const u8,
        remote_ra_len: usize,
        max_round: u64,
    ) -> SgxStatus;

}

macro_rules! enclave_fn {
    ($self:ident.$fn:ident($($arg:expr),*) -> $ret:ty) => {{
        let mut ret_val = u8::MAX;

        unsafe { $fn($self.eid(), &mut ret_val $(, $arg)*) }.into_result()?;

        Ok(<$ret>::from_repr(ret_val).expect("enclave should only return valid results").into())
    }};
}

type AttestRes<T = ()> = SgxResult<Result<T, EnclaveAutomataFFIError>>;
type InitPeerHandshakeRes<T = ()> = SgxResult<Result<T, InitPeerHandshakeError>>;

impl NxbftEnclave {
    pub(crate) fn init(&self, n: u64) -> SgxResult<Result<(), EnclaveAutomataFFIError>> {
        enclave_fn!(self.nxbft_tee_init(n) -> EnclaveAutomataFFIResult)
    }

    fn inner_attest(
        &self,
        local_pub_key: &mut Ec256PublicKey,
        local_ra: &mut [u8],
        local_ra_len: &mut usize,
    ) -> AttestRes {
        let (local_ra, local_ra_max_len) = sgx_utils::slice::call_param_to_ptr_mut(local_ra);

        enclave_fn!(self.nxbft_tee_attest(local_pub_key, local_ra, local_ra_max_len, local_ra_len) -> EnclaveAutomataFFIResult)
    }

    pub(crate) fn attest(&self, local_ra_max_len: usize) -> AttestRes<(Ec256PublicKey, Box<[u8]>)> {
        let mut local_pub_key = Ec256PublicKey::default();
        let mut local_ra = vec![0; local_ra_max_len];
        let mut local_ra_len = 0;

        let result = self.inner_attest(&mut local_pub_key, &mut local_ra, &mut local_ra_len)?;

        local_ra.truncate(local_ra_len);

        Ok(result.map(|()| (local_pub_key, local_ra.into_boxed_slice())))
    }

    fn inner_init_peer_handshake(
        &self,
        peer_id: u64,
        remote_pub_key: Ec256PublicKey,
        remote_ra: &[u8],
        local_proposed_secret: &mut [u8],
        local_proposed_secret_len: &mut usize,
        local_proposed_secret_mac: &mut Mac128bit,
    ) -> InitPeerHandshakeRes {
        let (remote_ra, remote_ra_len) = sgx_utils::slice::call_param_to_ptr(remote_ra);
        let (local_proposed_secret, local_proposed_secret_max_len) =
            sgx_utils::slice::call_param_to_ptr_mut(local_proposed_secret);

        enclave_fn!(self.nxbft_tee_init_peer_handshake(
            peer_id,
            &remote_pub_key,
            remote_ra,
            remote_ra_len,
            local_proposed_secret,
            local_proposed_secret_max_len,
            local_proposed_secret_len,
            local_proposed_secret_mac
        ) -> InitPeerHandshakeResult)
    }

    pub(crate) fn init_peer_handshake(
        &self,
        peer_id: u64,
        remote_pub_key: Ec256PublicKey,
        remote_ra: &[u8],
        local_proposed_secret_max_len: usize,
    ) -> InitPeerHandshakeRes<(Box<[u8]>, Mac128bit)> {
        let mut local_proposed_secret = vec![0; local_proposed_secret_max_len];
        let mut local_proposed_secret_len = 0;
        let mut local_proposed_secret_mac = Mac128bit::default();

        let result = self.inner_init_peer_handshake(
            peer_id,
            remote_pub_key,
            remote_ra,
            &mut local_proposed_secret,
            &mut local_proposed_secret_len,
            &mut local_proposed_secret_mac,
        )?;

        local_proposed_secret.truncate(local_proposed_secret_len);

        Ok(result.map(|()| {
            (
                local_proposed_secret.into_boxed_slice(),
                local_proposed_secret_mac,
            )
        }))
    }

    pub(crate) fn complete_peer_handshake(
        &self,
        peer_id: u64,
        remote_proposed_secret: &[u8],
        remote_proposed_secret_mac: Mac128bit,
    ) -> SgxResult<Result<(), CompletePeerHandshakeError>> {
        let (remote_proposed_secret, remote_proposed_secret_len) =
            sgx_utils::slice::call_param_to_ptr(remote_proposed_secret);

        enclave_fn!(self.nxbft_tee_complete_peer_handshake(
            peer_id,
            remote_proposed_secret,
            remote_proposed_secret_len,
            &remote_proposed_secret_mac
        ) -> CompletePeerHandshakeResult)
    }

    pub(crate) fn is_ready(&self) -> SgxResult<IsReadyResult> {
        enclave_fn!(self.nxbft_tee_is_ready() -> IsReadyResult)
    }

    fn inner_sign(
        &self,
        message: &[u8],
        signature: &mut Ec256Signature,
        counter: &mut u64,
    ) -> SgxResult<Result<(), EnclaveAutomataFFIError>> {
        let (message, message_len) = sgx_utils::slice::call_param_to_ptr(message);

        enclave_fn!(self.nxbft_tee_sign(message, message_len, signature, counter) -> EnclaveAutomataFFIResult)
    }

    pub(crate) fn sign(
        &self,
        message: &[u8],
    ) -> SgxResult<Result<(Ec256Signature, u64), EnclaveAutomataFFIError>> {
        let mut signature = Ec256Signature::default();
        let mut counter = 0;

        let result = self.inner_sign(message, &mut signature, &mut counter)?;

        Ok(result.map(|()| (signature, counter)))
    }

    fn inner_toss(
        &self,
        proof: &[u8],
        peer_id: &mut u64,
    ) -> SgxResult<Result<(), EnclaveAutomataFFIError>> {
        let (proof, proof_len) = sgx_utils::slice::call_param_to_ptr(proof);

        enclave_fn!(self.nxbft_tee_toss(proof, proof_len, peer_id) -> EnclaveAutomataFFIResult)
    }

    pub(crate) fn toss(&self, proof: &[u8]) -> SgxResult<Result<u64, EnclaveAutomataFFIError>> {
        let mut peer_id = 0;

        let result = self.inner_toss(proof, &mut peer_id)?;

        Ok(result.map(|()| (peer_id)))
    }

    fn inner_export(
        &self,
        export: &mut [u8],
        export_len: &mut usize,
    ) -> SgxResult<Result<(), EnclaveAutomataFFIError>> {
        let (export, export_max_len) = sgx_utils::slice::call_param_to_ptr_mut(export);

        enclave_fn!(self.nxbft_tee_export(export, export_max_len, export_len) -> EnclaveAutomataFFIResult)
    }

    pub(crate) fn export(
        &self,
        export_max_len: usize,
    ) -> SgxResult<Result<Box<[u8]>, EnclaveAutomataFFIError>> {
        let mut export = vec![0; export_max_len];
        let mut export_len = 0;

        let result = self.inner_export(&mut export, &mut export_len)?;

        export.truncate(export_len);

        Ok(result.map(|()| export.into_boxed_slice()))
    }

    pub(crate) fn recover(&self, export: &[u8]) -> SgxResult<Result<(), EnclaveAutomataFFIError>> {
        let (export, export_len) = sgx_utils::slice::call_param_to_ptr(export);

        enclave_fn!(self.nxbft_tee_recover(export, export_len) -> EnclaveAutomataFFIResult)
    }

    pub(crate) fn replace_peer(
        &self,
        peer_id: u64,
        remote_pub_key: Ec256PublicKey,
        remote_ra: &[u8],
        max_round: u64,
    ) -> SgxResult<Result<(), EnclaveAutomataFFIError>> {
        let (remote_ra, remote_ra_len) = sgx_utils::slice::call_param_to_ptr(remote_ra);

        enclave_fn!(self.nxbft_tee_replace_peer(peer_id, &remote_pub_key, remote_ra, remote_ra_len, max_round) -> EnclaveAutomataFFIResult)
    }
}
