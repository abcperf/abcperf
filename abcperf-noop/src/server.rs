use std::{marker::PhantomData, net::SocketAddr};

use abcperf::{atomic_broadcast::Decision, MessageType};
use abcperf::{config::ClientConfig, MessageDestination, Server};
use abcperf_client_proxy::Signature;
use abcperf_generic_client::cs::{typed::TypedCSTrait, CSTrait};
use abcperf_generic_client::response::ResponseHandler;
use anyhow::{anyhow, Result};
use async_trait::async_trait;
use futures::Future;
use serde::{Deserialize, Serialize};
use shared_ids::{ClientId, ReplicaId, RequestId};
use std::sync::Arc;
use tokio::sync::{mpsc, oneshot, Mutex};
use tracing::Instrument;

use crate::{
    payload::{Payload, ReqPayload, RespPayload},
    MyClientConfig,
};
use minbft::RequestPayload;

type Channel<I> = mpsc::Sender<(ClientId, I)>;

pub trait PayloadType: 'static + Send {
    fn into_base_type(self) -> (ClientId, ReqPayload);
}

impl PayloadType for ((ClientId, ReqPayload), Box<[Signature]>) {
    fn into_base_type(self) -> (ClientId, ReqPayload) {
        self.0
    }
}

impl PayloadType for (ClientId, ReqPayload) {
    fn into_base_type(self) -> (ClientId, ReqPayload) {
        self
    }
}

#[derive(Debug)]
pub struct NoopServer<CS: CSTrait, P: PayloadType> {
    cs: TypedCSTrait<CS, ReqPayload, RespPayload>,
    phantom_data: PhantomData<P>,
}

impl<CS: CSTrait, P: PayloadType> NoopServer<CS, P> {
    pub(crate) fn new(cs: TypedCSTrait<CS, ReqPayload, RespPayload>) -> Self {
        Self {
            cs,
            phantom_data: PhantomData::default(),
        }
    }
}

#[derive(Serialize, Deserialize, Debug)]
pub enum NoopReplicaMessage {}

#[async_trait]
impl<CS: CSTrait, P: PayloadType + Decision> Server for NoopServer<CS, P> {
    /// requests send by clients
    type AlgoRequest = ReqPayload;

    /// responses for clients
    type AlgoResponse = P;

    type Config = MyClientConfig;

    type ReplicaMessage = NoopReplicaMessage;

    async fn run<F: Future<Output = ()>>(
        self,
        config: ClientConfig<MyClientConfig>,
        requests: mpsc::Sender<(ClientId, Self::AlgoRequest)>,
        responses: mpsc::Receiver<P>,
        exit: oneshot::Receiver<()>,
        ready: oneshot::Sender<SocketAddr>,
        local_socket: SocketAddr,
        _replica_send: impl Send + Fn(MessageDestination, Self::ReplicaMessage) -> F,
        _replica_recv: mpsc::Receiver<(MessageType, ReplicaId, Self::ReplicaMessage)>,
    ) {
        let client_handler = Arc::new(ClientHandlerConnections::default());

        let join_handle = tokio::spawn(
            handle_responses(
                responses,
                client_handler.clone(),
                config.client.response_payload,
            )
            .in_current_span(),
        );

        let cs = self.cs.configure_debug();

        let server = cs.server(local_socket);

        let (addr, server) = server.start(
            (requests, client_handler),
            |(requests, client_handler), request| async {
                handle_request(requests, *request.as_ref(), request, client_handler)
                    .await
                    .ok()
                    .flatten()
            },
            exit,
        );

        ready.send(addr).unwrap();

        server.await;

        join_handle.await.unwrap();
    }
}

async fn handle_responses<P: PayloadType>(
    mut incoming: mpsc::Receiver<P>,
    client_handler: Arc<ClientHandlerConnections>,
    response_payload: u64,
) {
    while let Some((client_id, msg)) = incoming.recv().await.map(|m| m.into_base_type()) {
        client_handler
            .handle_response(
                client_id,
                msg.id(),
                RespPayload::new(Payload::new(response_payload as usize)),
            )
            .await;
    }
}

async fn handle_request(
    channel: Channel<ReqPayload>,
    id: ClientId,
    request: ReqPayload,
    client_handler: Arc<ClientHandlerConnections>,
) -> Result<Option<RespPayload>> {
    let (tx, rx) = oneshot::channel();
    client_handler.handle_request(id, request.id(), tx).await;
    channel
        .send((id, request))
        .await
        .map_err(|_| anyhow!("Failed to send"))?;
    Ok(rx.await.ok()) // closed channels are fine (newer request arrived)
}

#[derive(Default, Debug)]
pub(super) struct ClientHandlerConnections {
    handler_connections: Mutex<ResponseHandler<RespPayload>>,
}

impl ClientHandlerConnections {
    pub(super) async fn handle_response(
        &self,
        client_id: ClientId,
        request_id: RequestId,
        payload: RespPayload,
    ) {
        self.handler_connections
            .lock()
            .await
            .response(client_id, request_id, payload);
    }

    pub(super) async fn handle_request(
        &self,
        client_id: ClientId,
        request_id: RequestId,
        channel: oneshot::Sender<RespPayload>,
    ) {
        self.handler_connections
            .lock()
            .await
            .request(client_id, request_id, channel);
    }
}
