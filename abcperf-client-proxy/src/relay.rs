use std::{
    cmp::Ordering,
    collections::{HashMap, HashSet},
    mem,
};

use abcperf::{atomic_broadcast::AtomicBroadcast, MessageDestination};
use derivative::Derivative;
use shared_ids::{ClientId, ReplicaId, RequestId};
use tokio::sync::mpsc;

use crate::{CustomReplicaMessage, SignedResponse};

type Channel<A> = mpsc::Sender<(
    MessageDestination,
    CustomReplicaMessage<
        <A as AtomicBroadcast>::ReplicaMessage,
        <A as AtomicBroadcast>::Transaction,
        <A as AtomicBroadcast>::Decision,
    >,
)>;

pub(super) struct ResponseRelay<A: AtomicBroadcast> {
    clients: HashMap<ClientId, ClientResponseRelay<A>>,
    channel: Channel<A>,
}

impl<A: AtomicBroadcast> ResponseRelay<A> {
    pub(super) fn new(channel: Channel<A>) -> Self {
        Self {
            channel,
            clients: HashMap::new(),
        }
    }

    pub(super) fn on_local_request(&mut self, client_id: ClientId, request_id: RequestId) {
        self.clients
            .entry(client_id)
            .or_default()
            .on_local_request(request_id);
    }

    pub(super) async fn on_remote_request(
        &mut self,
        client_id: ClientId,
        request_id: RequestId,
        remote: ReplicaId,
    ) {
        self.clients
            .entry(client_id)
            .or_default()
            .on_remote_request(&self.channel, request_id, remote)
            .await
    }

    pub(super) async fn on_response(
        &mut self,
        client_id: ClientId,
        request_id: RequestId,
        response: SignedResponse<A::Decision>,
    ) {
        self.clients
            .entry(client_id)
            .or_default()
            .on_response(&self.channel, request_id, response)
            .await
    }
}

#[derive(Derivative)]
#[derivative(Default(bound = ""))]
struct ClientResponseRelay<A: AtomicBroadcast> {
    latest_request: Option<RequestId>,
    state: ClientResponseRelayState<A>,
}

enum ClientResponseRelayState<A: AtomicBroadcast> {
    WatingForResponse {
        to_send_to: HashSet<ReplicaId>,
    },
    CachedResponse {
        already_send_to: HashSet<ReplicaId>,
        response: SignedResponse<A::Decision>,
    },
}

impl<A: AtomicBroadcast> Default for ClientResponseRelayState<A> {
    fn default() -> Self {
        Self::WatingForResponse {
            to_send_to: HashSet::new(),
        }
    }
}

impl<A: AtomicBroadcast> ClientResponseRelay<A> {
    fn current_or_new_request(&mut self, request_id: RequestId) -> bool {
        let request_id = Some(request_id);
        match request_id.cmp(&self.latest_request) {
            Ordering::Less => false,
            Ordering::Equal => true,
            Ordering::Greater => {
                *self = Self {
                    latest_request: request_id,
                    state: ClientResponseRelayState::default(),
                };
                true
            }
        }
    }

    fn on_local_request(&mut self, request_id: RequestId) {
        self.current_or_new_request(request_id);
    }

    async fn on_remote_request(
        &mut self,
        channel: &Channel<A>,
        request_id: RequestId,
        remote: ReplicaId,
    ) {
        if !self.current_or_new_request(request_id) {
            return;
        }
        match &mut self.state {
            ClientResponseRelayState::WatingForResponse { to_send_to } => {
                to_send_to.insert(remote);
            }
            ClientResponseRelayState::CachedResponse {
                already_send_to,
                response,
            } => {
                if already_send_to.insert(remote) {
                    let _ = channel
                        .send((
                            MessageDestination::Unicast(remote),
                            CustomReplicaMessage::Response(response.clone()),
                        ))
                        .await;
                }
            }
        }
    }

    async fn on_response(
        &mut self,
        channel: &Channel<A>,
        request_id: RequestId,
        response: SignedResponse<A::Decision>,
    ) {
        if !self.current_or_new_request(request_id) {
            return;
        }
        if let ClientResponseRelayState::WatingForResponse { to_send_to } =
            mem::take(&mut self.state)
        {
            for dest in to_send_to.iter().copied() {
                let _ = channel
                    .send((
                        MessageDestination::Unicast(dest),
                        CustomReplicaMessage::Response(response.clone()),
                    ))
                    .await;
            }
            self.state = ClientResponseRelayState::CachedResponse {
                already_send_to: to_send_to,
                response,
            };
        } else {
            unreachable!()
        };
    }
}
