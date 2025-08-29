use anyhow::{anyhow, bail, ensure, Error, Result};
use futures_util::{Sink, SinkExt, Stream, StreamExt};
use serde::{Deserialize, Serialize};
use shared_ids::ReplicaId;
use tokio::{
    select,
    sync::{broadcast, mpsc, oneshot, watch},
    time::{sleep, Duration, Instant, Sleep},
};
use tokio_stream::wrappers::{errors::BroadcastStreamRecvError, BroadcastStream};
use tracing::{debug, error, info, Instrument};
use warp::{
    filters::ws::{Message, WebSocket},
    Filter,
};

use std::{
    collections::{HashMap, HashSet},
    future,
    net::SocketAddr,
    num::NonZeroU64,
    pin::Pin,
    sync::Arc,
};

use crate::{
    atomic_broadcast::AtomicBroadcastInfo,
    config::update::{
        C2RNetworkConfigUpdate, FaultEmulationConfigUpdate, LoadConfigUpdate, NetworkConfigUpdate,
        R2RNetworkConfigUpdate, ReplicaConfigUpdate,
    },
    generator::LiveConfig,
    message::Memory,
    replica::communication::StopHandle,
    stats::{Micros, ReplicaLiveStats},
    MessageDestination,
};

const AUTHENTICATION_TOKEN: &str = ";d0d7D866%tec4tsFX_/OT^!%}]*>|{|";

#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct StaticInfo {
    pub(crate) algorithm: String,
    pub(crate) state_machine: String,
    pub(crate) num_workers: u64,
    // contains ram size
    pub(crate) nodes: Vec<Memory>,
}

#[derive(Debug, Serialize, Deserialize)]
struct NodeStat {
    node_id: ReplicaId,
    ready: bool,
    state: String,
    leader: bool,
    cpu: u8,
    ram: f64,
    bandwidth: f64,
}

