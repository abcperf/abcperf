use std::collections::HashMap;
use std::fs;
use std::sync::Arc;
use std::time::{Duration, Instant};
use std::{net::SocketAddr, path::PathBuf};

use crate::application::Application;
use crate::atomic_broadcast::AtomicBroadcastInfo;
use crate::client::ClientWorkerId;
use crate::config::update::{LoadConfigUpdate, ReplicaConfigUpdate};
use crate::config::ValidatedConfig;
use crate::message::{
    CollectClientStatsMsg, CollectReplicaStatsMsg, HelloMsg, InitMsg, InstanceType, Memory,
    QuitMsg, StartClientWorkerMsg, StartReplicaMsg, StartWaitForErrorMsg,
};
use crate::message::{ErrorResponse, StartTimeMsg};
use crate::net_channel::{
    init_receiving_side, init_sending_side, start_net_to_channel, start_watch_channel_to_net,
};
use crate::orchestrator::config_updater::ConfigUpdater;
use crate::stats::{HostInfo, Micros, ReplicaLiveStats, RunInfo, TimeInfo};
use crate::time::SharedTime;
use crate::{InnerInterStageState, MessageDestination, SecondStageMode};
use futures::stream::FuturesUnordered;
use webserver::{StaticInfo, Webserver, WebserverChannels};

use base64::engine::general_purpose::URL_SAFE;
use base64::Engine;
use bytes::Bytes;
use clap::Args;
use futures::{future, join, FutureExt, Stream, StreamExt};
use futures_util::SinkExt;
use itertools::Itertools;
use quinn::{RecvStream, SendStream};
use rand::{rngs::OsRng, Rng, SeedableRng};

use shared_ids::ReplicaId;
use tokio::sync::mpsc::error::TrySendError;
use tokio::sync::mpsc::{self, Sender};
use tokio::sync::watch;
use tokio::{select, signal, sync::oneshot, time};

use tokio_util::codec::{FramedRead, FramedWrite, LengthDelimitedCodec};
use tracing::{debug, error, error_span, info, warn, Instrument};

use anyhow::{anyhow, bail, ensure, Result};
use uuid::Uuid;

mod config_updater;
pub(crate) mod save;
mod webserver;

use crate::{
    config::Config,
    connection::{OrchestratedConnection, OrchestratedConnections},
    message::{PeerPortMsg, SetupPeersMsg, StopMsg},
    AtomicBroadcast, MainSeed, SeedRng, VersionInfo,
};

use self::save::SaveOpt;

/// Models the options with which the Orchestrator can be configured.
#[derive(Args)]
pub(super) struct OrchestratorOpt {
    /// The inner options with which the Orchestrator can be configured.
    #[clap(flatten)]
    inner: OrchestratorInnerOpt,

    /// The config file to use.
    pub(super) config: PathBuf,
}

/// Models the inner options with which the Orchestrator can be configured.
#[derive(Args)]
pub(super) struct OrchestratorInnerOpt {
    /// The option that describes how the collected results should be saved.
    #[clap(flatten)]
    save: SaveOpt,

    /// The rng seed that should be used (defaults to a random seed).
    #[clap(long, value_parser = decode_seed)]
    seed: Option<MainSeed>,

    #[clap(long)]
    web_server: Option<SocketAddr>,

    /// The port that the Orchestrator listens on
    /// for incoming connections from replicas.
    listen: SocketAddr,
}

impl OrchestratorInnerOpt {
    /// Creates a new [OrchestratorInnerOpt].
    ///
    /// # Arguments
    ///
    /// * `save` - The option that describes how the collected results
    ///            should be saved.
    /// * `description` - The description of the orchestrator.
    /// * `seed` - The rng seed that should be used (defaults to a random seed).
    /// * `listen` - The port that the Orchestrator listens on
    ///              for incoming connections from replicas.
    #[cfg(test)]
    pub(crate) fn new(
        save: SaveOpt,
        seed: Option<MainSeed>,
        web_server: Option<SocketAddr>,
        listen: impl Into<SocketAddr>,
    ) -> Self {
        Self {
            save,
            seed,
            web_server,
            listen: listen.into(),
        }
    }
}

