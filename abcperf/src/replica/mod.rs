use std::{
    collections::HashMap,
    net::SocketAddr,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};

use crate::{
    atomic_broadcast::AtomicBroadcastChannels, message::StartTimeMsg, MessageDestination,
    MessageType,
};
use bytes::Bytes;
use clap::Args;
use futures::{future, stream, Stream, StreamExt};
use quinn::{
    Connection, ConnectionError, Endpoint, OpenUni, ReadError, ReadToEndError, RecvStream,
    WriteError,
};
use rand::{Rng, SeedableRng};
use shared_ids::{AnyId, ReplicaId};
use tokio_stream::wrappers::ReceiverStream;

use serde::{Deserialize, Serialize};
use tokio::{
    join,
    sync::{mpsc, oneshot},
};

use anyhow::{anyhow, Result};
use ip_family::IpFamilyExt;
use tracing::{error, error_span, info, trace, Instrument};

use crate::{
    config::Config,
    connection::{self, bi_message_stream, Responder},
    message::{
        ClientReplicaMessage, CollectStatsMsg, InitMsg, InitResponseError, PeerPortMsg,
        PeerPortResponse, QuitMsg, SetupPeersMsg, SetupPeersResponseError, StartMsg, StartResponse,
        StopMsg,
    },
    quic_new_client_connect,
    replica::stats::{MessageCounter, StatsSampler},
    AtomicBroadcast, EndRng, SeedRng, Server, VersionInfo,
};

mod stats;

#[derive(Args)]
pub(super) struct ReplicaOpt {
    /// the port to listen on for client connections
    #[clap(long, default_value_t = 0)]
    client_port: u16,

    /// the port to listen on for replica connections
    #[clap(long, default_value_t = 0)]
    replica_port: u16,

    /// address of the client to connect to
    client: SocketAddr,
}

impl ReplicaOpt {
    #[cfg(test)]
    pub(crate) fn new(client: SocketAddr) -> Self {
        Self {
            client_port: 0,
            replica_port: 0,
            client,
        }
    }
}

#[derive(Serialize, Deserialize)]
#[serde(bound = "")]
struct MessageWrapper<
    A: AtomicBroadcast + 'static,
    S: Server<AlgoRequest = A::Transaction, AlgoResponse = A::Decision>,
> {
    message_type: MessageType,
    message: InnerMessageWrapper<A, S>,
}

impl<
        A: AtomicBroadcast + 'static,
        S: Server<AlgoRequest = A::Transaction, AlgoResponse = A::Decision>,
    > MessageWrapper<A, S>
{
    fn new(message_type: impl Into<MessageType>, message: InnerMessageWrapper<A, S>) -> Self {
        Self {
            message_type: message_type.into(),
            message,
        }
    }
}

#[derive(Serialize, Deserialize)]
enum InnerMessageWrapper<
    A: AtomicBroadcast + 'static,
    S: Server<AlgoRequest = A::Transaction, AlgoResponse = A::Decision>,
> {
    Algo(A::ReplicaMessage),
    Server(S::ReplicaMessage),
}

/// main method for when started in replica mode
pub(super) async fn main<
    A: AtomicBroadcast,
    S: Server<AlgoRequest = A::Transaction, AlgoResponse = A::Decision>,
