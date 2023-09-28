use std::{collections::HashMap, fmt::Debug, num::NonZeroU64};

use anyhow::Error;
use serde::{Deserialize, Serialize};
use shared_ids::{ClientId, IdIter};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use trait_alias_macro::pub_trait_alias_macro;

use crate::{MessageDestination, MessageType, ReplicaId};

/// Used to integrate an Atomic Broadcast into abcperf
pub trait AtomicBroadcast: 'static {
    /// Custom configuration for the algorithm
    type Config: ConfigurationExtension;

    /// Messages the algorithm sends between replicas
    type ReplicaMessage: ReplicaMessage;

    /// Requests for the algorithm
    type Transaction: Transaction;

    /// Responses by the algorithm
    type Decision: Decision;

    /// Start the atomic broadcast
    fn start(
        self,
        config: AtomicBroadcastConfiguration<Self::Config>,
        channels: AtomicBroadcastChannels<Self::ReplicaMessage, Self::Transaction, Self::Decision>,
        ready_for_clients: impl Send + 'static + FnOnce() + Sync,
    ) -> JoinHandle<Result<(), Error>>;
}

pub_trait_alias_macro!(ConfigurationExtension = for<'a> Deserialize<'a> + Serialize + Send + Clone + Debug + 'static + Sync + Into<HashMap<String, String>>);
pub_trait_alias_macro!(ReplicaMessage = for<'a> Deserialize<'a> + Serialize + Debug + Send + Unpin);
pub_trait_alias_macro!(Transaction = for<'a> Deserialize<'a> + Serialize + Debug + Send + Sync + 'static);
pub_trait_alias_macro!(Decision = for<'a> Deserialize<'a> + Serialize + Debug + Send + Sync + 'static + Clone);

pub struct AtomicBroadcastConfiguration<A: ConfigurationExtension> {
    pub replica_id: ReplicaId,
    pub n: NonZeroU64,
    pub t: u64,
    pub extension: A,
}

impl<A: ConfigurationExtension> AtomicBroadcastConfiguration<A> {
    pub fn replicas(&self) -> impl Iterator<Item = ReplicaId> {
        IdIter::default().take(self.n.get().try_into().unwrap())
    }
}

pub struct AtomicBroadcastChannels<RM: ReplicaMessage, Req: Transaction, Resp: Decision> {
    pub incoming_replica_messages: mpsc::Receiver<(MessageType, ReplicaId, RM)>,
    pub outgoing_replica_messages: mpsc::Sender<(MessageDestination, RM)>,
    pub requests: mpsc::Receiver<(ClientId, Req)>,
    pub responses: mpsc::Sender<Resp>,
}