/// Decodes the provided seed.
///
/// # Arguments
///
/// * `input` - The seed as a string slice to be decoded.
fn decode_seed(input: &str) -> Result<MainSeed> {
    URL_SAFE
        .decode(input)?
        .try_into()
        .map_err(|v: Vec<u8>| anyhow!("invalid seed length: {}", v.len()))
}

/// The main function for when the Orchestrator mode is started.
///
/// # Arguments
///
/// * `opt` - The options with which the Orchestrator should be configured.
/// * `algo_info` - The information on the running algorithm.
/// * `client_emulator` - The client emulator which should be used.
pub(super) async fn main<ABC: AtomicBroadcast, APP: Application<Transaction = ABC::Transaction>>(
    opt: OrchestratorOpt,
    version_info: VersionInfo,
    config: Config<ABC::Config, APP::Config>,
) -> Result<()> {
    let client_span = error_span!("orchestrator").entered();

    let out = run::<ABC, APP>(opt.inner, config, version_info, None).await;

    client_span.exit();

    out
}

struct ReplicaController {
    conns: OrchestratedConnections<ReplicaId>,
}

/// The [FramedWrite] type for sending network config adjustments to the
/// replicas.
type FramedWriteNetConf = FramedWrite<SendStream, LengthDelimitedCodec>;
/// The [FramedRead] type for receiving [AtomicBroadcastInfo] from the replicas.
type FramedReadABCInfo = FramedRead<RecvStream, LengthDelimitedCodec>;
/// The [FramedRead] type for receiving [ReplicaLiveStats] from the replicas.
type FramedReadRepStats = FramedRead<RecvStream, LengthDelimitedCodec>;

impl ReplicaController {
    fn new(conns: Vec<(u64, OrchestratedConnection)>) -> Self {
        Self {
            conns: OrchestratedConnections::new(conns),
        }
    }

    async fn setup_peers(&self) -> Result<Vec<Memory>> {
        // Retrieve information regarding the addresses and ports at which
        // the replicas listen.
        let ports = self.conns.send_all(PeerPortMsg).await?;
        let addresses: Box<[SocketAddr]> = self
            .conns
            .remote_addresses()
            .zip(ports.iter().map(|r| r.incoming_peer_connection_port))
            .map(SocketAddr::from)
            .collect();

        // Send each replica all the addresses at which the others listen
        // so that they connect to each other.
        let results = self
            .conns
            .send_all(SetupPeersMsg(addresses.clone()))
            .await?;
        for (id, res) in results.into_iter().enumerate() {
            res.map_err(|e| anyhow!("peer {} failed to connect to other peers: {}", id, e))?
        }

        Ok(ports.into_iter().map(|r| r.total_memory).collect())
    }

