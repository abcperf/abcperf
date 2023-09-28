use std::{fmt::Debug, sync::Arc};

use abcperf_client_proxy::ResponseInfoNoClientId;
use abcperf_generic_client::Generate;
use anyhow::Result;
use minbft::RequestPayload;
use serde::{Deserialize, Serialize};
use shared_ids::{AnyId, ClientId, IdIter, RequestId};

use crate::MyClientConfig;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ReqPayload {
    client_id: ClientId,
    request_id: RequestId,
    payload: Payload,
}

impl RequestPayload for ReqPayload {
    fn id(&self) -> RequestId {
        self.request_id
    }

    fn verify(&self, _id: ClientId) -> Result<()> {
        Ok(())
    }
}

impl AsRef<RequestId> for ReqPayload {
    fn as_ref(&self) -> &RequestId {
        &self.request_id
    }
}

impl AsRef<ClientId> for ReqPayload {
    fn as_ref(&self) -> &ClientId {
        &self.client_id
    }
}

impl ResponseInfoNoClientId for ReqPayload {
    fn request_id(&self) -> RequestId {
        self.request_id
    }

    fn hash_with_digest<D: abcperf_client_proxy::Digest>(&self, digest: &mut D) {
        digest.update(self.request_id.as_u64().to_be_bytes());
        digest.update(&self.payload)
    }
}

impl Generate<MyClientConfig> for ReqPayload {
    type Generator = ReqPayloadGenerator;

    fn generator(config: &MyClientConfig, client_id: ClientId) -> Self::Generator {
        ReqPayloadGenerator {
            client_id,
            id_gen: IdIter::default(),
            request_payload: config.request_payload,
        }
    }
}

pub struct ReqPayloadGenerator {
    client_id: ClientId,
    id_gen: IdIter<RequestId>,
    request_payload: u64,
}

impl Iterator for ReqPayloadGenerator {
    type Item = ReqPayload;

    fn next(&mut self) -> Option<Self::Item> {
        let payload = ReqPayload {
            client_id: self.client_id,
            request_id: self
                .id_gen
                .next()
                .expect("we should not run out of request ids"),
            payload: Payload::new(self.request_payload as usize),
        };
        Some(payload)
    }
}

#[repr(transparent)]
#[derive(Deserialize, Serialize, PartialEq, Eq, Clone)]
#[serde(transparent)]
pub(super) struct Payload {
    #[serde(with = "serde_bytes")]
    payload: Box<[u8]>,
}

impl Debug for Payload {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Payload")
            .field("size", &self.payload.len())
            .finish()
    }
}

impl Payload {
    pub(super) fn new(size: usize) -> Self {
        Self {
            payload: vec![0; size].into_boxed_slice(),
        }
    }
}

impl AsRef<[u8]> for Payload {
    fn as_ref(&self) -> &[u8] {
        &self.payload
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RespPayload {
    payload: Arc<Payload>,
}

impl RespPayload {
    pub(super) fn new(payload: Payload) -> Self {
        Self {
            payload: Arc::new(payload),
        }
    }
}
