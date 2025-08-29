use std::{fmt::Debug, net::SocketAddr};

use crate::orchestrator::save::SaveOpt;
use crate::time::SharedTime;
use anyhow::Result;
use serde::{Deserialize, Serialize};
use shared_ids::ReplicaId;
use sysinfo::System;
use thiserror::Error;
use uuid::Uuid;

use crate::{MainSeed, VersionInfo};

/// Models the messages of the communication
/// between the Orchestrator and the replicas.
#[derive(Deserialize, Serialize, Debug)]
pub(super) enum OrchRepMessage {
    Hello(HelloMsg),
    Init(InitMsg),
    /// Signals the replica to send its port.
    PeerPort(PeerPortMsg),
    /// Sets up the receiving replica by sending them the addresses
    /// of each replica to which they should connect to.
    SetupPeers(SetupPeersMsg),
    /// Signals the receiving replica to start the algorithm.
    StartReplica(StartReplicaMsg),
    /// Signals the receiving replica to start the algorithm.
    StartClientWorker(StartClientWorkerMsg),
    /// Contains the synced start time of the benchmark which
    /// the replica should use to stop collecting stats in the future.
    StartTime(StartTimeMsg),
    /// Signals the replica to gracefully stop (stop the stats collector,
    /// stop the web server, stop receive handlers).
    Stop(StopMsg),
    /// Signals the replica to send its collected stats.
    CollectReplicaStats(CollectReplicaStatsMsg),
    CollectClientStats(CollectClientStatsMsg),
    // / Signals the replica to quit.
    Quit(QuitMsg),
    StartWaitForError(StartWaitForErrorMsg),
}

/// The trait that models a client-server message.
pub(super) trait CSMsg:
    Into<OrchRepMessage> + Clone + TryFrom<OrchRepMessage, Error = OrchRepMessage>
{
    type Response: for<'a> Deserialize<'a> + Serialize;
}

macro_rules! impl_msg {
    ($req:ty, $enum:ident) => {
        impl_msg!($req, $enum, ());
    };
    ($req:ty, $enum:ident, $resp:ty) => {
        impl From<$req> for OrchRepMessage {
            fn from(msg: $req) -> Self {
                Self::$enum(msg)
            }
        }

        impl TryFrom<OrchRepMessage> for $req {
            type Error = OrchRepMessage;

            fn try_from(msg: OrchRepMessage) -> Result<Self, Self::Error> {
                if let OrchRepMessage::$enum(msg) = msg {
                    Ok(msg)
                } else {
                    Err(msg)
                }
            }
        }

        impl CSMsg for $req {
            type Response = $resp;
        }
    };
}

#[derive(Deserialize, Serialize, Debug, Clone)]
pub(super) struct HelloMsg {
    /// The information on the running algorithm.
    pub(super) version_info: VersionInfo,
    /// The serialized configuration for the replica.
    pub(super) config_string: String,
}

/// Models the response that the replica should send to the Orchestrator
/// upon receiving [OrchRepMessage::Init] if an error occurs when initiating.
#[derive(Debug, Error, Serialize, Deserialize)]
pub(super) enum HelloResponseError {
    /// Signals that the client is running another algorithm
    /// other than the one set for the replica.
    #[error("client is running other algorithm than the replica that tried to connect")]
    AlgoMissmatch,
}

#[derive(Debug, Serialize, Deserialize, Hash, Eq, PartialEq, Copy, Clone, Ord, PartialOrd)]
pub enum InstanceType {
    Replica,
    Client,
}

impl_msg!(HelloMsg, Hello, Result<InstanceType, HelloResponseError>);

#[derive(Deserialize, Serialize, Debug, Clone)]
pub(super) struct InitMsg {
    pub(super) id: u64,
    pub(super) main_seed: MainSeed,
}

impl_msg!(InitMsg, Init);

/// Models the message for [OrchRepMessage::PeerPort] that the Orchestrator
/// should send to the replica signaling it to send its incoming
/// connection port.
#[derive(Deserialize, Serialize, Debug, Clone)]
pub(super) struct PeerPortMsg;

#[derive(Debug, Serialize, Deserialize)]
#[repr(transparent)]
#[serde(transparent)]
pub(super) struct Memory(f64);

