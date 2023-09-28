use std::{fmt::Debug, net::SocketAddr};

use crate::stats::ReplicaStats;
use crate::{atomic_broadcast::ConfigurationExtension, time::SharedTime};
use anyhow::Result;
use serde::{Deserialize, Serialize};
use shared_ids::ReplicaId;
use thiserror::Error;

use crate::{
    config::{ClientConfigExt, Config},
    MainSeed, VersionInfo,
};

#[derive(Deserialize, Serialize, Debug)]
pub(super) enum ClientReplicaMessage {
    Init(InitMsg),
    PeerPort(PeerPortMsg),
    SetupPeers(SetupPeersMsg),
    Start(StartMsg),
    StartTime(StartTimeMsg),
    Stop(StopMsg),
    CollectStats(CollectStatsMsg),
    Quit(QuitMsg),
}

pub(super) trait CSMsg:
    Into<ClientReplicaMessage> + Clone + TryFrom<ClientReplicaMessage, Error = ClientReplicaMessage>
{
    type Response: for<'a> Deserialize<'a> + Serialize;
}

#[derive(Deserialize, Serialize, Debug, Clone)]
pub(super) struct InitMsg {
    pub(super) id: ReplicaId,
    pub(super) algo_info: VersionInfo,
    pub(super) serialized_config: String,
    pub(super) main_seed: MainSeed,
    pub(super) faulty: bool,
}

impl InitMsg {
    pub(super) fn new<A: ConfigurationExtension, C: ClientConfigExt>(
        id: ReplicaId,
        config: &Config<A, C>,
        algo_info: VersionInfo,
        main_seed: MainSeed,
        faulty: bool,
    ) -> Self {
        let serialized_config =
            serde_json::to_string(config).expect("config should always be serializable");
        Self {
            id,
            serialized_config,
            algo_info,
            main_seed,
            faulty,
        }
    }

    pub(super) fn config<A: ConfigurationExtension, C: ClientConfigExt>(&self) -> Config<A, C> {
        serde_json::from_str(&self.serialized_config)
            .expect("config should always be deserializable")
    }
}

impl From<InitMsg> for ClientReplicaMessage {
    fn from(msg: InitMsg) -> Self {
        Self::Init(msg)
    }
}

impl TryFrom<ClientReplicaMessage> for InitMsg {
    type Error = ClientReplicaMessage;

    fn try_from(msg: ClientReplicaMessage) -> Result<Self, Self::Error> {
        if let ClientReplicaMessage::Init(msg) = msg {
            Ok(msg)
        } else {
            Err(msg)
        }
    }
}

#[derive(Debug, Error, Serialize, Deserialize)]
pub(super) enum InitResponseError {
    #[error("client is running other algorithm than the replica that tried to connect")]
    AlgoMissmatch,
}

impl CSMsg for InitMsg {
    type Response = Result<(), InitResponseError>;
}

#[derive(Deserialize, Serialize, Debug, Clone)]
pub(super) struct PeerPortMsg;

impl From<PeerPortMsg> for ClientReplicaMessage {
    fn from(msg: PeerPortMsg) -> Self {
        Self::PeerPort(msg)
    }
}

impl TryFrom<ClientReplicaMessage> for PeerPortMsg {
    type Error = ClientReplicaMessage;

