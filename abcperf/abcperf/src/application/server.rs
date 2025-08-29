use std::future::Future;

use anyhow::Result;
use shared_ids::{ClientId, ReplicaId, RequestId};
use tokio::{
    sync::{mpsc, oneshot},
    task::JoinHandle,
};

use crate::{config::ApplicationServerConfig, MessageDestination, MessageType};

use super::{
    ABCTransaction, ApplicationConfig, ApplicationReplicaMessage, ApplicationRequest,
    ApplicationResponse,
};

pub struct ApplicationServerChannels<S: ApplicationServer> {
    pub smr_requests: mpsc::Receiver<(ClientId, RequestId, S::Request)>,
    pub smr_responses: mpsc::Sender<(ClientId, RequestId, S::Response)>,
    pub incoming_replica_messages:
        mpsc::UnboundedReceiver<(MessageType, ReplicaId, S::ReplicaMessage)>,
    pub ready: oneshot::Sender<()>,
    pub shutdown: oneshot::Receiver<()>,
}

pub struct ApplicationServerStarted<S: ApplicationServer> {
    pub abc_transaction_input_channel: mpsc::Receiver<(ClientId, RequestId, S::Transaction)>,
    pub abc_transaction_output_channel: mpsc::Sender<(ClientId, RequestId, S::Transaction)>,
    pub application_server_join_handle: JoinHandle<Result<()>>,
}

pub trait ApplicationServer: Sized + 'static {
    type Request: ApplicationRequest;
    type Response: ApplicationResponse;
    type Transaction: ABCTransaction;
    type Config: ApplicationConfig;
    type ReplicaMessage: ApplicationReplicaMessage;

    fn start<F: Send + Future<Output = ()> + 'static>(
        config: ApplicationServerConfig<Self::Config>,
        channels: ApplicationServerChannels<Self>,
        send_to_replica: impl 'static + Sync + Send + Fn(MessageDestination, Self::ReplicaMessage) -> F,
    ) -> ApplicationServerStarted<Self>;
}