>(
    opt: ReplicaOpt,
    algo_info: VersionInfo,
    algo: impl FnOnce() -> A,
    client_handler: impl FnOnce() -> S,
) -> Result<()> {
    let replica_init_span = error_span!("replica-init").entered();
    let client_con = quic_new_client_connect(opt.client, "localhost").await?;

    let bi_streams = futures_util::stream::unfold(client_con, |c| async move {
        let con = c.accept_bi().await;
        Some((con, c))
    });

    let mut client_con = Box::pin(bi_message_stream::<ClientReplicaMessage, _>(bi_streams));

    let (message, responder) =
        connection::recv_message_of_type::<InitMsg, _>(&mut client_con).await?;
    replica_init_span.exit();
    let replica_span = error_span!("replica", id = message.id.as_u64()).entered();

    if algo_info != message.algo_info {
        responder
            .reply(Err(InitResponseError::AlgoMissmatch))
            .await?;
        return Err(anyhow!(
            "running {:?} and tried to connect to {:?}",
            algo_info,
            message.algo_info
        ));
    }

    responder.reply(Ok(())).await?;
    let faulty = message.faulty;
    info!("got assigned {:?} (faulty: {})", message.id, faulty);

    let seed_rng = SeedRng::from_seed(message.main_seed);

    let config = message.config();

    let replica_socket = SocketAddr::new(opt.client.family().unspecified(), opt.replica_port);

    let client_socket = SocketAddr::new(opt.client.family().unspecified(), opt.client_port);

    let endpoint = crate::quic_server(replica_socket)?;

    let (_, responder) =
        connection::recv_message_of_type::<PeerPortMsg, _>(&mut client_con).await?;
    let incoming_peer_connection_port = endpoint.local_addr()?.port();
    responder
        .reply(PeerPortResponse {
            incoming_peer_connection_port,
        })
        .await?;

    let (msg, responder) =
        connection::recv_message_of_type::<SetupPeersMsg, _>(&mut client_con).await?;

    info!("starting connection to other peers");
    let result = setup_direct_connections(message.id, msg.0, &endpoint).await;
    responder
        .reply(
            result
                .as_ref()
                .map(|_| ())
                .map_err(|_| SetupPeersResponseError::PeerConnectionSetupFailed),
        )
        .await?;
    let conns = result?;

    info!("connected to other peers");
    start::<A, _, _>(
        message.id,
        config,
        conns,
        client_con,
        client_socket,
        (algo(), client_handler()),
        seed_rng,
        faulty,
    )
    .await?;

    replica_span.exit();
    Ok(())
}

/// all replica to replica connection established now start the algorithm
#[allow(clippy::too_many_arguments)]
async fn start<
    A: AtomicBroadcast + 'static,
    C: Unpin + Stream<Item = Result<(ClientReplicaMessage, Responder)>>,
    S: Server<AlgoRequest = A::Transaction, AlgoResponse = A::Decision>,
