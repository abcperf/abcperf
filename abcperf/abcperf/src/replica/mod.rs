use std::{
    net::{IpAddr, SocketAddr},
    ops::Deref,
    sync::Arc,
    time::Duration,
};

use crate::{
    application::{
        server::{ApplicationServer, ApplicationServerChannels, ApplicationServerStarted},
        Application,
    },
    atomic_broadcast::AtomicBroadcastInfo,
    config::{
        update::{
            C2RNetworkConfigUpdate, FaultEmulationConfigUpdate, NetworkConfigUpdate,
            R2RNetworkConfigUpdate, ReplicaConfigUpdate,
        },
        Config,
    },
    message::{
        CollectReplicaStatsMsg, InitMsg, Memory, PeerPortMsg, PeerPortResponse, QuitMsg,
        SetupPeersMsg, SetupPeersResponseError, StartReplicaMsg, StartResponse, StartTimeMsg,
        StopMsg,
    },
    replica::{
        communication::{start_replica_server, SendChannels},
        stats::StatsSampler,
    },
    stats::{AlgoStats, ReplicaLiveStats},
    worker::{CommonOrchestratedOpt, OrchestratorCon},
    ABCChannels, MessageType,
};
use bytes::Bytes;
use clap::Args;
use crossbeam_utils::atomic::AtomicCell;
use futures_util::SinkExt;
use netem::{config::Loss, TCNetemTracker};
use quinn::{Connection, RecvStream, SendStream};
use shared_ids::ReplicaId;

use serde::{Deserialize, Serialize};
use sysinfo::{Networks, System};
use tokio::{
    sync::{
        mpsc::{self, Receiver},
        oneshot,
    },
    time::timeout,
};

use anyhow::{anyhow, ensure, Result};
use tokio_stream::StreamExt;
use tokio_util::{
    codec::{FramedRead, FramedWrite, LengthDelimitedCodec},
    sync::CancellationToken,
};
use tracing::{debug, error, error_span, info, Instrument};

use crate::{
    connection::{self},
    replica::stats::MessageCounter,
    AtomicBroadcast,
};

pub(crate) mod communication;
mod stats;
mod webserver;

/// Models the options with which a replica can be configured.
#[derive(Args)]
pub(super) struct ReplicaOpt {
    /// The port to listen on for the connection with the clients.
    #[clap(long, default_value_t = 0)]
    client_port: u16,

    /// The port to listen on for replica connections.
    #[clap(long, default_value_t = 0)]
    replica_port: u16,

    #[clap(flatten)]
    orchestratred: CommonOrchestratedOpt,
}

impl Deref for ReplicaOpt {
    type Target = CommonOrchestratedOpt;

    fn deref(&self) -> &Self::Target {
        &self.orchestratred
    }
}

impl ReplicaOpt {
    /// Creates a new [ReplicaOpt].
    ///
    /// # Arguments
    ///
    /// * `orchestrator_address` - The address of the orchestrator
    ///                            to connect to.
    #[cfg(test)]
    pub(crate) fn new(orchestrator_address: SocketAddr) -> Self {
        Self {
            client_port: 0,
            replica_port: 0,
            orchestratred: CommonOrchestratedOpt::new(orchestrator_address),
        }
    }
}

/// Wraps a message with its type.
#[derive(Serialize, Deserialize)]
#[serde(bound = "")]
struct MessageWrapper<ABC: AtomicBroadcast, APP: Application<Transaction = ABC::Transaction>> {
    message_type: MessageType,
    message: InnerMessageWrapper<ABC, APP>,
}

impl<ABC: AtomicBroadcast, APP: Application<Transaction = ABC::Transaction>>
    MessageWrapper<ABC, APP>
{
    /// Creates a new wrapped message.
    ///
    /// # Arguments
    ///
    /// * `message_type` - The type of the message to wrap.
    /// * `message` - The message itself to wrap.
    fn new(message_type: impl Into<MessageType>, message: InnerMessageWrapper<ABC, APP>) -> Self {
        Self {
            message_type: message_type.into(),
            message,
        }
    }
}

/// Models the inner message that is sent between replicas.
#[derive(Serialize, Deserialize)]
enum InnerMessageWrapper<ABC: AtomicBroadcast, APP: Application<Transaction = ABC::Transaction>> {
    Establish(ReplicaId),
    /// Models an algorithm message.
    Abc(ABC::ReplicaMessage),
    /// Models an application message.
    App(APP::ReplicaMessage),
}

