use std::{fmt::Debug, future::Future, time::Instant};

use serde::{de::DeserializeOwned, Deserialize, Serialize};
use shared_ids::{ClientId, RequestId};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use trait_alias_macro::pub_trait_alias_macro;

use crate::{config::AtomicBroadcastConfiguration, MessageDestination, MessageType, ReplicaId};

// Models a request for the algorithm.
pub_trait_alias_macro!(ABCTransaction = Debug + Serialize + DeserializeOwned + Clone + Send + Sync);
pub_trait_alias_macro!(ABCConfig = Debug + Serialize + DeserializeOwned + Clone);
// Models a message that the algorithm sends between replicas.
pub_trait_alias_macro!(ABCReplicaMessage = Debug + Serialize + DeserializeOwned + Send);

/// Models an Atomic Broadcast that may be integrated into ABCperf.
pub trait AtomicBroadcast: 'static {
    /// The custom configuration for the algorithm.
    type Config: ABCConfig;

    /// The messages the algorithm sends between replicas.
    type ReplicaMessage: ABCReplicaMessage;

    /// The requests for the algorithm.
    type Transaction: ABCTransaction;

    /// Starts the atomic broadcast.
    ///
    /// # Arguments
    ///
    /// * `config` - The configuration of the atomic broadcast.
    /// * `channels` - The channels of the atomic broadcast.
    /// * `send_ready_for_clients` - The function to send a ready signal when
    ///                         the [AtomicBroadcast] is ready for clients.
    fn start<F: Send + Future<Output = ()> + 'static>(
        config: AtomicBroadcastConfiguration<Self::Config>,
        channels: ABCChannels<Self::ReplicaMessage, Self::Transaction>,
        send_ready_for_clients: impl Send + 'static + FnOnce() + Sync,
        send_to_replica: impl 'static + Sync + Send + Fn(MessageDestination, Self::ReplicaMessage) -> F,
    ) -> JoinHandle<Vec<Instant>>;
}

/// Models the message that contains live information received from the
/// atomic broadcast algorithm.
#[derive(Debug, Eq, PartialEq, Clone, Serialize, Deserialize)]
pub struct AtomicBroadcastInfo {
    /// Indicates whether or not the replica running the algorithm is ready for
    /// client requests.
    pub ready: bool,
    /// Contains specific state information of the algorithm that the replica
    /// is running.
    pub state: String,
    /// Contains information on which replica is the leader for the algorithm
    /// running on the replica.
    /// None for non-leader-based algorithms.
    pub leader: Option<ReplicaId>,
}

/// Models the channels used in the [AtomicBroadcast].
pub struct ABCChannels<RM: ABCReplicaMessage, T: ABCTransaction> {
    /// The channel side to receive messages from replicas.
    pub incoming_replica_messages: mpsc::UnboundedReceiver<(MessageType, ReplicaId, RM)>,
    /// The channel side to receive client requests.
    pub transaction_input: mpsc::Receiver<(ClientId, RequestId, T)>,
    /// The channel side to send responses to client requests.
    pub transaction_output: mpsc::Sender<(ClientId, RequestId, T)>,
    pub update_info: mpsc::Sender<AtomicBroadcastInfo>,
    pub do_recover: mpsc::Receiver<()>,
}