    /// Create the replica communication streams that should be used to send
    /// and receive messages to and from the replicas.
    async fn create_replica_comm_streams(
        &self,
    ) -> Result<(
        HashMap<ReplicaId, FramedWriteNetConf>,
        HashMap<ReplicaId, FramedReadABCInfo>,
        HashMap<ReplicaId, FramedReadRepStats>,
    )> {
        let mut map_write_live_conf_adj = HashMap::new();
        let mut map_read_live_abc_info = HashMap::new();
        let mut map_read_live_rep_stats = HashMap::new();

        for (replica_id, conn) in self.conns.connections() {
            // Create receiver for live atomic broadcast information.
            let receiver_live_abc_info = conn
                .clone()
                .accept_uni()
                .await
                .map_err(|e| anyhow!("failed to open stream: {}", e))?;
            let mut transport_live_abc_info =
                FramedRead::new(receiver_live_abc_info, LengthDelimitedCodec::new());
            ensure!(transport_live_abc_info.next().await.unwrap()?.as_ref() == b"1");
            map_read_live_abc_info.insert(replica_id, transport_live_abc_info);

            // Create receiver for live replica stats.
            let receiver_live_stats = conn
                .clone()
                .accept_uni()
                .await
                .map_err(|e| anyhow!("failed to open stream: {}", e))?;
            let mut transport_live_stats =
                FramedRead::new(receiver_live_stats, LengthDelimitedCodec::new());
            ensure!(transport_live_stats.next().await.unwrap()?.as_ref() == b"2");
            map_read_live_rep_stats.insert(replica_id, transport_live_stats);

            // Create sender for live network config adjustments.
            let sender_live_conf_adj = conn
                .clone()
                .open_uni()
                .await
                .map_err(|e| anyhow!("failed to open stream: {}", e))?;
            let mut transport_live_conf_adj =
                FramedWrite::new(sender_live_conf_adj, LengthDelimitedCodec::new());
            transport_live_conf_adj.send(Bytes::from("3")).await?;
            map_write_live_conf_adj.insert(replica_id, transport_live_conf_adj);
        }

        Ok((
            map_write_live_conf_adj,
            map_read_live_abc_info,
            map_read_live_rep_stats,
        ))
    }

    async fn start(&self) -> Result<Vec<(ReplicaId, SocketAddr)>> {
        let web_ports = self.conns.send_all(StartReplicaMsg).await?;

        // Receive the endpoints of the applications running on the replicas.
        Ok(self
            .conns
            .ids()
            .zip(
                self.conns
                    .remote_addresses()
                    .zip(web_ports.iter().map(|r| r.server_port))
                    .map(SocketAddr::from),
            )
            .collect())
    }

    async fn sync(&self, start_time: SharedTime) -> Result<()> {
        self.conns.send_all(StartTimeMsg(start_time)).await?;

        Ok(())
    }

    async fn stop(&self) -> Result<()> {
        self.conns.send_all(StopMsg).await?;

        Ok(())
    }

    async fn stats(&self, save_opt: SaveOpt, run_id: Option<Uuid>) -> Result<()> {
        self.conns
            .send_all(CollectReplicaStatsMsg(save_opt, run_id))
            .await?;
        Ok(())
    }

    async fn quit(&self) -> Result<()> {
        self.conns.send_all(QuitMsg).await?;
        self.conns.close();
        Ok(())
    }

    fn start_wait_for_replica_errors(&self) -> impl Stream<Item = (ReplicaId, ErrorResponse)> + '_ {
        self.conns
            .iter()
            .map(|(id, conn)| {
                conn.send(StartWaitForErrorMsg)
                    .map(move |r| r.ok().map(|r| (id, r)))
                    .map(future::ready)
            })
            .collect::<FuturesUnordered<_>>()
            .filter_map(|e| e)
    }
}

struct ClientWorkerController {
    conns: OrchestratedConnections<ClientWorkerId>,
}

impl ClientWorkerController {
    fn new(conns: Vec<(u64, OrchestratedConnection)>) -> Self {
        Self {
            conns: OrchestratedConnections::new(conns),
        }
    }

    async fn start(
        &self,
        replicas: Vec<(ReplicaId, SocketAddr)>,
        start_time: SharedTime,
    ) -> Result<()> {
        self.conns
            .send_all(StartClientWorkerMsg {
                replicas,
                start_time,
            })
            .await?;

        Ok(())
    }

