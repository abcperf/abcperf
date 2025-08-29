use std::{future::Future, time::Instant};

use anyhow::Result;
use shared_ids::ClientId;
use thiserror::Error;
use tokio::{sync::mpsc, task::JoinHandle};
use tokio_util::sync::CancellationToken;

use crate::{
    config::ApplicationClientEmulatorConfig,
    generator::LiveConfig,
    stats::{ClientStats, Micros},
    SeedRng,
};

use super::{ApplicationConfig, ApplicationRequest, ApplicationResponse};

#[derive(Debug, Error)]
pub enum SMRClientError {
    #[error("client cancelled")]
    Cancelled,
    #[error("got at least one invalid response")]
    GotAtLeastOneInvalidResponse,
    #[error("not enough valid response")]
    NotEnoughValidResponse,
}

pub trait SMRClient<Req: ApplicationRequest, Resp: ApplicationResponse>: Send + 'static {
    fn send_request(
        &mut self,
        request: Req,
    ) -> impl Future<Output = Result<Resp, SMRClientError>> + Send;

    fn client_id(&self) -> ClientId;
}

pub trait SMRClientFactory<Req: ApplicationRequest, Resp: ApplicationResponse>:
    Send + 'static
{
    type Client: SMRClient<Req, Resp>;

    fn create_client(&mut self) -> Self::Client;
}

pub struct ApplicationClientEmulatorArguments {
    pub start_time: Instant,
    pub stop: CancellationToken,
    pub rng: SeedRng,
    pub live_config: mpsc::Receiver<LiveConfig>,
    pub sender_stats_live: mpsc::Sender<Micros>,
}

pub trait ApplicationClientEmulator<CF: SMRClientFactory<Self::Request, Self::Response>> {
    type Request: ApplicationRequest;
    type Response: ApplicationResponse;
    type Config: ApplicationConfig;

    fn start(
        client_factory: CF,
        config: ApplicationClientEmulatorConfig<Self::Config>,
        args: ApplicationClientEmulatorArguments,
    ) -> JoinHandle<Result<Vec<ClientStats>>>;
}
