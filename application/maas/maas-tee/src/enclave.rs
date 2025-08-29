use once_cell::sync::OnceCell;
use remote_attestation::include_enclave;
use remote_attestation_shared::types::PubKey;
use serde::{Deserialize, Serialize};
use sgx_types::types::{Ec256PublicKey, Mac128bit};
use sgx_urts::enclave::SgxEnclave;
use sgx_utils::{SgxResult, SgxStatusExt};
use shared_ids::{ClientId, RequestId};

use crate::ecall;

include_enclave!("../../maas-tee-enclave/maas_tee.signed.so");

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct SharedKeyAttestation {
    pub shared_pub_key: PubKey,
    #[serde(with = "serde_bytes")]
    pub(crate) intel_report: Box<[u8]>,
}

impl SharedKeyAttestation {
    pub(crate) fn new(shared_key: impl Into<PubKey>, intel_report: impl Into<Box<[u8]>>) -> Self {
        Self {
            shared_pub_key: shared_key.into(),
            intel_report: intel_report.into(),
        }
    }

    pub fn verify(&self) -> SgxResult {
        // TODO RA verify
        Ok(())
    }
}

/// The struct that defines an attestation
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct MAttestation {
    pub pub_key: PubKey,
    #[serde(with = "serde_bytes")]
    pub(crate) intel_report: Box<[u8]>,
}

impl MAttestation {
    pub(crate) fn new(pub_key: impl Into<PubKey>, intel_report: impl Into<Box<[u8]>>) -> Self {
        Self {
            pub_key: pub_key.into(),
            intel_report: intel_report.into(),
        }
    }

    pub fn verify(&self) -> SgxResult {
        // TODO RA verify
        Ok(())
    }
}

/// The struct that defines an attestation
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ProposedSecret {
    pub(crate) attestation: MAttestation,
    #[serde(with = "serde_bytes")]
    pub(crate) secret: Box<[u8]>,
    pub(crate) mac: Mac128bit,
}

impl ProposedSecret {
    pub(crate) fn new(
        attestation: MAttestation,
        secret: impl Into<Box<[u8]>>,
        mac: Mac128bit,
    ) -> Self {
        Self {
            attestation,
            secret: secret.into(),
            mac,
        }
    }
}

pub struct Enclave {
    enclave: SgxEnclave,
    my_attestation: OnceCell<MAttestation>,
    shared_attestation: OnceCell<SharedKeyAttestation>,
}

impl Enclave {
    pub fn new(id: usize) -> SgxResult<Self> {
        // call sgx_create_enclave to initialize an enclave instance
        // Debug Support: set 2nd parameter to 1
        let debug = true;

        let enclave = SgxEnclave::create(ENCLAVE_PATH.as_path(), debug).into_result()?;

        let enclave = Self {
            enclave,
            my_attestation: OnceCell::new(),
            shared_attestation: OnceCell::new(),
        };

        enclave.init(id)?;

        Ok(enclave)
    }

    fn init(&self, id: usize) -> SgxResult {
        crate::ecall::init(self.enclave.eid(), id)
    }

    pub fn from_bft(
        &self,
        client_id: ClientId,
        request_id: RequestId,
        payload: &[u8],
        mac: Mac128bit,
    ) -> SgxResult {
        crate::ecall::from_bft(
            self.enclave.eid(),
            client_id.as_u64(),
            request_id.as_u64(),
            payload,
            mac,
        )
    }

    pub fn from_client(
        &self,
        client_id: ClientId,
        request_id: RequestId,
        pub_key: PubKey,
        payload: &[u8],
        mac: Mac128bit,
    ) -> SgxResult {
        crate::ecall::from_client(
            self.enclave.eid(),
            client_id.as_u64(),
            request_id.as_u64(),
            pub_key.into(),
            payload,
            mac,
        )
    }

    fn generate_attestation(&self) -> SgxResult<MAttestation> {
        let mut local_pub_key = Ec256PublicKey::default();
        let mut local_ra = [0u8; 4096];
        let mut local_ra_len = 0;

        ecall::attest(
            self.enclave.eid(),
            &mut local_pub_key,
            &mut local_ra,
            &mut local_ra_len,
        )?;
        let local_ra = &local_ra[..local_ra_len];

        Ok(MAttestation::new(local_pub_key, local_ra))
    }

    fn generate_shared_attestation(&self) -> SgxResult<SharedKeyAttestation> {
        let mut shared_pub_key = Ec256PublicKey::default();
        let mut local_ra = [0u8; 4096];
        let mut local_ra_len = 0;

        ecall::attest_shared_key(
            self.enclave.eid(),
            &mut shared_pub_key,
            &mut local_ra,
            &mut local_ra_len,
        )?;
        let local_ra = &local_ra[..local_ra_len];

        Ok(SharedKeyAttestation::new(shared_pub_key, local_ra))
    }

