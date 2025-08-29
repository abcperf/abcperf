use std::{collections::HashMap, marker::PhantomData, net::SocketAddr, sync::Arc};

use anyhow::{anyhow, Error};
use bytes::{Bytes, BytesMut};
use crossbeam_utils::atomic::AtomicCell;
use futures::SinkExt;
use futures_util::{Future, StreamExt};
use id_set::ReplicaSet;
use itertools::Itertools;
use rand::{thread_rng, Rng};
use rustls::{Certificate, ClientConfig, PrivateKey};
use shared_ids::{map::ReplicaMap, ReplicaId};
use tokio::{
    net::{TcpListener, TcpStream},
    sync::{mpsc, oneshot},
    task::JoinHandle,
};
use tokio_rustls::server::TlsStream as TlsServerStream;
use tokio_rustls::TlsAcceptor;
use tokio_rustls::TlsConnector;
use tokio_util::{
    codec::{Framed, LengthDelimitedCodec},
    task::AbortOnDropHandle,
};
use tracing::{error, info, trace, warn, Instrument};

use crate::{
    application::Application, config::Config, AtomicBroadcast, MessageDestination, MessageType,
};

use super::{stats::MessageCounter, InnerMessageWrapper, MessageWrapper};

pub(crate) struct StopHandle<T> {
    join_handle: JoinHandle<T>,
    stop_signal: oneshot::Sender<()>,
}

impl<T: Send + 'static> StopHandle<T> {
    pub(crate) fn spawn<
        F: FnOnce(oneshot::Receiver<()>) -> Fut,
        Fut: Future<Output = T> + Send + 'static,
    >(
        spawn: F,
    ) -> Self {
        let (stop_send, stop_recv) = oneshot::channel();

        Self::new(tokio::spawn(spawn(stop_recv).in_current_span()), stop_send)
    }
}

impl<T> StopHandle<T> {
    pub(crate) fn new(join_handle: JoinHandle<T>, stop_signal: oneshot::Sender<()>) -> Self {
        Self {
            join_handle,
            stop_signal,
        }
    }

    pub(crate) async fn stop(self) -> T {
        let _ = self.stop_signal.send(());
        self.join_handle.await.unwrap()
    }
}

fn codec() -> LengthDelimitedCodec {
    LengthDelimitedCodec::builder()
        .max_frame_length(512 * 1024 * 1024)
        .new_codec()
}

fn serialize<ABC: AtomicBroadcast, APP: Application<Transaction = ABC::Transaction>>(
    msg: MessageWrapper<ABC, APP>,
) -> Bytes {
    bincode::serialize(&msg)
        .expect("should always be serializable")
        .into()
}

fn deserialize<ABC: AtomicBroadcast, APP: Application<Transaction = ABC::Transaction>>(
    bytes: BytesMut,
) -> Result<MessageWrapper<ABC, APP>, Error> {
    match bincode::deserialize(&bytes) {
        Ok(message) => Ok(message),
        Err(e) => {
            error!("Cannot deserialize message: {:?}", e);
            Err(anyhow!(e))
        }
    }
}

#[allow(clippy::too_many_arguments)]
async fn connection_setup<
    ABC: AtomicBroadcast,
    APP: Application<Transaction = ABC::Transaction>,
>(
    connection_listener: TcpListener,
    acceptor: TlsAcceptor,
    n: u64,
    message_counter: Arc<MessageCounter>,
    to_abc: mpsc::UnboundedSender<(MessageType, ReplicaId, ABC::ReplicaMessage)>,
    to_app: mpsc::UnboundedSender<(MessageType, ReplicaId, APP::ReplicaMessage)>,
    test_done: oneshot::Sender<()>,
    omission_chance: Arc<AtomicCell<f64>>,
    stop: oneshot::Receiver<()>,
) {
    let mut connected = ReplicaSet::with_capacity(n);
    let mut handles = Vec::with_capacity(n as usize);

    while connected.len() < n {
        let stream = match connection_listener.accept().await {
            Ok((stream, _)) => stream,
            Err(e) => {
                error!("Could not establish connection with other replica: {:?}", e);
                return;
            }
        };
        let stream = match acceptor.accept(stream).await {
            Ok(stream) => stream,
            Err(e) => {
                error!("Could not setup tls context: {:?}", e);
                return;
            }
        };
        let mut framed = Framed::new(stream, codec());
        let frame = match framed.next().await {
            Some(result) => match result {
                Ok(bytes) => bytes,
                Err(e) => {
                    error!("Could not read initial frame: {:?}", e);
                    return;
                }
            },
            None => {
                error!("Connection closed before initial frame received");
                return;
            }
        };
        let MessageWrapper::<ABC, APP> { message, .. } = match deserialize(frame) {
            Ok(message) => message,
            Err(e) => {
                error!("Cannot deserialize initial frame: {:?}", e);
                return;
            }
        };
        let InnerMessageWrapper::Establish(from) = message else {
            error!("First message was not of type Establish");
            return;
        };
        info!("Incoming connection from {}", from);
        connected.insert(from);
        handles.push(AbortOnDropHandle::new(tokio::spawn(connection_handler::<
            ABC,
            APP,
        >(
            from,
            framed,
            message_counter.clone(),
            to_abc.clone(),
            to_app.clone(),
            omission_chance.clone(),
        ))));
    }
    test_done.send(()).unwrap();
    info!("All replicas connected");
    let _ = stop.await;
}