/// The main function for when the replica mode is started.
///
/// # Arguments
///
/// * `opt` - The options with which the replica should be configured.
/// * `algo_info` - The information on the running algorithm.
/// * `algo` - A function which upon calling once
///            should create an instance of the algorithm.
/// * `client_handler` - A function which upon calling once
///                      should create an instance of the client handler.
pub(super) async fn stage_2<
    ABC: AtomicBroadcast,
    APP: Application<Transaction = ABC::Transaction>,
>(
    opt: ReplicaOpt,
    mut orchestrator_con: OrchestratorCon,
    orchestrator_con_uni: Connection,
    config: Config<ABC::Config, APP::Config>,
    message: InitMsg,
) -> Result<()> {
    let my_id = ReplicaId::from_u64(message.id);

    let replica_span = error_span!("replica", id = my_id.as_u64()).entered();

    // Set the replica to faulty if the initial message requested it so.
    info!("got assigned {}", my_id);

    // Create a socket for listening to other replicas.
    let replica_socket = SocketAddr::new(opt.ip_family().unspecified(), opt.replica_port);

    // Create a socket for listening to the orchestrator.
    let orchestrator_socket = SocketAddr::new(opt.ip_family().unspecified(), opt.client_port);

    let message_counter = MessageCounter::new();

    // Receive and handle the message of type PeerPort sent by the orchestrator.
    // Upon receival, reply with the listening port of the endpoint that
    // listens to the other replicas.
    let (_, responder) =
        connection::recv_message_of_type::<PeerPortMsg, _>(&mut orchestrator_con).await?;

    let omission_chance_cell = Arc::new(AtomicCell::new(0.0));

    let (to_abc, in_recv) = mpsc::unbounded_channel();
    let (to_app, replica_recv_recv) = mpsc::unbounded_channel();
    let (test_done, wait_for_connections) = oneshot::channel();
    let (incoming_peer_connection_port, stop_replica_server) = start_replica_server::<ABC, APP>(
        &config,
        replica_socket,
        message_counter.clone(),
        to_abc,
        to_app,
        test_done,
        omission_chance_cell.clone(),
    )
    .await;

    let total_memory = Memory::from_local_system();

    responder
        .reply(PeerPortResponse {
            incoming_peer_connection_port,
            total_memory,
        })
        .await?;

    // Receive and handle the message of type SetupPeers sent by the
    // orchestrator.
    // Upon receival, establish connections to the other replicas.
    let (msg, responder) =
        connection::recv_message_of_type::<SetupPeersMsg, _>(&mut orchestrator_con).await?;

    info!("starting connection to other peers");
    let channels = SendChannels::<ABC, APP>::start(
        config.replicas().zip(msg.0.iter().copied()),
        message_counter.clone(),
        my_id,
        omission_chance_cell.clone(),
    )
    .await;

    let result = timeout(Duration::from_secs(5), wait_for_connections).await;

    responder
        .reply(
            result
                .as_ref()
                .map(|_| ())
                .map_err(|_| SetupPeersResponseError::PeerConnectionSetupFailed),
        )
        .await?;
    result?.unwrap();

    info!("connected to other peers");

    // Create sender for live atomic broadcast information.
    let sender_live_abc_info = orchestrator_con_uni
        .open_uni()
        .await
        .map_err(|e| anyhow!("failed to open stream: {}", e))?;
    let mut transport_live_abc_info =
        FramedWrite::new(sender_live_abc_info, LengthDelimitedCodec::new());
    transport_live_abc_info.send(Bytes::from("1")).await?;

    // Create sender for live replica stats.
    let sender_live_stats = orchestrator_con_uni
        .open_uni()
        .await
        .map_err(|e| anyhow!("failed to open stream: {}", e))?;
    let mut transport_live_stats = FramedWrite::new(sender_live_stats, LengthDelimitedCodec::new());
    transport_live_stats.send(Bytes::from("2")).await?;

    // Create receiver for live config adjustments.
    let receiver_live_conf_adj = orchestrator_con_uni
        .accept_uni()
        .await
        .map_err(|e| anyhow!("failed to open stream: {}", e))?;
    let mut transports_replica_config_updates =
        FramedRead::new(receiver_live_conf_adj, LengthDelimitedCodec::new());
    ensure!(
        transports_replica_config_updates
            .next()
            .await
            .unwrap()?
            .as_ref()
            == b"3"
    );

    info!("setting up channels");

    let channels = Arc::new(channels);

    // Create transmit and receive channels.
    let (tx_requests, rx_requests) = mpsc::channel(1000); // TODO add limit
    let (tx_responses, rx_responses) = mpsc::channel(100000); // TODO add limit

    let webserver_stop = CancellationToken::new();
    let (web_server, server_addr) = webserver::start::<APP::Server>(
        tx_requests,
        rx_responses,
        webserver_stop.clone(),
        orchestrator_socket,
    )
    .await;

    let (ready_send_app, ready_recv_app) = oneshot::channel();
    let (shutdown_app_send, shutdown_app_recv) = oneshot::channel();

    let ApplicationServerStarted {
        abc_transaction_input_channel,
        abc_transaction_output_channel,
        application_server_join_handle,
    } = {
        let channels = channels.clone();
        APP::Server::start(
            config.server(my_id),
            ApplicationServerChannels {
                smr_requests: rx_requests,
                smr_responses: tx_responses,
                incoming_replica_messages: replica_recv_recv,
                ready: ready_send_app,
                shutdown: shutdown_app_recv,
            },
            move |msg_dest, msg| {
                let c_clone = channels.clone();
                async move {
                    c_clone
                        .send(msg_dest, InnerMessageWrapper::<ABC, APP>::App(msg))
                        .await;
                }
            },
        )
    };

    let server_port = server_addr.port();

    // Receive and handle start message.
    let (_, responder) =
        connection::recv_message_of_type::<StartReplicaMsg, _>(&mut orchestrator_con).await?;

    let stats = StatsSampler::new(Duration::from_secs(1), message_counter);

    let (ready_send, ready_recv) = oneshot::channel();
    let (send_update_info, recv_update_info) = mpsc::channel(100);

    let (send_do_recover, recv_do_recover) = mpsc::channel(1000);

    info!("starting algorithm");
    let algo = ABC::start(
        config.algo(my_id),
        ABCChannels {
            incoming_replica_messages: in_recv,
            transaction_input: abc_transaction_input_channel,
            transaction_output: abc_transaction_output_channel,
            update_info: send_update_info,
            do_recover: recv_do_recover,
        },
        move || {
            ready_send
                .send(())
                .expect("receiver of ready channel should not be dropped");
        },
        move |msg_dest, msg| {
            let c_clone = channels.clone();
            async move {
                c_clone
                    .send(msg_dest, InnerMessageWrapper::<ABC, APP>::Abc(msg))
                    .await;
            }
        },
    );

    // Wait for the algorithm to be ready to accept client connections.
    ready_recv
        .await
        .expect("sender of ready channel should not be dropped");

    ready_recv_app
        .await
        .expect("sender of ready channel should not be dropped");

    info!("algorithm ready for client connections");
    responder.reply(StartResponse { server_port }).await?;

    let my_ip = config
        .replicas()
        .zip(msg.0.iter().copied())
        .find(|(e, _)| *e == my_id)
        .expect("we are always in the list")
        .1
        .ip();

    let mut replica_ips: Vec<_> = msg
        .0
        .iter()
        .copied()
        .map(|a| a.ip())
        .filter(|ip| *ip != my_ip)
        .collect();

    replica_ips.sort();
    replica_ips.dedup();

    // Start threads to receive live messages from
    // Orchestrator.
    let _handle_recv_net_conf = tokio::spawn(
        recv_replica_config_updates(
            my_id,
            vec![], // TODO once we have workers
            replica_ips,
            transports_replica_config_updates,
            omission_chance_cell,
            send_do_recover,
        )
        .in_current_span(),
    );
    // Start threads to send live messages to Orchestrator.
    let _handle_send_live_abc_info = tokio::spawn(
        send_live_abc_info(recv_update_info, transport_live_abc_info).in_current_span(),
    );
    let _handle_send_live_stats =
        tokio::spawn(send_live_stats(transport_live_stats).in_current_span());

    // Receive and handle start time message.
    let (StartTimeMsg(start_time), responder) =
        connection::recv_message_of_type(&mut orchestrator_con).await?;
    let start_time = start_time.into();
    responder.reply(()).await?;

    // Wait for the stop signal from the Orchestrator.
    let (_, responder) =
        connection::recv_message_of_type::<StopMsg, _>(&mut orchestrator_con).await?;
    responder.reply(()).await?;

    info!("stopping stats collector");
    let stats = stats.stop(start_time).await?;

    info!("stopping web server");
    webserver_stop.cancel();
    web_server.await?;
    info!("stopping application server");
    let _ = shutdown_app_send.send(());
    application_server_join_handle.await??;
    info!("stopped web server");

    info!("stopping receive handlers");
    stop_replica_server.stop().await;
    info!("stopped receive handlers");

    info!("collecting stats");
    let algo_stats = match algo.await {
        Ok(stats) => stats,
        Err(e) => {
            error!("algorithm failed when joining: {}", e);
            return Err(anyhow!("algorithm failed"));
        }
    };
    let algo_stats = AlgoStats::new(algo_stats, start_time);
    info!("collected stats");

    info!("waiting for stats command");
    let (replica_stat_msg, responder) =
        connection::recv_message_of_type::<CollectReplicaStatsMsg, _>(&mut orchestrator_con)
            .await?;

    if let Some(run_id) = replica_stat_msg.1 {
        info!("saving stats");

        replica_stat_msg
            .0
            .save_replica_samples(run_id, my_id, stats, algo_stats)
            .await?;
        info!("saved stats");
    } else {
        info!("no saving of stats requested");
    }
    responder.reply(()).await?;

    info!("waiting for quit command");
    let (_, responder) =
        connection::recv_message_of_type::<QuitMsg, _>(&mut orchestrator_con).await?;
    responder.reply(()).await?;
    info!("quitting");

    replica_span.exit();
    Ok(())
}