    pub fn shared_attestation(&self) -> SgxResult<&SharedKeyAttestation> {
        self.shared_attestation
            .get_or_try_init(|| self.generate_shared_attestation())
    }

    pub fn attestation(&self) -> SgxResult<&MAttestation> {
        self.my_attestation
            .get_or_try_init(|| self.generate_attestation())
    }

    pub fn ma_continue(&self, remote_ra: MAttestation) -> SgxResult<ProposedSecret> {
        let mut local_proposed_secret_mac = Mac128bit::default();
        let mut local_proposed_secret = [0u8; 4096];
        let mut local_proposed_secret_len = 0;

        ecall::ma_continue(
            self.enclave.eid(),
            remote_ra.pub_key.into(),
            &remote_ra.intel_report,
            &mut local_proposed_secret,
            &mut local_proposed_secret_len,
            &mut local_proposed_secret_mac,
        )?;
        let local_proposed_secret = &local_proposed_secret[..local_proposed_secret_len];

        Ok(ProposedSecret::new(
            self.attestation()?.clone(),
            local_proposed_secret,
            local_proposed_secret_mac,
        ))
    }

    pub fn ma_finish(&self, remote_proposed_secret: ProposedSecret) -> SgxResult {
        ecall::ma_finish(
            self.enclave.eid(),
            remote_proposed_secret.attestation.pub_key.into(),
            &remote_proposed_secret.attestation.intel_report,
            &remote_proposed_secret.secret,
            remote_proposed_secret.mac,
        )
    }

    #[allow(dead_code)] // Used by tests
    pub(crate) fn shared_secret_info(&self) -> SgxResult<u8> {
        let mut k_anonymity_hash = 0;

        ecall::shared_secret_info(self.enclave.eid(), &mut k_anonymity_hash)?;

        Ok(k_anonymity_hash)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn start_enclave() {
        Enclave::new(0).unwrap();
    }

    #[test]
    fn start_ma() {
        let enclave = Enclave::new(0).unwrap();
        enclave.generate_attestation().unwrap();
    }

    #[test]
    fn continue_ma_self() {
        let enclave = Enclave::new(0).unwrap();
        let ma = enclave.generate_attestation().unwrap();
        enclave.ma_continue(ma).unwrap();
    }

    #[test]
    fn continue_ma_other() {
        let enclave = Enclave::new(0).unwrap();
        let ma = enclave.generate_attestation().unwrap();

        let enclave = Enclave::new(1).unwrap();
        enclave.ma_continue(ma).unwrap();
    }

    #[test]
    fn continue_ma_cross() {
        let enclave_0 = Enclave::new(0).unwrap();
        let enclave_1 = Enclave::new(1).unwrap();

        enclave_0
            .ma_continue(enclave_1.generate_attestation().unwrap())
            .unwrap();
        enclave_1
            .ma_continue(enclave_0.generate_attestation().unwrap())
            .unwrap();
    }

    #[test]
    fn continue_ma_multi() {
        let enclaves: Vec<_> = (0..5).map(|i| Enclave::new(i).unwrap()).collect();

        let attestations: Vec<_> = enclaves
            .iter()
            .map(|e| e.generate_attestation().unwrap())
            .collect();

        for e in enclaves {
            for a in &attestations {
                e.ma_continue(a.clone()).unwrap();
            }
        }
    }

    #[test]
    fn continue_ma_finish_2() {
        let enclave_0 = Enclave::new(0).unwrap();
        let enclave_1 = Enclave::new(1).unwrap();

        enclave_0
            .ma_finish(
                enclave_1
                    .ma_continue(enclave_0.generate_attestation().unwrap())
                    .unwrap(),
            )
            .unwrap();
        enclave_1
            .ma_finish(
                enclave_0
                    .ma_continue(enclave_1.generate_attestation().unwrap())
                    .unwrap(),
            )
            .unwrap();
    }

    #[test]
    fn continue_ma_finish_multi() {
        let enclaves: Vec<_> = (0..5).map(|i| Enclave::new(i).unwrap()).collect();

        let attestations: Vec<_> = enclaves
            .iter()
            .map(|e| (e.generate_attestation().unwrap(), e))
            .collect();

        for e in &enclaves {
            for (a, r_e) in &attestations {
                r_e.ma_finish(e.ma_continue(a.clone()).unwrap()).unwrap();
            }
        }

        let first = enclaves.first().unwrap().shared_secret_info().unwrap();
        for e in enclaves {
            assert_eq!(first, e.shared_secret_info().unwrap());
        }
    }
}