async fn connection_handler<
    ABC: AtomicBroadcast,
    APP: Application<Transaction = ABC::Transaction>,
>(
    from: ReplicaId,
    mut framed: Framed<TlsServerStream<TcpStream>, LengthDelimitedCodec>,
    message_counter: Arc<MessageCounter>,
    to_abc: mpsc::UnboundedSender<(MessageType, ReplicaId, ABC::ReplicaMessage)>,
    to_app: mpsc::UnboundedSender<(MessageType, ReplicaId, APP::ReplicaMessage)>,
    omission_chance: Arc<AtomicCell<f64>>,
) {
    while let Some(frame) = framed.next().await {
        if thread_rng().gen_bool(omission_chance.load()) {
            trace!("Omitting message from {}", from);
            continue;
        }
        let MessageWrapper::<ABC, APP> {
            message,
            message_type,
        } = match frame {
            Ok(frame) => match deserialize::<ABC, APP>(frame) {
                Ok(message) => message,
                Err(e) => {
                    warn!("Cannot deserialize message from {}: {:?}", from, e);
                    continue;
                }
            },
            Err(e) => {
                error!("Cannot read frame from {}, closing\n{:?}", from, e);
                return;
            }
        };

        match message {
            InnerMessageWrapper::Abc(message) => {
                trace!("got algo message {:?} from replica {}", message, from);
                message_counter.algo().message_type(message_type).rx();
                let _ = to_abc.send((message_type, from, message));
            }
            InnerMessageWrapper::App(message) => {
                trace!("got server message {:?} from replica {}", message, from);
                message_counter.server().message_type(message_type).rx();
                let _ = to_app.send((message_type, from, message));
            }
            _ => {
                error!("Received setup message after setup from {}, closing", from);
                return;
            }
        }
    }
    info!("Connection to {} closed by peer", from);
}

#[allow(clippy::too_many_arguments)]
pub(super) async fn start_replica_server<
    ABC: AtomicBroadcast,
    APP: Application<Transaction = ABC::Transaction>,
>(
    config: &Config<ABC::Config, APP::Config>,
    socket: SocketAddr,
    message_counter: Arc<MessageCounter>,
    to_abc: mpsc::UnboundedSender<(MessageType, ReplicaId, ABC::ReplicaMessage)>,
    to_app: mpsc::UnboundedSender<(MessageType, ReplicaId, APP::ReplicaMessage)>,
    test_done: oneshot::Sender<()>,
    omission_chance: Arc<AtomicCell<f64>>,
) -> (u16, StopHandle<()>) {
    let n = u64::from(config.n);

    let (stop_signal, stop) = oneshot::channel();

    let cert = Certificate(crate::CERTIFICATE_DER.into());
    let key = PrivateKey(crate::PRIVATE_KEY_DER.into());
    let config = rustls::ServerConfig::builder()
        .with_safe_defaults()
        .with_no_client_auth()
        .with_single_cert(vec![cert], key)
        .expect("TLS config is static and should work");

    let acceptor = TlsAcceptor::from(Arc::new(config));

    let connection_listener = TcpListener::bind(socket)
        .await
        .unwrap_or_else(|e| panic!("Cannot bind to {}:\n{:?}", socket, e));

    let port = connection_listener
        .local_addr()
        .unwrap_or_else(|e| panic!("Cannot read local port number: {:?}", e))
        .port();

    let join_handle = tokio::spawn(connection_setup::<ABC, APP>(
        connection_listener,
        acceptor,
        n,
        message_counter,
        to_abc,
        to_app,
        test_done,
        omission_chance,
        stop,
    ));
    (port, StopHandle::new(join_handle, stop_signal))
}

