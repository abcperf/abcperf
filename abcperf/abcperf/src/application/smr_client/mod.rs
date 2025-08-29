use std::{
    marker::PhantomData,
    net::{IpAddr, SocketAddr},
    num::NonZeroU64,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
};

use anyhow::{anyhow, Result};
use base64::{alphabet::URL_SAFE, engine::GeneralPurposeConfig};
use reqwest::{
    header::{HeaderValue, CONTENT_TYPE},
    Client, StatusCode,
};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use serde_bytes_repr::{ByteFmtDeserializer, ByteFmtSerializer};
use serde_json::{Deserializer, Serializer};
use shared_ids::{ClientId, ReplicaId, RequestId};
use tokio_util::sync::CancellationToken;

use super::{ApplicationRequest, ApplicationResponse};

pub mod bft;
pub mod fallback;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum SmrClient {
    Bft,
    Fallback,
}

#[derive(Clone)]
#[repr(transparent)]
pub struct CurrentPrimary {
    state: Arc<AtomicU64>,
}

impl Default for CurrentPrimary {
    fn default() -> Self {
        Self::new()
    }
}

impl CurrentPrimary {
    const NONE_STATE: u64 = u64::MAX;
    pub fn new() -> Self {
        let state = Arc::new(AtomicU64::new(Self::NONE_STATE));
        Self { state }
    }

    pub fn set(&self, replica_id: Option<ReplicaId>) {
        let val = if let Some(replica_id) = replica_id {
            let id = replica_id.as_u64();
            assert_ne!(id, Self::NONE_STATE);
            id
        } else {
            Self::NONE_STATE
        };

        self.state.store(val, Ordering::Relaxed)
    }

    pub fn get(&self) -> Option<ReplicaId> {
        let id = self.state.load(Ordering::Relaxed);

        if id != Self::NONE_STATE {
            Some(ReplicaId::from_u64(id))
        } else {
            None
        }
    }
}

pub struct SharedFactory<Req: ApplicationRequest, Resp: ApplicationResponse> {
    replicas: Arc<[(ReplicaId, SocketAddr)]>,
    stop: CancellationToken,
    next_client_id: ClientId,
    shards: NonZeroU64,
    web_client: WebClient<Req, Resp>,
}

impl<Req: ApplicationRequest, Resp: ApplicationResponse> SharedFactory<Req, Resp> {
    pub fn new(
        replicas: Arc<[(ReplicaId, SocketAddr)]>,
        stop: CancellationToken,
        shard_id: u64,
        shards: NonZeroU64,
    ) -> Self {
        assert!(shard_id < shards.get());
        let next_client_id = ClientId::from_u64(shard_id);
        Self {
            replicas,
            stop,
            next_client_id,
            shards,
            web_client: WebClient::default(),
        }
    }

    fn get_next_client_id(&mut self) -> ClientId {
        let id = self.next_client_id;
        self.next_client_id += self.shards.get();
        id
    }

    fn create_shared_client(&mut self) -> SharedClient<Req, Resp> {
        let client_id = self.get_next_client_id();
        let next_request_id = RequestId::from_u64(0);
        let client = self.web_client.clone();

        SharedClient {
            client,
            replicas: self.replicas.clone(),
            stop: self.stop.clone(),
            client_id,
            next_request_id,
        }
    }
}

pub struct SharedClient<Req: ApplicationRequest, Resp: ApplicationResponse> {
    client: WebClient<Req, Resp>,
    replicas: Arc<[(ReplicaId, SocketAddr)]>,
    stop: CancellationToken,
    client_id: ClientId,
    next_request_id: RequestId,
}

impl<Req: ApplicationRequest, Resp: ApplicationResponse> SharedClient<Req, Resp> {
    fn get_next_request_id(&mut self) -> RequestId {
        let id = self.next_request_id;
        self.next_request_id += 1;
        id
    }
}

pub struct WebClient<Request: ApplicationRequest, Response: ApplicationResponse> {
    client: Client,
    phantom_data: PhantomData<(Request, Response)>,
}

impl<Request: ApplicationRequest, Response: ApplicationResponse> Default
    for WebClient<Request, Response>
{
    fn default() -> Self {
        let client = Client::builder()
            .no_gzip() // we want no compression since dummy payloads contain only zeros
            .no_brotli() // we want no compression since dummy payloads contain only zeros
            .no_deflate() // we want no compression since dummy payloads contain only zeros
            .no_proxy() // always use direct connection
            .http2_prior_knowledge()
            .danger_accept_invalid_certs(true) // TODO better validation
            .build()
            .expect("static config always valid");

        Self {
            client,
            phantom_data: PhantomData,
        }
    }
}

impl<Request: ApplicationRequest, Response: ApplicationResponse> Clone
    for WebClient<Request, Response>
{
    fn clone(&self) -> Self {
        Self {
            client: self.client.clone(),
            phantom_data: PhantomData,
        }
    }
}

impl<Request: ApplicationRequest, Response: ApplicationResponse> WebClient<Request, Response> {
    pub async fn request(
        &self,
        client_id: ClientId,
        request_id: RequestId,
        addr: SocketAddr,
        request: Request,
    ) -> Result<Response> {
        let body = to_vec::<Request>(&request);

        let url = match addr.ip() {
            IpAddr::V4(_) => format!(
                "https://{}:{}/{}/{}",
                addr.ip(),
                addr.port(),
                client_id.as_u64(),
                request_id.as_u64()
            ),
            IpAddr::V6(_) => format!(
                "https://[{}]:{}/{}/{}",
                addr.ip(),
                addr.port(),
                client_id.as_u64(),
                request_id.as_u64()
            ),
        };

        let response = self
            .client
            .post(url)
            .header(CONTENT_TYPE, HeaderValue::from_static("application/json"))
            .body(body)
            .send()
            .await?;

        let status_code = response.status();
        if status_code != StatusCode::OK {
            let body = response.text().await?;
            return Err(anyhow!(
                "wrong status code it was {:?} {}",
                status_code,
                body
            ));
        }

        let content_type = response.headers().get(CONTENT_TYPE);
        if content_type != Some(&HeaderValue::from_static("application/json")) {
            return Err(anyhow!("content type not json it was {:?}", content_type));
        }

        let response = response.bytes().await?;

        let response = from_slice::<Response>(&response)?;

        Ok(response)
    }
}

pub fn to_vec<R: Serialize>(value: &R) -> Vec<u8> {
    let mut data = Vec::new();

    let mut serializer = Serializer::new(&mut data);
    let serializer =
        ByteFmtSerializer::base64(&mut serializer, URL_SAFE, GeneralPurposeConfig::new());
    value
        .serialize(serializer)
        .expect("should always be serializable");

    data
}

pub fn from_slice<R>(data: &[u8]) -> Result<R>
where
    R: DeserializeOwned,
{
    let mut deserializer = Deserializer::from_slice(data);
    let deserializer =
        ByteFmtDeserializer::new_base64(&mut deserializer, URL_SAFE, GeneralPurposeConfig::new());
    let value = R::deserialize(deserializer)?;

    Ok(value)
}
