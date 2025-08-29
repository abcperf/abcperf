use std::{
    future::Future,
    num::NonZeroU64,
    time::{Duration, Instant},
};

use abcperf::{
    application::{
        client::{
            ApplicationClientEmulator, ApplicationClientEmulatorArguments, SMRClient,
            SMRClientError, SMRClientFactory,
        },
        server::{ApplicationServer, ApplicationServerChannels, ApplicationServerStarted},
        Application,
    },
    config::{ApplicationClientEmulatorConfig, ApplicationServerConfig, DistributionConfig},
    generator::start_generator,
    stats::{ClientSample, ClientStats, Micros, Timestamp},
    MessageDestination,
};
use anyhow::Result;
use futures::future;
use payload::Payload;
use serde::{Deserialize, Serialize};
use tokio::{
    sync::mpsc::{self, error::TrySendError},
    task::JoinHandle,
};
use tracing::{debug, info, warn, Instrument};

pub mod payload;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NoopApplicationConfig {
    maximum_client_count: NonZeroU64,
    distribution: DistributionConfig,
    payload_size: u64,
}

pub struct NoopApplication {}

impl Application for NoopApplication {
    type Request = Payload;
    type Response = Payload;
    type Transaction = Payload;
    type Config = NoopApplicationConfig;
    type ClientEmulator<CF: SMRClientFactory<Self::Request, Self::Response>> =
        NoopApplicationClient;
    type Server = NoopApplicationServer;
    type ReplicaMessage = ();
}

pub struct NoopApplicationClient {}

impl<CF: SMRClientFactory<Payload, Payload>> ApplicationClientEmulator<CF>
    for NoopApplicationClient
{
    type Request = Payload;
    type Response = Payload;
    type Config = NoopApplicationConfig;

    fn start(
        client_factory: CF,
        config: ApplicationClientEmulatorConfig<Self::Config>,
        args: ApplicationClientEmulatorArguments,
    ) -> JoinHandle<Result<Vec<ClientStats>>> {
        tokio::spawn(Self::run(client_factory, config, args))
    }
}

impl NoopApplicationClient {
    async fn run<CF: SMRClientFactory<Payload, Payload>>(
        mut client_factory: CF,
        config: ApplicationClientEmulatorConfig<NoopApplicationConfig>,
        args: ApplicationClientEmulatorArguments,
    ) -> Result<Vec<ClientStats>> {
        let ApplicationClientEmulatorArguments {
            start_time,
            stop,
            mut rng,
            live_config,
            sender_stats_live,
        } = args;

        let (generator_join_handel, new_request, mut new_consumer) = start_generator(
            stop,
            config.application.maximum_client_count,
            config.application.distribution,
            &mut rng,
            live_config,
            config.client_workers,
            config.worker_id,
        );

        let mut client_join_handles = Vec::new();

        while let Some(()) = new_consumer.recv().await {
            let client = client_factory.create_client();

            client_join_handles.push(tokio::spawn(
                Self::client_loop(
                    client,
                    new_request.clone(),
                    sender_stats_live.clone(),
                    config.application.payload_size,
                )
                .in_current_span(),
            ));
        }

        info!("Stopped creating new clients");

        let clients = future::join_all(client_join_handles.into_iter()).await;

        debug!("Collected all clients' results");

        generator_join_handel.await?;

        clients
            .into_iter()
            .map(|e| {
                Ok(ClientStats {
                    samples: e?
                        .into_iter()
                        .filter_map(|(i, d)| {
                            Timestamp::new(start_time, i).map(|timestamp| ClientSample {
                                timestamp,
                                processing_time: d.into(),
                            })
                        })
                        .collect(),
                })
            })
            .collect::<Result<Vec<_>>>()
    }

    #[allow(clippy::too_many_arguments)]
    async fn client_loop<C: SMRClient<Payload, Payload>>(
        mut client: C,
        receiver: async_channel::Receiver<()>,
        sender_stats_live: mpsc::Sender<Micros>,
        payload_size: u64,
    ) -> Vec<(Instant, Duration)> {
        let mut last_log = Instant::now();
        let mut elapsed_vec = Vec::new();

        while let Ok(()) = receiver.recv().await {
            let payload = Payload::new(payload_size as usize);
            let start = Instant::now();

            let _response = match client.send_request(payload).await {
                Ok(resp) => resp,
                Err(SMRClientError::Cancelled) => {
                    break;
                }
                Err(e) => {
                    warn!("client aborted because of error: {e}");
                    break;
                }
            };

            let elapsed = start.elapsed();
            match sender_stats_live.try_send(elapsed.into()) {
                Ok(_) | Err(TrySendError::Closed(_)) => {}
                Err(TrySendError::Full(_)) => {
                    let now = Instant::now();
                    if now.duration_since(last_log) > Duration::from_secs(10) {
                        warn!("too many requests, live stats inaccurate");
                        last_log = now;
                    }
                }
            }
            elapsed_vec.push((start, elapsed));
        }
        elapsed_vec
    }
}

pub struct NoopApplicationServer {}

impl ApplicationServer for NoopApplicationServer {
    type Request = Payload;
    type Response = Payload;
    type Transaction = Payload;
    type ReplicaMessage = ();
    type Config = NoopApplicationConfig;

    fn start<F: Send + Future<Output = ()>>(
        _config: ApplicationServerConfig<Self::Config>,
        channels: ApplicationServerChannels<Self>,
        _send_to_replica: impl 'static + Sync + Send + Fn(MessageDestination, Self::ReplicaMessage) -> F,
    ) -> ApplicationServerStarted<Self> {
        let ApplicationServerChannels {
            smr_requests,
            smr_responses,
            ready,
            ..
        } = channels;
        let _ = ready.send(());
        ApplicationServerStarted {
            abc_transaction_input_channel: smr_requests,
            abc_transaction_output_channel: smr_responses,
            application_server_join_handle: tokio::spawn(async { Ok(()) }),
        }
    }
}