pub(super) struct SendChannels<
    ABC: AtomicBroadcast,
    APP: Application<Transaction = ABC::Transaction>,
> {
    all: Vec<mpsc::Sender<Bytes>>,
    by_id: ReplicaMap<mpsc::Sender<Bytes>>,
    counter: Arc<MessageCounter>,
    phantom_data: PhantomData<fn() -> (ABC, APP)>,
}

impl<ABC: AtomicBroadcast, APP: Application<Transaction = ABC::Transaction>>
    SendChannels<ABC, APP>
{
    pub(super) async fn start(
        addrs: impl Iterator<Item = (ReplicaId, SocketAddr)>,
        message_counter: Arc<MessageCounter>,
        my_id: ReplicaId,
        omission_chance: Arc<AtomicCell<f64>>,
    ) -> Self {
        let mut channels = Vec::new();
        let mut channels_by_id = HashMap::new();

        let mut root_cert_store = rustls::RootCertStore::empty();
        let authority = Certificate(crate::CERTIFICATE_AUTHORITY_DER.into());
        root_cert_store
            .add(&authority)
            .expect("Certificate is static");
        let tls_client_config = Arc::new(
            rustls::ClientConfig::builder()
                .with_safe_defaults()
                .with_root_certificates(root_cert_store)
                .with_no_client_auth(),
        );

        for (id, dest) in addrs {
            let (send, recv) = mpsc::channel(1000);
            if id != my_id {
                channels.push(send.clone());
            }
            channels_by_id.insert(id, send);

            tokio::spawn(
                send_handler(
                    id,
                    dest,
                    tls_client_config.clone(),
                    recv,
                    omission_chance.clone(),
                )
                .in_current_span(),
            );
        }

        let channels_by_id: Vec<_> = channels_by_id
            .into_iter()
            .sorted_by_key(|(id, _)| *id)
            .map(|(_, chan)| chan)
            .collect();

        let n = channels_by_id.len();
        let by_id: ReplicaMap<_> = channels_by_id.into();

        let channels = Self {
            all: channels,
            by_id,
            counter: message_counter,
            phantom_data: PhantomData,
        };

        for i in 0..n {
            channels
                .send(
                    MessageDestination::Unicast(ReplicaId::from_u64(i as u64)),
                    InnerMessageWrapper::<ABC, APP>::Establish(my_id),
                )
                .await;
        }
        channels
    }

    pub(super) async fn send(
        &self,
        dest: MessageDestination,
        message: InnerMessageWrapper<ABC, APP>,
    ) {
        match message {
            InnerMessageWrapper::Establish(_) => {}
            InnerMessageWrapper::Abc(_) => self.counter.algo().message_type(&dest).tx(),
            InnerMessageWrapper::App(_) => self.counter.server().message_type(&dest).tx(),
        }

        let data = serialize(MessageWrapper::new(&dest, message));

        match dest {
            MessageDestination::Unicast(id) => {
                self.by_id[id].send(data).await.unwrap();
            }
            MessageDestination::Broadcast => {
                for channel in &self.all {
                    channel.send(data.clone()).await.unwrap();
                }
            }
        }
    }
}

async fn send_handler(
    dest_id: ReplicaId,
    dest_addr: SocketAddr,
    tls_client_config: Arc<ClientConfig>,
    mut recv: mpsc::Receiver<bytes::Bytes>,
    omission_chance: Arc<AtomicCell<f64>>,
) {
    let domain = rustls::ServerName::try_from("localhost").expect("Static domain name");
    let connector = TlsConnector::from(tls_client_config);
    let stream = match TcpStream::connect(dest_addr).await {
        Ok(stream) => stream,
        Err(e) => {
            error!("Cannot establish connection to peer {}: {:?}", dest_id, e);
            return;
        }
    };
    let stream = match connector.connect(domain, stream).await {
        Ok(stream) => stream,
        Err(e) => {
            error!("Cannot establish TLS with peer {}: {:?}", dest_id, e);
            return;
        }
    };
    let mut framed = Framed::new(stream, codec());

    while let Some(bytes) = recv.recv().await {
        if thread_rng().gen_bool(omission_chance.load()) {
            trace!("Omitting message to {}", dest_id);
            continue;
        }
        if let Err(e) = framed.send(bytes).await {
            error!("Cannot send to peer {}: {:?}", dest_id, e);
            return;
        }
    }
    info!("Closing connection to peer {}", dest_id);
}
