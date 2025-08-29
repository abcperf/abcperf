use std::fmt::Debug;

use anyhow::{anyhow, Result};
use bytes::Bytes;
use hashbar::{Hashbar, Hasher};
use maas_tee::{enclave::SharedKeyAttestation, PubKey};
use maas_types::ClientRequest;
use serde::{Deserialize, Serialize};
use sgx_crypto::aes::gcm::{Aad, AesGcm, Nonce};
use sgx_types::types::{Key128bit, Mac128bit};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub struct EncrypteTransaction {
    pub(super) payload: Vec<u8>,
    pub(super) mac: Mac128bit,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub struct EncryptedRequest {
    pub(super) payload: Bytes,
    pub(super) mac: Mac128bit,
    pub(super) pubkey: PubKey,
}

impl EncryptedRequest {
    pub fn from_client_request(
        request: ClientRequest,
        enc_key: Key128bit,
        pubkey: PubKey,
    ) -> Result<Self> {
        let ser_client_req = bincode::serialize(&request)?;

        let mut payload = vec![0; ser_client_req.len()];

        let mac = AesGcm::new(
            &enc_key,
            Nonce::zeroed(), /* TODO remove */
            Aad::from(&[]),
        )?
        .encrypt(ser_client_req.as_slice(), &mut payload)?;

        let payload = Bytes::from(payload);

        Ok(Self {
            payload,
            mac,
            pubkey,
        })
    }
}

impl From<EncryptedRequest> for InnerRequest {
    fn from(transaction: EncryptedRequest) -> Self {
        Self::Encrypted(transaction)
    }
}

impl From<EncryptedRequest> for Request {
    fn from(transaction: EncryptedRequest) -> Self {
        InnerRequest::from(transaction).into()
    }
}

impl Hashbar for EncrypteTransaction {
    fn hash<H: Hasher>(&self, hasher: &mut H) {
        hasher.update(&self.payload);
        hasher.update(&self.mac);
    }
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Request(pub(crate) InnerRequest);

#[derive(Debug, Serialize, Deserialize, Clone)]
pub(crate) enum InnerRequest {
    Encrypted(EncryptedRequest),
    RequestRemoteAttestation,
}

impl From<InnerRequest> for Request {
    fn from(inner: InnerRequest) -> Self {
        Self(inner)
    }
}

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Response(pub(crate) InnerResponse);

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) enum InnerResponse {
    Encrypted(EncryptedResponse),
    RemoteAttestation(SharedKeyAttestation),
}

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct EncryptedResponse {
    pub enc_payload: Vec<u8>,
    pub mac: Mac128bit,
}

impl EncryptedResponse {
    pub(crate) fn decrypt(&self, enc_key: Key128bit) -> Result<()> {
        let mut payload_bytes = vec![0; self.enc_payload.len()];

        AesGcm::new(
            &enc_key,
            Nonce::zeroed(), /* TODO remove */
            Aad::from(&[]),
        )
        .unwrap()
        .decrypt(&self.enc_payload, payload_bytes.as_mut_slice(), &self.mac)
        .map_err(|e| anyhow!("sgx error during decrypt: {}", e))?;

        let repsonse: Result<(), ()> = bincode::deserialize(payload_bytes.as_slice())?;

        repsonse.map_err(|_| anyhow!("request failed"))
    }
}

impl From<InnerResponse> for Response {
    fn from(inner: InnerResponse) -> Self {
        Self(inner)
    }
}