    fn try_from(msg: ClientReplicaMessage) -> Result<Self, Self::Error> {
        if let ClientReplicaMessage::PeerPort(msg) = msg {
            Ok(msg)
        } else {
            Err(msg)
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub(super) struct PeerPortResponse {
    pub(super) incoming_peer_connection_port: u16,
}

impl CSMsg for PeerPortMsg {
    type Response = PeerPortResponse;
}

#[derive(Deserialize, Serialize, Debug, Clone)]
pub(super) struct SetupPeersMsg(pub(super) Box<[SocketAddr]>);

impl From<SetupPeersMsg> for ClientReplicaMessage {
    fn from(msg: SetupPeersMsg) -> Self {
        Self::SetupPeers(msg)
    }
}

impl TryFrom<ClientReplicaMessage> for SetupPeersMsg {
    type Error = ClientReplicaMessage;

    fn try_from(msg: ClientReplicaMessage) -> Result<Self, Self::Error> {
        if let ClientReplicaMessage::SetupPeers(msg) = msg {
            Ok(msg)
        } else {
            Err(msg)
        }
    }
}

#[derive(Debug, Error, Serialize, Deserialize)]
pub(super) enum SetupPeersResponseError {
    #[error("replica failed to setup peer connections")]
    PeerConnectionSetupFailed,
}

impl CSMsg for SetupPeersMsg {
    type Response = Result<(), SetupPeersResponseError>;
}

#[derive(Deserialize, Serialize, Debug, Clone)]
pub(super) struct StartMsg;

impl From<StartMsg> for ClientReplicaMessage {
    fn from(msg: StartMsg) -> Self {
        Self::Start(msg)
    }
}

impl TryFrom<ClientReplicaMessage> for StartMsg {
    type Error = ClientReplicaMessage;

    fn try_from(msg: ClientReplicaMessage) -> Result<Self, Self::Error> {
        if let ClientReplicaMessage::Start(msg) = msg {
            Ok(msg)
        } else {
            Err(msg)
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub(super) struct StartResponse {
    pub(super) server_port: u16,
}

impl CSMsg for StartMsg {
    type Response = StartResponse;
}

#[derive(Deserialize, Serialize, Debug, Clone)]
pub(super) struct StartTimeMsg(pub SharedTime);

impl From<StartTimeMsg> for ClientReplicaMessage {
    fn from(msg: StartTimeMsg) -> Self {
        Self::StartTime(msg)
    }
}

impl TryFrom<ClientReplicaMessage> for StartTimeMsg {
    type Error = ClientReplicaMessage;

    fn try_from(msg: ClientReplicaMessage) -> Result<Self, Self::Error> {
        if let ClientReplicaMessage::StartTime(msg) = msg {
            Ok(msg)
        } else {
            Err(msg)
        }
    }
}

impl CSMsg for StartTimeMsg {
    type Response = ();
}

#[derive(Deserialize, Serialize, Debug, Clone)]
pub(super) struct StopMsg;

impl From<StopMsg> for ClientReplicaMessage {
    fn from(msg: StopMsg) -> Self {
        Self::Stop(msg)
    }
}

impl TryFrom<ClientReplicaMessage> for StopMsg {
    type Error = ClientReplicaMessage;

    fn try_from(msg: ClientReplicaMessage) -> Result<Self, Self::Error> {
        if let ClientReplicaMessage::Stop(msg) = msg {
            Ok(msg)
        } else {
            Err(msg)
        }
    }
}

impl CSMsg for StopMsg {
    type Response = ();
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub(super) struct CollectStatsMsg;

impl From<CollectStatsMsg> for ClientReplicaMessage {
    fn from(msg: CollectStatsMsg) -> Self {
        Self::CollectStats(msg)
    }
}

impl TryFrom<ClientReplicaMessage> for CollectStatsMsg {
    type Error = ClientReplicaMessage;

    fn try_from(msg: ClientReplicaMessage) -> Result<Self, Self::Error> {
        if let ClientReplicaMessage::CollectStats(msg) = msg {
            Ok(msg)
        } else {
            Err(msg)
        }
    }
}

impl CSMsg for CollectStatsMsg {
    type Response = ReplicaStats;
}

#[derive(Deserialize, Serialize, Debug, Clone)]
pub(super) struct QuitMsg;

impl From<QuitMsg> for ClientReplicaMessage {
    fn from(msg: QuitMsg) -> Self {
        Self::Quit(msg)
    }
}

impl TryFrom<ClientReplicaMessage> for QuitMsg {
    type Error = ClientReplicaMessage;

    fn try_from(msg: ClientReplicaMessage) -> Result<Self, Self::Error> {
        if let ClientReplicaMessage::Quit(msg) = msg {
            Ok(msg)
        } else {
            Err(msg)
        }
    }
}

impl CSMsg for QuitMsg {
    type Response = ();
}