    async fn setup_channels(
        &self,
    ) -> Result<(
        mpsc::Receiver<(u64, Micros)>,
        watch::Sender<LoadConfigUpdate>,
        watch::Sender<Option<ReplicaId>>,
    )> {
        let (send_stats, recv_stats) = mpsc::channel(1000);
        let (send_config_update, recv_config_update) = watch::channel(LoadConfigUpdate {
            ticks_per_second: 10.try_into().expect("is > 0"),
            requests_per_tick: 1.try_into().expect("is > 0"),
        });
        let (send_primary_update, recv_primary_update) = watch::channel(None);

        for (_, conn) in self.conns.connections() {
            let recv_stats = init_receiving_side(conn, "send_stats").await?;
            let send_config_update = init_sending_side(conn, "recv_config_update").await?;
            let send_primary_update = init_sending_side(conn, "recv_primary_update").await?;

            start_net_to_channel(recv_stats, send_stats.clone());
            start_watch_channel_to_net(recv_config_update.clone(), send_config_update);
            start_watch_channel_to_net(recv_primary_update.clone(), send_primary_update);
        }

        Ok((recv_stats, send_config_update, send_primary_update))
    }

    async fn stop(&self) -> Result<()> {
        self.conns.send_all(StopMsg).await?;

        Ok(())
    }

    async fn stats(&self, save_opt: SaveOpt, run_id: Option<Uuid>) -> Result<()> {
        self.conns
            .send_all(CollectClientStatsMsg(save_opt, run_id))
            .await?;
        Ok(())
    }

    fn start_wait_for_replica_errors(
        &self,
    ) -> impl Stream<Item = (ClientWorkerId, ErrorResponse)> + '_ {
        self.conns
            .iter()
            .map(|(id, conn)| {
                conn.send(StartWaitForErrorMsg)
                    .map(move |r| r.ok().map(|r| (id, r)))
                    .map(future::ready)
            })
            .collect::<FuturesUnordered<_>>()
            .filter_map(|e| e)
    }

    async fn quit(&self) -> Result<()> {
        self.conns.send_all(QuitMsg).await?;
        Ok(())
    }
}

/// Sets up the connections with the replicas and runs the algorithm.
///
/// # Arguments
///
/// * `opt` - The inner options with which the Orchestrator can be configured.
/// * `config` - The configuration options with which
///              the algorithm should be configured.
/// * `algo_info` - The information on the running algorithm.
/// * `client_emulator` - The client emulator which should be used.
/// * `addr_info` - The oneshot channel into which the information on the
///                 orchestrator's listening address should be sent.
pub(super) async fn run<ABC: AtomicBroadcast, APP: Application<Transaction = ABC::Transaction>>(
    opt: OrchestratorInnerOpt,
    config: Config<ABC::Config, APP::Config>,
    version_info: VersionInfo,
    addr_info: Option<oneshot::Sender<SocketAddr>>,
) -> Result<()> {
    // Create the seed to send it to the replicas.
    let seed: MainSeed = opt.seed.unwrap_or_else(|| OsRng.gen());

    let seed_string = URL_SAFE.encode(seed);
    info!("using seed: '{}'", seed_string);
    let mut seed_rng = SeedRng::from_seed(seed);

    // Create a QUIC endpoint at which the orchestrator
    // interacts with the replicas.
    let endpoint = crate::quic_new_server(opt.listen)?;
    let local_addr = endpoint.local_addr()?;
    info!("listening on {}", local_addr);
    if let Some(addr_info) = addr_info {
        let _ = addr_info.send(local_addr);
    }

    // Wait for the replicas to connect to the Orchestrator.
    let mut conns =
        wait_for_replicas::<ABC, APP>(endpoint, &config, &version_info, &mut seed_rng).await?;
    let replicas = conns.remove(&InstanceType::Replica).unwrap();
    let client_workers = conns.remove(&InstanceType::Client).unwrap();
    assert!(conns.is_empty());

    let replica_controller = ReplicaController::new(replicas);
    let client_worker_controller = ClientWorkerController::new(client_workers);

    let mut replica_errors = replica_controller.start_wait_for_replica_errors();
    let mut client_worker_errors = client_worker_controller.start_wait_for_replica_errors();

    select! {
        biased;
        Some((id, result)) = replica_errors.next() => {
            let err = anyhow!("{}", result.as_ref()).context(format!(
                "Replica {id} encountered an error",
            ));
            error!("{}", err);
            return Err(err);
        },
        Some((id, result)) = client_worker_errors.next() => {
            let err = anyhow!("{}", result.as_ref()).context(format!(
                "Client worker {id} encountered an error",
            ));
            error!("{}", err);
            return Err(err);
        },
        result = run_if_no_crashes::<ABC, APP>(
            opt,
            config,
            version_info,
            seed_string,
            &replica_controller,
            &client_worker_controller,
        ) => {
            result?;
        },
    }

    Ok(())
}