>(
    my_id: ReplicaId,
    config: Config<A::Config, S::Config>,
    conns: Vec<Connection>,
    mut client: C,
    client_socket: SocketAddr,
    (algo, server): (A, S),
    mut seed_rng: SeedRng,
    faulty: bool,
) -> Result<()> {
    info!("setting up channels");

    let message_counter = MessageCounter::new();

    let omission_enabled = Arc::new(AtomicBool::new(false));

    let SetupReplicaSend {
        replica_recv_recv,
        abort_handles,
        channels,
        in_recv,
    } = SetupReplicaSend::<A, S>::run(
        conns,
        my_id,
        &message_counter,
        &mut seed_rng,
        omission_enabled.clone(),
        config.omission_chance,
    );

    let (out_send, out_recv) = mpsc::channel(1000);
    // TODO add limit

    let channels = Arc::new(channels);

    let _serialize_join = {
        let channels = channels.clone();

        tokio::spawn(
            async move {
                let mut out_recv = out_recv;
                while let Some((msg_dest, msg)) = out_recv.recv().await {
                    channels.send(msg_dest, InnerMessageWrapper::<A, S>::Algo(msg));
                }
            }
            .in_current_span(),
        )
    };

    let (tx_requests, rx_requests) = mpsc::channel(1000); // TODO add limit
    let (tx_responses, rx_responses) = mpsc::channel(1000); // TODO add limit

    let (server_stop_send, server_stop_recv) = oneshot::channel();
    let (server_ready_send, server_ready_recv) = oneshot::channel();

    let _web_server = {
        tokio::spawn(
            server
                .run(
                    config.client(),
                    tx_requests,
                    rx_responses,
                    server_stop_recv,
                    server_ready_send,
                    client_socket,
                    move |msg_dest, msg| {
                        channels.send(msg_dest, InnerMessageWrapper::<A, S>::Server(msg));
                        async {}
                    },
                    replica_recv_recv,
                )
                .in_current_span(),
        )
    };

    let server_stop = server_stop_send;
    let server_addr = server_ready_recv.await.unwrap();
    let server_port = server_addr.port();

    let (_, responder) = connection::recv_message_of_type::<StartMsg, _>(&mut client).await?;

    let stats = StatsSampler::new(config.replicas.sample_delay(), message_counter);

    let (ready_send, ready_recv) = oneshot::channel();

    info!("starting algorithm");
    let _algo = algo.start(
        config.algo(my_id),
        AtomicBroadcastChannels {
            incoming_replica_messages: in_recv,
            outgoing_replica_messages: out_send,
            requests: rx_requests,
            responses: tx_responses,
        },
        move || {
            ready_send
                .send(())
                .expect("receiver of ready channel should not be dropped");
        },
    );

    // wait for algorithm to be ready to accept client connections
    ready_recv
        .await
        .expect("sender of ready channel should not be dropped");

    omission_enabled.store(faulty, Ordering::SeqCst);

    info!("algorithm ready for client connections");
    responder.reply(StartResponse { server_port }).await?;

    let (StartTimeMsg(start_time), responder) =
        connection::recv_message_of_type(&mut client).await?;
    responder.reply(()).await?;

    // wait for stop signal from client
    let (_, responder) = connection::recv_message_of_type::<StopMsg, _>(&mut client).await?;
    responder.reply(()).await?;

    info!("stopping stats collector");
    let stats = stats.stop(start_time.into()).await?;

    info!("stopping web server");
    server_stop
        .send(())
        .map_err(|_| anyhow!("failed to stop"))?;
    //web_server.await?;

    info!("stopping receive handlers");
    for abort_handle in abort_handles {
        abort_handle.abort()
    }

    let (_, responder) =
        connection::recv_message_of_type::<CollectStatsMsg, _>(&mut client).await?;
    info!("sending back stats");
    responder.reply(stats).await?;

    info!("waiting for client to quit");
    let (_, responder) = connection::recv_message_of_type::<QuitMsg, _>(&mut client).await?;
    responder.reply(()).await?;

    info!("quitting");
    Ok(())
}

struct SendChannels {
    all: Vec<mpsc::Sender<Bytes>>,
    by_id: HashMap<ReplicaId, mpsc::Sender<Bytes>>,
    counter: Arc<MessageCounter>,
}

impl SendChannels {
    fn encode_message<
        A: AtomicBroadcast + 'static,
        S: Server<AlgoRequest = A::Transaction, AlgoResponse = A::Decision>,
    >(
        msg: MessageWrapper<A, S>,
    ) -> Bytes {
        let data = bincode::serialize(&msg).expect("should always be serializable");
        let data: Bytes = data.into();
        data
    }

    fn send<
        A: AtomicBroadcast + 'static,
        S: Server<AlgoRequest = A::Transaction, AlgoResponse = A::Decision>,
    >(
        &self,
        dest: MessageDestination,
        message: InnerMessageWrapper<A, S>,
    ) {
        let data = Self::encode_message::<A, S>(MessageWrapper::new(&dest, message));

        self.counter.server().message_type(&dest).tx();

        match dest {
            MessageDestination::Unicast(id) => {
                self.by_id.get(&id).unwrap().try_send(data).unwrap();
            }
            MessageDestination::Broadcast => {
                for channel in &self.all {
                    channel.try_send(data.clone()).unwrap();
                }
            }
        }
    }
}

struct SetupReplicaSend<
    A: AtomicBroadcast + 'static,
    S: Server<AlgoRequest = A::Transaction, AlgoResponse = A::Decision>,
> {
    replica_recv_recv: mpsc::Receiver<(MessageType, ReplicaId, S::ReplicaMessage)>,
    abort_handles: Vec<stream::AbortHandle>,
    channels: SendChannels,
    in_recv: mpsc::Receiver<(MessageType, ReplicaId, A::ReplicaMessage)>,
}