async fn recv_replica_config_updates(
    my_id: ReplicaId,
    clients: Vec<IpAddr>,
    replicas: Vec<IpAddr>,
    mut transports_replica_config_updates: FramedRead<RecvStream, LengthDelimitedCodec>,
    omission_chance_cell: Arc<AtomicCell<f64>>,
    do_recover: mpsc::Sender<()>,
) -> Result<()> {
    let mut netem_tracker = TCNetemTracker::default();
    let mut last_omission_chance = 0.0;
    while let Some(live_msg) = transports_replica_config_updates.next().await {
        let msg = bincode::deserialize::<ReplicaConfigUpdate>(&live_msg.unwrap())
            .expect("Live message from Orchestrator was not of expected format.");

        let (dsts, update) = match msg {
            ReplicaConfigUpdate::R2RNetwork(R2RNetworkConfigUpdate(update)) => (&replicas, update),
            ReplicaConfigUpdate::C2RNetwork(C2RNetworkConfigUpdate(update)) => (&clients, update),
            ReplicaConfigUpdate::FaultEmulation(FaultEmulationConfigUpdate {
                replica_id,
                omission_chance,
            }) => {
                assert_eq!(replica_id, my_id);
                if omission_chance == 0.0 && last_omission_chance != 0.0 {
                    let _ = do_recover.send(()).await;
                }
                last_omission_chance = omission_chance;
                omission_chance_cell.store(omission_chance);
                continue;
            }
        };

        let NetworkConfigUpdate {
            latency,
            jitter,
            packet_loss,
        } = update;

        let netem_config =
            netem::config::Config::new(latency, jitter, Loss::new(packet_loss).unwrap());

        for dst in dsts.iter().copied() {
            match netem_tracker.config_egress(dst, &netem_config) {
                Ok(_) => debug!("Successfully updated the network config of replica {my_id:?} for destination {dst:?}."),
                Err(e) => error!("Could not update the network config of replica {my_id:?} for destination {dst:?}: {e:?}"),
            }
        }
    }

    // TODO: If network is lost or channel is closed too soon, all network config is reset
    netem_tracker
        .clean_all_netem_configs()
        .expect("Failed to cleanup NetEm configuration adjustments on replica {my_id:?}.");
    Ok(())
}