async fn run_if_no_crashes<
    ABC: AtomicBroadcast,
    APP: Application<Transaction = ABC::Transaction>,
>(
    opt: OrchestratorInnerOpt,
    config: Config<<ABC as AtomicBroadcast>::Config, <APP as Application>::Config>,
    version_info: VersionInfo,
    seed_string: String,
    replica_controller: &ReplicaController,
    client_worker_controller: &ClientWorkerController,
) -> Result<(), anyhow::Error> {
    info!("all peers connected to orchestrator");
    let replicas_total = replica_controller.setup_peers().await?;
    info!("all peers connected to each other");

    // Create QUIC communication streams with replicas.
    let (transports_replica_config_updates, transports_live_abc_info, transports_live_rep_stats) =
        replica_controller.create_replica_comm_streams().await?;

    // Signal the replicas to start the algorithm.
    info!("starting replicas");

    let replicas_endpoints = replica_controller.start().await?;

    let (recv_stats, send_config_update, send_primary_update) =
        client_worker_controller.setup_channels().await?;
    let send_config_update = Arc::new(send_config_update);

    // Create channels for webserver communication.;
    let (sender_adjust_replica_config, recv_adjust_replica_config) = mpsc::channel(1000);
    let (sender_abc_info, recv_abc_info) = mpsc::channel(1000);
    let (sender_replica_usage_stats, recv_replica_usage_stats) = mpsc::channel(1000);

    let config_updater = ConfigUpdater::from_config(
        config.clone(),
        send_config_update.clone(),
        sender_adjust_replica_config.clone(),
    );

    // Start benchmark by starting the client emulator.
    info!(
        "starting experiment:\t{}\tn = {}",
        config.abc.algorithm, config.n
    );
    let start_time = SharedTime::synced_now();

    let config_updater = config_updater.start(start_time.into());

    client_worker_controller
        .start(replicas_endpoints, start_time)
        .await?;

    let webserver_stop = opt.web_server.map(|bind_addr| {
        // Create the webserver to act as an interface between the demonstrator
        // and ABCperf.
        let webserver_channels = WebserverChannels {
            sender_adjust_client_config: send_config_update,
            recv_client: recv_stats,
            recv_abc_info,
            recv_replica_stats: recv_replica_usage_stats,
            sender_adjust_replica_config,
        };
        let static_info = StaticInfo {
            algorithm: config.abc.algorithm.clone(),
            state_machine: config.application.name.clone(),
            num_workers: config.basic.client_workers.get(),
            nodes: replicas_total,
        };
        let webserver = Webserver::new(webserver_channels, bind_addr, static_info, config.n);
        webserver.start()
    });

    // Start threads to send live messages to the replicas.
    let _handle_sender_net_conf = tokio::spawn(
        send_replica_config_updates(
            recv_adjust_replica_config,
            transports_replica_config_updates,
        )
        .in_current_span(),
    );

    let sender_leader = start_update_primary(send_primary_update);

    // Start threads to receive live messages from the replicas.
    let mut _handle_recv_live_abc_info = Vec::new();
    for (from, transport_live_abc_info) in transports_live_abc_info {
        _handle_recv_live_abc_info.push(tokio::spawn(
            recv_live_abc_info(
                sender_abc_info.clone(),
                sender_leader.clone(),
                from,
                transport_live_abc_info,
            )
            .in_current_span(),
        ));
    }
    let mut _handles_recv_live_rep_stats = Vec::new();
    for (from, transport_live_rep_stats) in transports_live_rep_stats {
        _handles_recv_live_rep_stats.push(tokio::spawn(
            recv_live_replica_stats(
                sender_replica_usage_stats.clone(),
                from,
                transport_live_rep_stats,
            )
            .in_current_span(),
        ));
    }

    // Send the synced start time to each replica.
    replica_controller.sync(start_time).await?;

    // Loop for adjusting config as well as receiving logging and stats live.
    // Ends when a request by the user is received or if the experiment
    // duration is reached.

    let duration = wait_for_end_of_experiment(config.experiment_duration(), start_time).await?;

    let end_time = start_time + duration;

    if let Some(webserver_stop) = webserver_stop {
        webserver_stop.stop().await;
    }
    config_updater.stop().await;

    info!("stopping replicas");
    replica_controller.stop().await?;

    info!("stopping clients");
    client_worker_controller.stop().await?;

    // Receive total stats collected from the start until
    // the end of the benchmark.

    let run_id = opt
        .save
        .save_orchestrator_info(
            RunInfo {
                seed: seed_string,
                version_info,
                raw_config: serde_json::to_string(&config).unwrap(),
                basic_config: config.basic,
                abc_algorithm: config.abc.algorithm,
                state_machine_application: config.application.name,
                time_info: TimeInfo {
                    start_time: start_time.unix(),
                    end_time: end_time.unix(),
                    experiment_duration: duration.as_secs_f64(),
                },
            },
            HostInfo::collect(),
        )
        .await?;

    let res = join!(
        client_worker_controller.stats(opt.save.clone(), run_id),
        replica_controller.stats(opt.save, run_id),
    );
    res.0.unwrap();
    res.1.unwrap();

    info!("exit all replicas");
    // Quit the replicas.
    replica_controller.quit().await?;
    client_worker_controller.quit().await?;

    info!("quitting");

    Ok(())
}