impl Memory {
    pub(super) fn from_local_system() -> Self {
        let mut sys = System::new_all();
        sys.refresh_all();
        let total_memory_gb = sys.total_memory() as f64 / 1_000_000_000.0;

        Self(total_memory_gb)
    }
}

/// Models the response that the replica should send to the Orchestrator
///  upon receiving [OrchRepMessage::PeerPort].
#[derive(Debug, Serialize, Deserialize)]
pub(super) struct PeerPortResponse {
    /// The incoming connection port of the replica.
    pub(super) incoming_peer_connection_port: u16,

    pub(super) total_memory: Memory,
}

impl_msg!(PeerPortMsg, PeerPort, PeerPortResponse);

/// Models the message for [OrchRepMessage::SetupPeers] that the Orchestrator
/// should send to the replica by containing the addresses of each replica
/// to which it should connect to.
#[derive(Deserialize, Serialize, Debug, Clone)]
pub(super) struct SetupPeersMsg(pub(super) Box<[SocketAddr]>);

/// Models the response that the replica should send to the Orchestrator
/// upon receiving [OrchRepMessage::PeerPort] if an error occurs when
/// setting up the peer connections.
#[derive(Debug, Error, Serialize, Deserialize)]
pub(super) enum SetupPeersResponseError {
    #[error("replica failed to setup peer connections")]
    PeerConnectionSetupFailed,
}

impl_msg!(SetupPeersMsg, SetupPeers, Result<(), SetupPeersResponseError>);

/// Models the message for [OrchRepMessage::Start] that the Orchestrator
/// should send to the replica to signal it to start the algorithm.
#[derive(Deserialize, Serialize, Debug, Clone)]
pub(super) struct StartReplicaMsg;

/// Models the response that the replica should send to the Orchestrator
/// upon receiving [OrchRepMessage::Start].
#[derive(Debug, Serialize, Deserialize)]
pub(super) struct StartResponse {
    /// The endpoint port of the replica.
    pub(super) server_port: u16,
}

impl_msg!(StartReplicaMsg, StartReplica, StartResponse);

#[derive(Deserialize, Serialize, Debug, Clone)]
pub(super) struct StartClientWorkerMsg {
    pub(super) replicas: Vec<(ReplicaId, SocketAddr)>,
    pub(super) start_time: SharedTime,
}
impl_msg!(StartClientWorkerMsg, StartClientWorker);

/// Models the message for [OrchRepMessage::StartTime] that the Orchestrator
/// should send to the replica containing the shared time.
#[derive(Deserialize, Serialize, Debug, Clone)]
pub(super) struct StartTimeMsg(pub SharedTime);

impl_msg!(StartTimeMsg, StartTime);

/// Models the message for [OrchRepMessage::Stop] that the Orchestrator
/// should send to the replica to signal it to stop.
#[derive(Deserialize, Serialize, Debug, Clone)]
pub(super) struct StopMsg;

impl_msg!(StopMsg, Stop);

/// Models the message for [OrchRepMessage::CollectStats] that the Orchestrator
/// should send to the replica to signal it to send all its collected stats
/// from the start to the end of the run.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub(super) struct CollectReplicaStatsMsg(pub(crate) SaveOpt, pub(crate) Option<Uuid>);

impl_msg!(CollectReplicaStatsMsg, CollectReplicaStats);

#[derive(Clone, Debug, Deserialize, Serialize)]
pub(crate) struct CollectClientStatsMsg(pub(crate) SaveOpt, pub(crate) Option<Uuid>);

impl_msg!(CollectClientStatsMsg, CollectClientStats);

/// Models the message for [OrchRepMessage::Quit] that the Orchestrator
/// should send to the replica to signal it to quit.
#[derive(Deserialize, Serialize, Debug, Clone)]
pub(super) struct QuitMsg;

impl_msg!(QuitMsg, Quit);

#[derive(Deserialize, Serialize, Debug, Clone)]
pub(super) struct StartWaitForErrorMsg;

#[derive(Debug, Serialize, Deserialize)]
pub(super) struct ErrorResponse {
    error: String,
}

impl AsRef<str> for ErrorResponse {
    fn as_ref(&self) -> &str {
        &self.error
    }
}

impl From<anyhow::Error> for ErrorResponse {
    fn from(err: anyhow::Error) -> Self {
        Self {
            error: format!("{err}"),
        }
    }
}

impl_msg!(StartWaitForErrorMsg, StartWaitForError, ErrorResponse);