impl<
        A: AtomicBroadcast + 'static,
        S: Server<AlgoRequest = A::Transaction, AlgoResponse = A::Decision>,
    > SetupReplicaSend<A, S>
{
    fn run(
        conns: Vec<Connection>,
        my_id: ReplicaId,
        message_counter: &Arc<MessageCounter>,
        mut seed_rng: &mut SeedRng,
        omission_enabled: Arc<AtomicBool>,
        omission_chance: f64,
    ) -> Self {
        let (replica_recv_send, replica_recv_recv) = mpsc::channel(1000);

        let mut send_join = Vec::new();
        let mut abort_handles = Vec::new();
        let mut recv_join = Vec::new();
        let mut channels = Vec::new();
        let mut channels_by_id = HashMap::new();

        let (in_send, in_recv) = mpsc::channel(1000); // TODO add limit

        for (id, con) in conns.into_iter().enumerate() {
            let id = id as u64;
            let id = if id < my_id.as_u64() { id } else { id + 1 };
            let id = ReplicaId::from_u64(id);

            let (send, recv) = mpsc::channel(1000);
            channels.push(send.clone());
            channels_by_id.insert(id, send);

            send_join.push(tokio::spawn(
                send_handler(
                    recv,
                    id,
                    con.clone(),
                    EndRng::from_rng(&mut seed_rng).unwrap(),
                    omission_enabled.clone(),
                    omission_chance,
                )
                .in_current_span(),
            ));

            let uni_streams = futures_util::stream::unfold(con, |c| async move {
                let con = c.accept_uni().await;
                Some((con, c))
            });
            let (recv, handle) = stream::abortable(uni_streams);
            abort_handles.push(handle);
            recv_join.push({
                let in_send = in_send.clone();
                let message_counter = message_counter.clone();
                let replica_recv_send = replica_recv_send.clone();

                tokio::spawn(
                    recv_handler::<A, S, _>(
                        Box::pin(recv),
                        id,
                        in_send,
                        replica_recv_send,
                        message_counter,
                        EndRng::from_rng(&mut seed_rng).unwrap(),
                        omission_enabled.clone(),
                        omission_chance,
                    )
                    .in_current_span(),
                )
            });
        }
        drop(in_send);
        drop(replica_recv_send);
        Self {
            replica_recv_recv,
            abort_handles,
            channels: SendChannels {
                all: channels,
                by_id: channels_by_id,
                counter: message_counter.clone(),
            },
            in_recv,
        }
    }
}

#[allow(clippy::too_many_arguments)]
async fn recv_handler<
    A: AtomicBroadcast + 'static,
    S: Server<AlgoRequest = A::Transaction, AlgoResponse = A::Decision>,
    I: Stream<Item = Result<RecvStream, ConnectionError>> + Unpin,
>(
    mut recv: I,
    id: ReplicaId,
    to_algo: mpsc::Sender<(MessageType, ReplicaId, A::ReplicaMessage)>,
    to_server: mpsc::Sender<(MessageType, ReplicaId, S::ReplicaMessage)>,
    message_counter: Arc<MessageCounter>,
    mut rng: EndRng,
    omission_enabled: Arc<AtomicBool>,
    omission_chance: f64,
) {
    loop {
        match recv.next().await {
            Some(Ok(mut message)) => {
                let bytes: Vec<u8> = match message.read_to_end(crate::READ_TO_END_LIMIT).await {
                    Ok(bytes) => bytes,
                    Err(ReadToEndError::Read(ReadError::ConnectionLost(
                        ConnectionError::ApplicationClosed(_),
                    ))) => {
                        // other side closed connection, shutdown gracefully
                        break;
                    }
                    Err(e) => {
                        error!("failed to receive message: {:?}", e);
                        break;
                    }
                };

                if omission_enabled.load(Ordering::SeqCst) && rng.gen_bool(omission_chance) {
                    continue;
                }

                let message: MessageWrapper<A, S> = match bincode::deserialize(&bytes) {
                    Ok(message) => message,
                    Err(e) => {
                        error!("failed to decode message: {:?}", e);
                        break;
                    }
                };
                let MessageWrapper {
                    message_type,
                    message,
                } = message;

                match message {
                    InnerMessageWrapper::Algo(message) => {
                        trace!("got message {:?} from replica {:?}", message, id);
                        message_counter.algo().message_type(message_type).rx();
                        if to_algo.send((message_type, id, message)).await.is_err() {
                            break; // channel closed
                        }
                    }
                    InnerMessageWrapper::Server(message) => {
                        message_counter.server().message_type(message_type).rx();
                        to_server.send((message_type, id, message)).await.unwrap()
                    }
                }
            }
            Some(Err(ConnectionError::ApplicationClosed(_))) => {
                // other side closed connection, shutdown gracefully
                break;
            }
            Some(Err(e)) => {
                error!("failed to receive message: {}", e);
                break;
            }
            None => break,
        }
    }
}