async fn wait_for_end_of_experiment(
    experiment_duration: Option<Duration>,
    start_time: SharedTime,
) -> Result<Duration, anyhow::Error> {
    Ok(if let Some(experiment_duration) = experiment_duration {
        select! {
            ctrl_c = signal::ctrl_c() => {
                ctrl_c?;
                info!("stop requested by user");
                start_time.elapsed()
            }
            _ = time::sleep_until((start_time + experiment_duration).into()) => {
                info!("experiment completed after {}s passed", experiment_duration.as_secs());
                experiment_duration
            }
        }
    } else {
        signal::ctrl_c().await?;
        info!("stop requested by user");
        start_time.elapsed()
    })
}

pub(super) fn stage_1(
    opt: Box<OrchestratorOpt>,
    version_info: VersionInfo,
) -> Result<InnerInterStageState> {
    #[cfg(feature = "tokio-console")]
    console_subscriber::init();

    let config_string = fs::read_to_string(&opt.config)?;
    let config: Config = config_string.parse::<ValidatedConfig>()?.into();
    Ok(InnerInterStageState {
        abc_algorithm: config.abc.algorithm,
        state_machine_application: config.application.name,
        config_string,
        mode: SecondStageMode::Orchestrator(opt),
        version_info,
    })
}

/// Waits for all replicas to connect to the Orchestrator.
///
/// # Arguments
///
/// * `endpoint` - The QUIC endpoint of the Orchestrator to which the replicas
///                should connect to.
/// * `config` - The configuration that the replicas should be set with.
/// * `algo_info` - The information on the running algorithm.
/// * `seed` - The main seed which the replicas should use for the algorithm.
async fn wait_for_replicas<
    ABC: AtomicBroadcast,
    APP: Application<Transaction = ABC::Transaction>,
