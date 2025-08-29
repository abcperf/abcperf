use std::{fmt::Debug, future::Future, marker::PhantomData, time::Instant};

use abcperf::{
    config::AtomicBroadcastConfiguration, ABCChannels, AtomicBroadcast, MessageDestination,
};
use serde::{Deserialize, Serialize};
use tokio::task::JoinHandle;
use tracing::error;

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ReplyAlgoConfig {}

pub struct ReplyAlgo<P: Serialize + Debug + Sync + Clone + Send + for<'a> Deserialize<'a> + 'static>(
    PhantomData<P>,
);

impl<P: Serialize + Debug + Sync + Clone + Send + for<'a> Deserialize<'a> + 'static> Default
    for ReplyAlgo<P>
{
    fn default() -> Self {
        Self(Default::default())
    }
}

impl<P: Serialize + Debug + Sync + Clone + Send + for<'a> Deserialize<'a> + 'static> AtomicBroadcast
    for ReplyAlgo<P>
{
    type Config = ReplyAlgoConfig;

    type ReplicaMessage = ();

    type Transaction = P;

    fn start<F: Send + Future<Output = ()>>(
        _config: AtomicBroadcastConfiguration<Self::Config>,
        channels: ABCChannels<Self::ReplicaMessage, Self::Transaction>,
        send_ready_for_clients: impl Send + 'static + FnOnce() + Sync,
        _send_to_replica: impl 'static + Sync + Send + Fn(MessageDestination, Self::ReplicaMessage) -> F,
    ) -> JoinHandle<Vec<Instant>> {
        send_ready_for_clients();
        tokio::spawn(Self::run(channels))
    }
}

impl<P: Serialize + Debug + Sync + Clone + Send + for<'a> Deserialize<'a> + 'static> ReplyAlgo<P> {
    async fn run(
        channels: ABCChannels<
            <Self as AtomicBroadcast>::ReplicaMessage,
            <Self as AtomicBroadcast>::Transaction,
        >,
    ) -> Vec<Instant> {
        let ABCChannels {
            incoming_replica_messages: _,
            transaction_input: mut requests,
            transaction_output: responses,
            update_info: _,
            do_recover: _,
        } = channels;
        while let Some(msg) = requests.recv().await {
            if let Err(e) = responses.send(msg).await {
                error!("Error sending response: {:?}", e);
                return vec![];
            };
        }

        vec![]
    }
}
