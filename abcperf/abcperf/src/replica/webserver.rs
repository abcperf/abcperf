use std::{cmp::Ordering, mem, net::SocketAddr};

use bytes::Bytes;
use http::{header::CONTENT_TYPE, HeaderValue};
use scc::HashMap;

use futures::{stream, Stream, StreamExt};
use shared_ids::{ClientId, RequestId};
use std::sync::Arc;
use tokio::{
    sync::{mpsc, oneshot},
    task::JoinHandle,
};
use tokio_stream::wrappers::ReceiverStream;
use tokio_util::sync::CancellationToken;
use tracing::{error, info, Instrument};
use warp::Filter;

use crate::application::{
    server::ApplicationServer,
    smr_client::{from_slice, to_vec},
    ApplicationResponse,
};

async fn handle_responses<Resp: ApplicationResponse>(
    mut incoming: impl Stream<Item = (ClientId, RequestId, Resp)> + Unpin,
    client_handler: Arc<ResponseHandler<Resp>>,
) {
    while let Some((client_id, request_id, resp)) = incoming.next().await {
        client_handler.response(client_id, request_id, resp).await;
    }
}

pub(super) async fn start<APP: ApplicationServer>(
    requests: mpsc::Sender<(ClientId, RequestId, APP::Request)>,
    responses: mpsc::Receiver<(ClientId, RequestId, APP::Response)>,
    exit: CancellationToken,
    local_socket: SocketAddr,
) -> (JoinHandle<()>, SocketAddr) {
    let (ready_send, ready_recv) = oneshot::channel();
    let join_handle = tokio::spawn(
        run::<APP>(requests, responses, exit, ready_send, local_socket).in_current_span(),
    );
    (join_handle, ready_recv.await.unwrap())
}

async fn run<APP: ApplicationServer>(
    requests: mpsc::Sender<(ClientId, RequestId, APP::Request)>,
    responses: mpsc::Receiver<(ClientId, RequestId, APP::Response)>,
    exit: CancellationToken,
    ready: oneshot::Sender<SocketAddr>,
    local_socket: SocketAddr,
) {
    let client_handler = Arc::new(ResponseHandler::default());

    let (stream, stop_handle_responses) = stream::abortable(ReceiverStream::new(responses));

    let handle_responses_join_handle =
        tokio::spawn(handle_responses(stream, client_handler.clone()).in_current_span());

    let send = {
        let client_handler = client_handler.clone();
        warp::post()
            .and(warp::header::exact("content-type", "application/json"))
            .and(warp::path!(u64 / u64))
            .and(warp::body::bytes())
            .and_then(move |client_id, request_id, body: Bytes| {
                let client_id = ClientId::from_u64(client_id);
                let request_id = RequestId::from_u64(request_id);
                let client_handler = client_handler.clone();
                let requests = requests.clone();
                async move {
                    let request = match from_slice::<APP::Request>(&body) {
                        Ok(request) => request,
                        Err(err) => {
                            tracing::warn!("request json body error: {}", err);
                            return Err(warp::reject::reject());
                        }
                    };

                    let (resp_send, resp_recv) = oneshot::channel();
                    let should_send = client_handler
                        .request(client_id, request_id, resp_send)
                        .await;

                    if should_send {
                        match requests.send((client_id, request_id, request)).await {
                            Ok(()) => {}
                            Err(_) => {
                                tracing::warn!("dropping rquest because channel is closed");
                                return Err(warp::reject::reject());
                            }
                        }
                    }

                    if let Ok(resp) = resp_recv.await {
                        Ok(resp)
                    } else {
                        // closed channels are fine (newer request arrived)
                        Err(warp::reject::reject())
                    }
                }
            })
            .map(|o| {
                let mut res = warp::reply::Response::new(to_vec::<APP::Response>(&o).into());
                res.headers_mut()
                    .insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
                res
            })
    };

    let (addr, server) = warp::serve(send)
        .tls()
        .cert(crate::CERTIFICATE_AUTHORITY_PEM)
        .key(crate::PRIVATE_KEY_PEM)
        .bind_with_graceful_shutdown(local_socket, exit.clone().cancelled_owned());

    let server_join_handle = tokio::spawn(server.in_current_span());

    ready.send(addr).unwrap();

    exit.cancelled().await;
    info!("stopping now");

    stop_handle_responses.abort();
    if let Err(err) = handle_responses_join_handle.await {
        error!("handle responses task paniced: {err}");
    }

    client_handler.clear(); // drop oneshot channels

    if let Err(err) = server_join_handle.await {
        error!("server task paniced: {err}");
    }
}