>(
    endpoint: quinn::Endpoint,
    config: &Config<ABC::Config, APP::Config>,
    version_info: &VersionInfo,
    seed: &mut SeedRng,
) -> Result<HashMap<InstanceType, Vec<(u64, OrchestratedConnection)>>> {
    let mut missing = HashMap::new();
    assert!(missing
        .insert(InstanceType::Replica, config.n.get())
        .is_none());
    assert!(missing
        .insert(InstanceType::Client, config.basic.client_workers.get())
        .is_none());

    let mut seeds = HashMap::new();
    for key in missing.keys().sorted().copied() {
        assert!(seeds
            .insert(key, SeedRng::from_rng(&mut *seed).unwrap())
            .is_none());
    }

    let mut next_id = HashMap::new();

    let mut connections = HashMap::<_, Vec<_>>::new();

    let latest_establishment_point = Instant::now() + Duration::from_secs(120);

    while missing.values().copied().max().unwrap_or(0) > 0 {
        let conn = select! {
            biased;
            conn_opt = endpoint.accept() => {
                if let Some(conn) = conn_opt {
                    conn
                } else {
                    bail!("endpoint closed, missing: {missing:?}");
                }
            }
            _ = time::sleep_until(latest_establishment_point.into()) => {
                bail!("timeout while waiting for connections, missing: {missing:?}");
            }
        }
        .await?;

        let addr = conn.remote_address();

        debug!("connection established with {addr}");

        let connection = OrchestratedConnection::new(conn);

        let response = connection
            .send(HelloMsg {
                version_info: version_info.clone(),
                config_string: serde_yaml::to_string(&config).expect("was valid yaml"),
            })
            .await;

        let con_type = match response {
            Ok(Ok(con_type)) => con_type,
            Ok(Err(e)) => {
                error!("connection with {} failed: {}", addr, e);
                continue;
            }
            Err(e) => {
                error!("connection with {} failed: {}", addr, e);
                continue;
            }
        };

        let missing = missing.entry(con_type).or_insert(0);
        if *missing > 0 {
            debug!("new {con_type:?} connected from {addr}");
            *missing -= 1;

            let next_id = next_id.entry(con_type).or_insert(0);
            let id = *next_id;
            *next_id += 1;

            let main_seed = seeds.get_mut(&con_type).unwrap().gen();

            connection.send(InitMsg { id, main_seed }).await?;
            connections
                .entry(con_type)
                .or_default()
                .push((id, connection));
        } else {
            error!("connection with {addr} aborted, we already have enough connections of type {con_type:?}");
        }
    }

    Ok(connections)
}

/// Sends the received live network config adjustments to the replicas.
///
/// # Arguments
///
/// * `recv_live_net_conf` - The channel side to receive network config
///                          adjustments that the specified replica should be
///                          configured with.
/// * `transports_live_conf_adj` - The streams to send the live network config
///                                adjustments to each replica.
async fn send_replica_config_updates(
    mut recv_replica_config_updates: mpsc::Receiver<(MessageDestination, ReplicaConfigUpdate)>,
    mut transports_replica_config_updates: HashMap<ReplicaId, FramedWriteNetConf>,
) -> Result<()> {
    while let Some((dest, update)) = recv_replica_config_updates.recv().await {
        let buf = bincode::serialize::<ReplicaConfigUpdate>(&update)?;
        let frame = Bytes::from(buf);
        match dest {
            MessageDestination::Unicast(id) => {
                transports_replica_config_updates
                    .get_mut(&id)
                    .expect("Request to adjust network config of unknown replica.")
                    .send(frame)
                    .await?
            }
            MessageDestination::Broadcast => {
                for write in transports_replica_config_updates.values_mut() {
                    write.send(frame.clone()).await?
                }
            }
        }
    }
    Ok(())
}