/// Sends the live [AtomicBroadcastInfo] to the Orchestrator.
///
/// # Arguments
///
/// * `recv_abc_info` - The channel side to receive [AtomicBroadcastInfo] from
///                     the running algorithm.
/// * `transport_live_abc_info` - The stream to send the [AtomicBroadcastInfo]
///                               to the Orchestrator.
async fn send_live_abc_info(
    mut recv_abc_info: Receiver<AtomicBroadcastInfo>,
    mut transport_live_abc_info: FramedWrite<SendStream, LengthDelimitedCodec>,
) -> Result<()> {
    while let Some(abc_info) = recv_abc_info.recv().await {
        let buf = bincode::serialize(&abc_info)?;
        let frame = Bytes::from(buf);
        transport_live_abc_info.send(frame).await?;
    }
    Ok(())
}

/// Sends the live [ReplicaLiveStats] to the Orchestrator.
///
/// # Arguments
///
/// * `sys` - Contains the system information of the replica that should be
///           refreshed beforehand.
/// * `transport_live_stats` - The stream to send the [ReplicaLiveStats] to the
///                            Orchestrator.
async fn send_live_stats(
    mut transport_live_stats: FramedWrite<SendStream, LengthDelimitedCodec>,
) -> Result<()> {
    let mut sys = System::new_all();
    sys.refresh_all();
    let mut net = Networks::new();
    net.refresh_list();

    loop {
        let live_stats = ReplicaLiveStats::new(&mut sys, &mut net);
        let buf = bincode::serialize(&live_stats)?;
        let frame = Bytes::from(buf);
        transport_live_stats.send(frame).await?;
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
}