#[derive(Debug, Serialize, Deserialize)]
struct NodeStats {
    node_stats: Vec<NodeStat>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ClientStats {
    throughput: u64,
    latency: u64,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type")]
#[serde(rename_all = "snake_case")]
enum WebSocketMessage {
    AuthRequest {
        token: String,
    },
    AuthResponse {
        success: bool,
        #[serde(flatten)]
        static_info: Arc<StaticInfo>,
    },
    // from frontend
    UpdateConfig(ConfigUpdate),
    // to frontend
    Config(ConfigUpdate),
    // from frontend
    UpdateFaulty(FaultUpdate),
    // to frontend
    Faulty(FaultUpdate),
    NodeStats(Arc<NodeStats>),
    ClientStats(ClientStats),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ConfigUpdate {
    omission_faults: u8,
    #[serde(flatten)]
    client: ConfigUpdateClient,
    #[serde(flatten)]
    network: ConfigUpdateNetwork,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct ConfigUpdateNetwork {
    latency: u16,
    packet_loss: u16,
}

type ConfigUpdateClient = LoadConfigUpdate;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct FaultUpdate {
    node_id: ReplicaId,
    faulty: bool,
}

pub(crate) struct WebserverChannels {
    pub sender_adjust_client_config: Arc<watch::Sender<LiveConfig>>,
    pub sender_adjust_replica_config: mpsc::Sender<(MessageDestination, ReplicaConfigUpdate)>,
    /// The channel side to receive live collected stats.
    pub recv_client: mpsc::Receiver<(u64, Micros)>,
    pub recv_abc_info: mpsc::Receiver<(ReplicaId, AtomicBroadcastInfo)>,
    pub recv_replica_stats: mpsc::Receiver<(ReplicaId, ReplicaLiveStats)>,
}

/// Models the webserver which serves as a communication medium
/// between ABCperf and the demonstrator.
pub struct Webserver {
    channels: WebserverChannels,
    bind_addr: SocketAddr,
    static_info: Arc<StaticInfo>,
    n: NonZeroU64,
}

impl Webserver {
    pub(crate) fn new(
        channels: WebserverChannels,
        bind_addr: SocketAddr,
        static_info: StaticInfo,
        n: NonZeroU64,
    ) -> Self {
        Self {
            channels,
            bind_addr,
            static_info: Arc::new(static_info),
            n,
        }
    }
    pub(crate) fn start(self) -> StopHandle<()> {
        StopHandle::spawn(|stop| self.run(stop))
    }

    async fn run(self, stop: oneshot::Receiver<()>) {
        let (ws_stop_send, ws_stop_recv) = oneshot::channel();
        let (dist_stop_send, dist_stop_recv) = oneshot::channel();
        let (update_send, update_recv) = mpsc::channel(100);

        let (bc_send, bc_recv) = broadcast::channel(100);
        drop(bc_recv);
        let bc_send_2 = bc_send.clone();

        let static_info = self.static_info.clone();
        let routes = warp::ws().map(move |ws: warp::ws::Ws| {
            let cf_s = update_send.clone();
            let bc_s = bc_send.clone();
            let static_info = static_info.clone();
            ws.on_upgrade(move |websocket: WebSocket| process(websocket, cf_s, bc_s, static_info))
        });
        let (_sock, fut) = warp::serve(routes).bind_with_graceful_shutdown(self.bind_addr, async {
            let _ = ws_stop_recv.await;
        });

        let ws = tokio::spawn(fut.in_current_span());
        let dist = tokio::spawn(
            self.distributor(bc_send_2, update_recv, dist_stop_recv)
                .in_current_span(),
        );

        let _ = stop.await;
        let _ = ws_stop_send.send(());
        let _ = ws.await;
        let _ = dist_stop_send.send(());
        let _ = dist.await;
    }

    async fn distributor(
        mut self,
        bc_send: broadcast::Sender<Brodcasts>,
        mut update_recv: mpsc::Receiver<Updates>,
        mut stop: oneshot::Receiver<()>,
    ) {
        let mut sleep = client_sleep();
        let mut last = Instant::now();
        let mut latency_sum = 0;
        let mut count = 0;

        let mut faults = HashSet::new();
        let mut omission_faults = 0;

        let mut last_config_update = ConfigUpdate {
            omission_faults: 0,
            client: ConfigUpdateClient {
                ticks_per_second: 500.try_into().expect("is > 0"),
                requests_per_tick: 11.try_into().expect("is > 0"),
            },
            network: ConfigUpdateNetwork {
                latency: 0,
                packet_loss: 0,
            },
        };
        let mut last_fault_updates = (0..self.n.get())
            .map(|i| {
                let id = ReplicaId::from_u64(i);
                (
                    id,
                    FaultUpdate {
                        node_id: id,
                        faulty: false,
                    },
                )
            })
            .collect::<HashMap<_, _>>();

        let mut last_config_update_client = None;
        let mut last_config_update_network = None;

        let mut last_replica_stats = HashMap::<_, ReplicaLiveStats>::new();
        let mut last_abc_infos = HashMap::<_, AtomicBroadcastInfo>::new();

        let mut i = 0;

        let _ = self
            .channels
            .sender_adjust_client_config
            .send(last_config_update.client.clone());

        loop {
            select! {
                biased;
                _ = &mut stop => {
                    return;
                }
                Some(update) = update_recv.recv() => {
                    match update {
                        Updates::Config(config) => {
                            if omission_faults != config.omission_faults {
                                omission_faults = config.omission_faults;
                                for r_id in faults.iter().copied() {
                                    let _ = self.channels.sender_adjust_replica_config.send((MessageDestination::Unicast(r_id), ReplicaConfigUpdate::FaultEmulation(FaultEmulationConfigUpdate { replica_id: r_id, omission_chance: (f64::from(omission_faults) / 100f64) }))).await;
                                }
                            }

                            if last_config_update_client.as_ref() != Some(&config.client) {
                                last_config_update_client = Some(config.client.clone());
                                let _ = self.channels.sender_adjust_client_config.send(config.client.clone());
                            }
                            if last_config_update_network.as_ref() != Some(&config.network) {
                                last_config_update_network = Some(config.network.clone());
                                debug!("Send network config update to orchestrator {last_config_update_network:?}");
                                let update = NetworkConfigUpdate { latency: config.network.latency, jitter: 0, packet_loss: config.network.packet_loss as f64 };
                                let _ = self.channels.sender_adjust_replica_config.send((MessageDestination::Broadcast, ReplicaConfigUpdate::C2RNetwork(C2RNetworkConfigUpdate(update.clone())))).await;
                                let _ = self.channels.sender_adjust_replica_config.send((MessageDestination::Broadcast, ReplicaConfigUpdate::R2RNetwork(R2RNetworkConfigUpdate(update)))).await;
                            }
                            //
                            last_config_update = config.clone();
                            let _ = bc_send.send(Brodcasts::Config(config));
                        },
                        Updates::Fault(fault_update) => {
                            let r_id = fault_update.node_id;

                            if fault_update.faulty {
                                faults.insert(r_id);
                                let _ = self.channels.sender_adjust_replica_config.send((MessageDestination::Unicast(r_id), ReplicaConfigUpdate::FaultEmulation(FaultEmulationConfigUpdate { replica_id: r_id, omission_chance: (f64::from(omission_faults) / 100f64) }))).await;
                            } else {
                                faults.remove(&r_id);
                                let _ = self.channels.sender_adjust_replica_config.send((MessageDestination::Unicast(r_id), ReplicaConfigUpdate::FaultEmulation(FaultEmulationConfigUpdate { replica_id: r_id, omission_chance: 0f64 }))).await;
                            }

                            last_fault_updates.insert(r_id, fault_update.clone());
                            let _ = bc_send.send(Brodcasts::Faulty(fault_update));
                        },
                        Updates::Full => {
                            let _ = bc_send.send(Brodcasts::Config(last_config_update.clone()));
                            for faulty in last_fault_updates.values() {
                                let _ = bc_send.send(Brodcasts::Faulty(faulty.clone()));
                            }
                        },
                    }
                }
                () = &mut sleep => {
                    sleep = client_sleep();
                    let now = Instant::now();
                    let elapsed = now.saturating_duration_since(last);
                    last = now;

                    let throughput = count as f64 / elapsed.as_secs_f64();
                    let latency = latency_sum as f64 / count as f64;
                    let latency = latency / 1000.0;
                    let _ = bc_send.send(Brodcasts::ClientStats(ClientStats { throughput: throughput.floor() as u64, latency: latency.floor() as u64 }));


                    latency_sum = 0;
                    count = 0;

                    if i == 0 {
                        let mut node_stats = Vec::new();

                        let mut leaders = HashSet::new();
                        for leader in last_abc_infos.values().flat_map(|i| i.leader.as_ref()) {
                            leaders.insert(leader);
                        }

                        for (id, replica) in &last_replica_stats {
                            if let Some(abc) = last_abc_infos.get(id) {
                                node_stats.push(NodeStat {
                                    node_id: *id,
                                    ready: abc.ready,
                                    state: abc.state.clone(),
                                    leader: leaders.contains(&id),
                                    cpu: replica.global_cpu_usage as u8,
                                    bandwidth: ((replica.bandwidth_rx_mb + replica.bandwidth_tx_mb) * 10.0).round()/10.0,
                                    ram: replica.mem_usage_gb,
                                })
                            }
                        }

                        let _ = bc_send.send(Brodcasts::NodeStats(Arc::new(NodeStats{ node_stats })));
                    }
                    i += 1;
                    i %= 2;
                }

                Some((id, abc)) = self.channels.recv_abc_info.recv() => {
                    last_abc_infos.insert(id, abc);

                }
                Some((id, replica)) = self.channels.recv_replica_stats.recv() => {
                    last_replica_stats.insert(id, replica);
                },
                Some((n, sum)) = self.channels.recv_client.recv() => {
                    count += n;
                    latency_sum += u64::from(sum);
                }
            }
        }
    }
}

#[derive(Debug)]
enum Msg {
    Close,
    Msg(WebSocketMessage),
}

async fn process(
    websocket: WebSocket,
    update_send: mpsc::Sender<Updates>,
    bc_send: broadcast::Sender<Brodcasts>,
    static_info: Arc<StaticInfo>,
) {
    let websocket = websocket.filter_map(|m| {
        let m = match m {
            Ok(m) => m,
            Err(e) => {
                error!("Web socket error: {:?}", e);
                return future::ready(None);
            }
        };
        let m = match m.to_str() {
            Ok(m) => m,
            Err(()) => {
                return future::ready(if m.is_close() {
                    Some(Msg::Close)
                } else {
                    error!("Invalid web socket msg: {:?}", m);
                    None
                });
            }
        };
        let m = match serde_json::from_str(m) {
            Ok(m) => m,
            Err(e) => {
                error!("Failed to deserialize web socket msg: {:?}", e);
                return future::ready(None);
            }
        };
        future::ready(Some(Msg::Msg(m)))
    });
    let mut websocket = websocket.with(|m| {
        future::ready(
            serde_json::to_string(&m)
                .map(Message::text)
                .map_err(Error::from),
        )
    });

    match inner_process(&mut websocket, update_send, bc_send, static_info).await {
        Ok(_) => {}
        Err(e) => {
            error!("websocket error: {e}");
        }
    }

    let _ = websocket.close().await;
}

#[derive(Debug, Clone)]
enum Brodcasts {
    Config(ConfigUpdate),
    Faulty(FaultUpdate),
    NodeStats(Arc<NodeStats>),
    ClientStats(ClientStats),
}

impl From<Brodcasts> for WebSocketMessage {
    fn from(value: Brodcasts) -> Self {
        match value {
            Brodcasts::Config(i) => Self::Config(i),
            Brodcasts::Faulty(i) => Self::Faulty(i),
            Brodcasts::NodeStats(i) => Self::NodeStats(i),
            Brodcasts::ClientStats(i) => Self::ClientStats(i),
        }
    }
}

#[derive(Debug)]
enum Updates {
    Config(ConfigUpdate),
    Fault(FaultUpdate),
    Full,
}

impl TryFrom<WebSocketMessage> for Updates {
    type Error = WebSocketMessage;

    fn try_from(value: WebSocketMessage) -> std::prelude::v1::Result<Self, Self::Error> {
        match value {
            WebSocketMessage::UpdateConfig(i) => Ok(Self::Config(i)),
            WebSocketMessage::UpdateFaulty(i) => Ok(Self::Fault(i)),
            _ => Err(value),
        }
    }
}

async fn inner_process(
    websocket: &mut (impl Sink<WebSocketMessage, Error = anyhow::Error> + Stream<Item = Msg> + Unpin),
    update_send: mpsc::Sender<Updates>,
    bc_send: broadcast::Sender<Brodcasts>,
    static_info: Arc<StaticInfo>,
) -> Result<()> {
    if let Some(first_msg) = websocket.next().await {
        if let Msg::Msg(WebSocketMessage::AuthRequest { token }) = first_msg {
            let success = token == AUTHENTICATION_TOKEN;

            websocket
                .send(WebSocketMessage::AuthResponse {
                    success,
                    static_info,
                })
                .await?;

            ensure!(success, "invalid token");
        } else {
            bail!("Got non auth message: {first_msg:?}");
        }
    } else {
        bail!("Got no auth message");
    }

    let bc_recv = bc_send.subscribe();
    drop(bc_send);
    let mut bc_recv = BroadcastStream::new(bc_recv);

    update_send.send(Updates::Full).await?;

    loop {
        select! {
            biased;
            Some(res) = bc_recv.next() => {
                match res {
                    Ok(bc) => {
                        websocket.send(bc.into()).await?;
                    },
                    Err(BroadcastStreamRecvError::Lagged(_)) => {
                        error!("web socket brodcast channel lagged");
                    },
                }
            }
            Some(msg) = websocket.next() => {
                match msg {
                    Msg::Close => {
                        info!("Web socket closed by remote party");
                        return Ok(());
                    }
                    Msg::Msg(msg) => {
                        update_send.send(msg.try_into().map_err(|msg| anyhow!("got unexpected msg: {msg:?}"))?).await?;
                    },
                }
            }
            else => break
        }
    }

    Ok(())
}

fn client_sleep() -> Pin<Box<Sleep>> {
    Box::pin(sleep(Duration::from_millis(1000)))
}