/// Receives the [AtomicBroadcastInfo]s from the replicas and sends them into
/// the dedicated sender channel side to which the webserver listens.
///
/// # Arguments
///
/// * `sender_abc_info` - The channel side to send [AtomicBroadcastInfo] of
///                       the replicas.
/// * `from` - The ID of the replica to which the stream belongs to (from which
///            the [AtomicBroadcastInfo] comes from).
/// * `transport_live_abc_info` - The stream to receive [AtomicBroadcastInfo]
///                               from the provided replica.
///                               The stream has to be connected to the replica.
async fn recv_live_abc_info(
    sender_abc_info: Sender<(ReplicaId, AtomicBroadcastInfo)>,
    sender_leader: Sender<(ReplicaId, Option<ReplicaId>)>,
    from: ReplicaId,
    mut transport_live_abc_info: FramedReadABCInfo,
) -> Result<()> {
    let mut last_log = Instant::now();
    let mut last_leader = None;

    while let Some(msg) = transport_live_abc_info.next().await {
        let abc_info = bincode::deserialize::<AtomicBroadcastInfo>(&msg.unwrap()).expect(
            "Expected live message AtomicBroadcastInfo, but the aforementioned was not received.",
        );
        if last_leader != abc_info.leader {
            last_leader = abc_info.leader;
            let _ = sender_leader.send((from, abc_info.leader)).await;
        }
        match sender_abc_info.try_send((from, abc_info)) {
            Ok(_) => {}
            Err(TrySendError::Closed(_)) => {
                break;
            }
            Err(TrySendError::Full(e)) => {
                let now = Instant::now();
                if now.duration_since(last_log) > Duration::from_secs(1) {
                    warn!("Could not send AtomicBroadcastInfo of replica {from:?} to Webserver: {e:?}");
                    last_log = now;
                }
            }
        }
    }
    Ok(())
}

fn start_update_primary(
    send_primary_update: watch::Sender<Option<ReplicaId>>,
) -> Sender<(ReplicaId, Option<ReplicaId>)> {
    let (sender, mut receiver) = mpsc::channel(1000);
    let mut state = HashMap::new();
    let mut last_replica = None;

    tokio::spawn(async move {
        while let Some((from, primary)) = receiver.recv().await {
            state.insert(from, primary);
            if let Some(current_replica) = state
                .iter()
                .chunk_by(|(_from, primary)| *primary)
                .into_iter()
                .map(|(primary, group)| (primary, group.count()))
                .sorted_by_key(|(_, count)| *count)
                .map(|(replica, _)| *replica)
                .next_back()
            {
                if last_replica != current_replica {
                    last_replica = current_replica;
                    let _ = send_primary_update.send(current_replica);
                }
            }
        }
    });

    sender
}

/// Receives the [ReplicaLiveStats] from the replicas and sends them into
/// the dedicated sender channel side to which the webserver listens.
///
/// # Arguments
///
/// * `sender_replica_usage_stats` - The channel side to send the
///                                  [ReplicaLiveStats] of the replicas.
/// * `from` - The ID of the replica to which the stream belongs to (from which
///            the [ReplicaLiveStats] comes from).
/// * `transport_live_abc_info` - The stream to receive [ReplicaLiveStats]
///                               from the provided replica.
///                               The stream has to be connected to the replica.                         
async fn recv_live_replica_stats(
    sender_replica_usage_stats: Sender<(ReplicaId, ReplicaLiveStats)>,
    from: ReplicaId,
    mut transport_live_rep_stats: FramedReadRepStats,
) -> Result<()> {
    let mut last_log = Instant::now();

    while let Some(msg) = transport_live_rep_stats.next().await {
        let rep_stats = bincode::deserialize::<ReplicaLiveStats>(&msg.unwrap()).expect(
            "Expected live message ReplicaLiveStats, but the aforementioned was not received.",
        );
        match sender_replica_usage_stats.try_send((from, rep_stats)) {
            Ok(_) => {}
            Err(mpsc::error::TrySendError::Full(_)) => {
                let now = Instant::now();
                if now.duration_since(last_log) > Duration::from_secs(1) {
                    warn!(
                        "Could not send usage stats of replica {from:?} to Webserver, channel full"
                    );
                    last_log = now;
                }
            }
            Err(mpsc::error::TrySendError::Closed(_)) => break,
        }
    }
    Ok(())
}
