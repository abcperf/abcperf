use std::{
    cmp::Ordering,
    collections::{HashMap, HashSet},
    fmt::Debug,
    mem,
};

use anyhow::Error;
use blake2::Blake2b;
pub use blake2::Digest;
pub use ed25519_dalek::Signature;
use rand::rngs::OsRng;

use abcperf::{
    atomic_broadcast::{AtomicBroadcast, AtomicBroadcastChannels, AtomicBroadcastConfiguration},
    MessageDestination,
};
use async_trait::async_trait;
use ed25519_dalek::SigningKey;
use relay::ResponseRelay;
use serde::{Deserialize, Serialize};
use shared_ids::{AnyId, ClientId, ReplicaId, RequestId};
use tokio::{select, sync::mpsc, task::JoinHandle};
use tracing::Instrument;

mod relay;

static ED25519_CONTEXT: &[u8] = b"abcperf-client-proxy-sig";

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct SignedResponse<S> {
    messsage: S,
    signature: Signature,
}

impl<S> SignedResponse<S> {
    fn new(messsage: S, signature: Signature) -> Self {
        Self {
            messsage,
            signature,
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub enum CustomReplicaMessage<M, R, S> {
    ReplicaMessage(M),
    RequestBroadcast((ClientId, R)),
    Response(SignedResponse<S>),
}

pub trait ResponseInfo {
    fn client_id(&self) -> ClientId;
    fn request_id(&self) -> RequestId;
    fn hash_with_digest<D: Digest>(&self, digest: &mut D);
}

pub trait ResponseInfoNoClientId {
    fn request_id(&self) -> RequestId;
    fn hash_with_digest<D: Digest>(&self, digest: &mut D);
}

impl<T: ResponseInfoNoClientId> ResponseInfo for (ClientId, T) {
    fn client_id(&self) -> ClientId {
        self.0
    }

    fn request_id(&self) -> RequestId {
        self.1.request_id()
    }

    fn hash_with_digest<D: Digest>(&self, digest: &mut D) {
        digest.update(self.client_id().as_u64().to_be_bytes());
        self.1.hash_with_digest(digest);
    }
}

pub struct ClientProxyAdapter<A: AtomicBroadcast> {
    inner: A,
}

impl<A: AtomicBroadcast> ClientProxyAdapter<A> {
    pub fn new(inner: A) -> Self {
        Self { inner }
    }
}

enum ClientState<R> {
    Empty,
    Filled {
        request_id: RequestId,
        response_state: ResponseState<R>,
    },
}

impl<R> Default for ClientState<R> {
    fn default() -> Self {
        Self::Empty
    }
}

impl<R: ResponseInfo + Eq> ClientState<R> {
    fn local_response(&mut self, response: R) {
        let new_id = if let Self::Filled {
            request_id,
            response_state,
        } = self
        {
            let id = response.request_id();
            match id.cmp(request_id) {
                Ordering::Less => return,
                Ordering::Equal => {
                    response_state.local_response(response);
                    return;
                }
                Ordering::Greater => id,
            }
        } else {
            response.request_id()
        };
        *self = Self::Filled {
            request_id: new_id,
            response_state: ResponseState::from_local(response),
        }
    }

    fn remote_response(&mut self, replica_id: ReplicaId, response: R, signature: Signature) {
        let new_id = if let Self::Filled {
            request_id,
            response_state,
        } = self
        {
            let id = response.request_id();
            match id.cmp(request_id) {
                Ordering::Less => return,
                Ordering::Equal => {
                    response_state.remote_response(replica_id, response, signature);
                    return;
                }
                Ordering::Greater => id,
            }
        } else {
            response.request_id()
        };
        *self = Self::Filled {
            request_id: new_id,
            response_state: ResponseState::from_remote(replica_id, response, signature),
        }
    }

    fn get_response(&mut self, t: u64) -> Option<(R, Box<[Signature]>)> {
        if let Self::Filled {
            request_id: _,
            response_state: response_state_ref,
        } = self
        {
            let mut result = None;
            let response_state = mem::replace(response_state_ref, ResponseState::Empty);
            let response_state = response_state.get_response(t, &mut result);
            *response_state_ref = response_state;
            result
        } else {
            None
        }
    }
}

enum ResponseState<R> {
    Empty,
    Local {
        local: R,
        confirmed_singatures: Vec<Signature>,
        confirmed_replicas: HashSet<ReplicaId>,
    },
    RemoteOnly {
        unconfirmed_signatures: Vec<(ReplicaId, R, Signature)>,
    },
}

impl<R: Eq> ResponseState<R> {
    fn from_local(response: R) -> Self {
        Self::Local {
            local: response,
            confirmed_singatures: Default::default(),
            confirmed_replicas: Default::default(),
        }
    }

    fn from_remote(replica_id: ReplicaId, response: R, signature: Signature) -> Self {
        Self::RemoteOnly {
            unconfirmed_signatures: vec![(replica_id, response, signature)],
        }
    }

    fn local_response(&mut self, response: R) {
        match self {
            ResponseState::Local { .. } => unreachable!(),
            ResponseState::RemoteOnly {
                unconfirmed_signatures,
            } => {
                let mut confirmed_singatures = Vec::new();
                let mut confirmed_replicas = HashSet::new();
                for (id, r, s) in mem::take(unconfirmed_signatures).into_iter() {
                    if r == response && confirmed_replicas.insert(id) {
                        confirmed_singatures.push(s);
                    }
                }
                *self = Self::Local {
                    confirmed_singatures,
                    confirmed_replicas,
                    local: response,
                }
            }
            ResponseState::Empty => *self = Self::from_local(response),
        }
    }

    fn remote_response(&mut self, replica_id: ReplicaId, response: R, signature: Signature) {
        match self {
            ResponseState::Local {
                local,
                confirmed_singatures,
                confirmed_replicas,
            } => {
                if local == &response && confirmed_replicas.insert(replica_id) {
                    confirmed_singatures.push(signature);
                }
            }
            ResponseState::RemoteOnly {
                unconfirmed_signatures,
            } => {
                unconfirmed_signatures.push((replica_id, response, signature));
            }
            ResponseState::Empty => *self = Self::from_remote(replica_id, response, signature),
        }
    }

    fn get_response(self, t: u64, result: &mut Option<(R, Box<[Signature]>)>) -> Self {
        match self {
            ResponseState::Local {
                local,
                confirmed_singatures,
                confirmed_replicas,
            } if confirmed_singatures.len() as u64 >= t => {
                assert_eq!(confirmed_singatures.len(), confirmed_replicas.len());
                *result = Some((local, confirmed_singatures.into()));
                Self::Empty
            }
            this => this,
        }
    }
}

#[async_trait]
impl<A: AtomicBroadcast + Send> AtomicBroadcast for ClientProxyAdapter<A>
where
    A::Transaction: Unpin + Clone + AsRef<RequestId>,
    A::Decision: Unpin + Clone + Eq + ResponseInfo,
{
    type Config = A::Config;

    type ReplicaMessage = CustomReplicaMessage<A::ReplicaMessage, A::Transaction, A::Decision>;

    type Transaction = A::Transaction;

    type Decision = (A::Decision, Box<[Signature]>);

    fn start(
        self,
        config: AtomicBroadcastConfiguration<Self::Config>,
        channels: AtomicBroadcastChannels<Self::ReplicaMessage, Self::Transaction, Self::Decision>,
        ready_for_clients: impl Send + 'static + FnOnce() + Sync,
    ) -> JoinHandle<Result<(), Error>> {
        tokio::spawn(async move {
            let AtomicBroadcastChannels {
                mut incoming_replica_messages,
                outgoing_replica_messages,
                mut requests,
                responses,
            } = channels;

            let priv_key = SigningKey::generate(&mut OsRng::default());
            let t = config.t;

            let (client_requests_send, client_requests_recv) = mpsc::channel(1000);

            let (incoming_send, incoming_recv) = mpsc::channel(1000);
            let (client_responses_send, mut client_responses_recv) =
                mpsc::channel::<A::Decision>(1000);

            let incoming_handler = tokio::spawn({
                let outgoing_replica_messages = outgoing_replica_messages.clone();
                async move {
                    let mut state: HashMap<ClientId, ClientState<<A as AtomicBroadcast>::Decision>> = HashMap::<ClientId, ClientState<A::Decision>>::new();
                    let mut relay = ResponseRelay::<A>::new(outgoing_replica_messages.clone());
                    loop {
                        select! {
                            Some((msg_type, id, m)) = incoming_replica_messages.recv() => {
                                match m {
                                    CustomReplicaMessage::ReplicaMessage(m) => {
                                        let _ = incoming_send.send((msg_type, id, m)).await;
                                    }
                                    CustomReplicaMessage::RequestBroadcast(r) => {
                                        relay.on_remote_request(r.0, *r.1.as_ref(), id).await;
                                        let _ = client_requests_send.try_send(r);
                                    }
                                    CustomReplicaMessage::Response(SignedResponse { messsage, signature }) => {
                                        let state = state.entry(messsage.client_id()).or_default();
                                        state.remote_response(id, messsage, signature);
                                        if let Some(m) = state.get_response(t) {
                                            let _ = responses.send(m).await;
                                        }
                                    }
                                }
                            }
                            Some(r) = client_responses_recv.recv() => {
                                let state = state.entry(r.client_id()).or_default();
                                state.local_response(r.clone());
                                if let Some(m) = state.get_response(t) {
                                    let _ = responses.send(m).await;
                                }
                                let mut hasher = Blake2b::new();
                                r.hash_with_digest(&mut hasher);
                                let sig = priv_key.sign_prehashed(hasher, Some(ED25519_CONTEXT)).unwrap();
                                let client_id = r.client_id();
                                let request_id = r.request_id();
                                let response = SignedResponse::new(r, sig);
                                relay.on_response(client_id, request_id, response).await
                            }
                            Some(r) = requests.recv() => {
                                relay.on_local_request(r.0, *r.1.as_ref());
                                let _ = client_requests_send.try_send(r.clone());
                                match outgoing_replica_messages
                                    .send((
                                        MessageDestination::Broadcast,
                                        CustomReplicaMessage::RequestBroadcast(r),
                                    ))
                                    .await
                                {
                                    Ok(()) => {}
                                    Err(_) => break,
                                }
                            }
                            else => break
                        }
                    }
                }.in_current_span()
            });

            let (outgoing_send, mut outgoing_recv) = mpsc::channel(1000);
            let outgoing_handler = tokio::spawn(
                async move {
                    while let Some((dest, m)) = outgoing_recv.recv().await {
                        match outgoing_replica_messages
                            .send((dest, CustomReplicaMessage::ReplicaMessage(m)))
                            .await
                        {
                            Ok(()) => {}
                            Err(_) => break,
                        }
                    }
                }
                .in_current_span(),
            );

            self.inner
                .start(
                    config,
                    AtomicBroadcastChannels {
                        incoming_replica_messages: incoming_recv,
                        outgoing_replica_messages: outgoing_send,
                        requests: client_requests_recv,
                        responses: client_responses_send,
                    },
                    ready_for_clients,
                )
                .await
                .unwrap()
                .unwrap();

            incoming_handler.await.unwrap();
            outgoing_handler.await.unwrap();
            Ok(())
        })
    }
}