#[derive(Debug)]
struct ResponseHandler<P> {
    clients: HashMap<ClientId, ClientHandlerState<P>>,
}

impl<P> Default for ResponseHandler<P> {
    fn default() -> Self {
        Self {
            clients: HashMap::new(),
        }
    }
}

impl<P> ResponseHandler<P> {
    async fn request(
        &self,
        client_id: ClientId,
        request_id: RequestId,
        channel: oneshot::Sender<P>,
    ) -> bool {
        self.clients
            .entry_async(client_id)
            .await
            .or_default()
            .request(request_id, channel)
    }

    async fn response(&self, client_id: ClientId, request_id: RequestId, resp: P) {
        self.clients
            .entry_async(client_id)
            .await
            .or_default()
            .response(request_id, resp);
    }

    fn clear(&self) {
        self.clients.clear();
    }
}

#[derive(Debug)]
struct ClientHandlerState<Resp> {
    id: RequestId,
    inner: InnerClientHandlerState<Resp>,
}

impl<Resp> Default for ClientHandlerState<Resp> {
    fn default() -> Self {
        Self {
            id: RequestId::from_u64(0),
            inner: InnerClientHandlerState::Neither,
        }
    }
}

impl<Resp> ClientHandlerState<Resp> {
    fn request(&mut self, id: RequestId, channel: oneshot::Sender<Resp>) -> bool {
        match id.cmp(&self.id) {
            Ordering::Less => false, // ignore
            Ordering::Equal => match self.inner.take() {
                InnerClientHandlerState::Neither => {
                    self.inner = InnerClientHandlerState::Channel(channel);
                    true
                }
                InnerClientHandlerState::Channel(_) => {
                    self.inner = InnerClientHandlerState::Channel(channel);
                    false
                }
                InnerClientHandlerState::Response(payload) => {
                    let _ = channel.send(payload);
                    self.next_id();
                    false
                }
            },

            Ordering::Greater => {
                *self = Self {
                    id,
                    inner: InnerClientHandlerState::Channel(channel),
                };
                true
            }
        }
    }

    fn response(&mut self, id: RequestId, payload: Resp) {
        match id.cmp(&self.id) {
            Ordering::Less => {} // ignore
            Ordering::Equal => match self.inner.take() {
                InnerClientHandlerState::Neither => {
                    self.inner = InnerClientHandlerState::Response(payload)
                }
                InnerClientHandlerState::Channel(channel) => {
                    let _ = channel.send(payload);
                    self.next_id();
                }
                InnerClientHandlerState::Response(_) => {} // ignore
            },
            Ordering::Greater => {
                *self = Self {
                    id,
                    inner: InnerClientHandlerState::Response(payload),
                }
            }
        }
    }

    fn next_id(&mut self) {
        let id = self.id.as_mut();
        *id = id.checked_add(1).unwrap();
    }
}

#[derive(Debug)]
enum InnerClientHandlerState<Resp> {
    Neither,
    Channel(oneshot::Sender<Resp>),
    Response(Resp),
}

impl<Resp> InnerClientHandlerState<Resp> {
    fn take(&mut self) -> Self {
        mem::replace(self, Self::Neither)
    }
}