async fn setup_direct_connections(
    peer_id: ReplicaId,
    sockets: Box<[SocketAddr]>,
    endpoint: &Endpoint,
) -> Result<Vec<Connection>> {
    let peer_id = peer_id.as_u64() as usize;
    let sockets = &sockets;
    let (incoming, outgoing) = join!(
        async move {
            let mut vec: Vec<(usize, Connection)> = Vec::new();
            while vec.len() < peer_id {
                let con = endpoint
                    .accept()
                    .await
                    .ok_or_else(|| anyhow!("no more connections available"))?
                    .await?;
                let addr = con.remote_address();
                let (peer_id, _) = sockets
                    .iter()
                    .copied()
                    .enumerate()
                    .filter(|(i, _)| *i < peer_id)
                    .find(|(_, a)| *a == addr)
                    .ok_or_else(|| anyhow!("invalid incoming con"))?;
                info!("connection from peer {} on {}", peer_id, addr);
                vec.push((peer_id, con));
            }
            vec.sort_by_key(|(p, _)| *p);
            vec.dedup_by_key(|(p, _)| *p);
            if vec.len() == peer_id {
                Result::<Vec<Connection>>::Ok(vec.into_iter().map(|(_, n)| n).collect())
            } else {
                Err(anyhow!("connection missing"))
            }
        },
        future::join_all(
            sockets
                .iter()
                .enumerate()
                .filter(|(i, _)| *i > peer_id)
                .map(|(peer, addr)| (peer, *addr, endpoint.clone()))
                .map(|(peer, addr, endpoint)| {
                    tokio::spawn(
                        async move {
                            info!("connecting to peer {} on {}", peer, addr);
                            Result::Ok(endpoint.connect(addr, "localhost")?.await?)
                        }
                        .in_current_span(),
                    )
                }),
        )
    );
    let mut incoming = incoming?;
    let outgoing: Result<Vec<Connection>> = outgoing.into_iter().map(|r| r?).collect();
    let mut outgoing = outgoing?;
    incoming.append(&mut outgoing);
    Ok(incoming)
}

async fn send_handler(
    recv: mpsc::Receiver<bytes::Bytes>,
    id: ReplicaId,
    c: Connection,
    mut rng: EndRng,
    omission_enabled: Arc<AtomicBool>,
    omission_chance: f64,
) {
    async fn send(open: OpenUni<'_>, message: Bytes) -> Result<(), WriteError> {
        let mut uni = open.await?;
        uni.write_all(&message).await?;
        Ok(())
    }

    let mut send_stream = ReceiverStream::new(recv)
        .filter(|_| {
            future::ready(
                !(omission_enabled.load(Ordering::SeqCst) && rng.gen_bool(omission_chance)),
            )
        })
        .map(|bytes| {
            trace!("sending message to replica {:?}", id);
            send(c.open_uni(), bytes)
        })
        .buffer_unordered(100);

    while let Some(res) = send_stream.next().await {
        if let Err(e) = res {
            match e {
                WriteError::ConnectionLost(ConnectionError::ApplicationClosed(_)) => (),
                _ => error!("failed to send message: {:?}", e),
            }
            break;
        }
    }
}
