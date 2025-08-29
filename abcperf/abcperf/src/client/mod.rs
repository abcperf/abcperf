use std::{ops::Deref, time::Duration};

use anyhow::Result;
use bytes::Bytes;
use clap::Args;
use futures::SinkExt;
use quinn::Connection;
use rand::SeedableRng;
use shared_ids::{id_type, ReplicaId};
use tokio::{select, sync::mpsc, task::JoinHandle, time};
use tokio_util::sync::CancellationToken;
use tracing::{error, error_span, info};

use crate::{
    application::{
        client::{ApplicationClientEmulator, ApplicationClientEmulatorArguments},
        smr_client::{
            bft::BftClientFactory, fallback::FallbackClientFactory, CurrentPrimary, SharedFactory,
            SmrClient,
        },
        Application,
    },
    config::{ApplicationClientEmulatorConfig, Config},
    connection,
    message::{CollectClientStatsMsg, InitMsg, QuitMsg, StartClientWorkerMsg, StopMsg},
    net_channel::{init_receiving_side, init_sending_side, start_net_to_channel, FramedWriteQuinn},
    stats::{ClientStats, Micros},
    worker::{CommonOrchestratedOpt, OrchestratorCon},
    AtomicBroadcast, EndRng, SeedRng,
};

id_type!(pub ClientWorkerId);

#[derive(Args)]
pub(crate) struct ClientOpt {
    #[clap(flatten)]
    orchestratred: CommonOrchestratedOpt,
}

impl ClientOpt {
    #[cfg(test)]
    pub(crate) fn new(orchestrator_address: std::net::SocketAddr) -> Self {
        Self {
            orchestratred: CommonOrchestratedOpt::new(orchestrator_address),
        }
    }
}

impl Deref for ClientOpt {
    type Target = CommonOrchestratedOpt;

    fn deref(&self) -> &Self::Target {
        &self.orchestratred
    }
}

pub(super) async fn stage_2<
    ABC: AtomicBroadcast,
    APP: Application<Transaction = ABC::Transaction>,
>(
    _opt: ClientOpt,
    mut orchestrator_con: OrchestratorCon,
    connection: Connection,
    config: Config<ABC::Config, APP::Config>,
    message: InitMsg,
) -> Result<()> {
    let my_id = ClientWorkerId::from_u64(message.id);
    let mut seed_rng = SeedRng::from_seed(message.main_seed);

    let span = error_span!("client_worker", id = my_id.as_u64()).entered();

    info!("Ready");

    let send_stats = init_sending_side(&connection, "send_stats").await?;
    let recv_config_update = init_receiving_side(&connection, "recv_config_update").await?;
    let recv_primary_update_net = init_receiving_side(&connection, "recv_primary_update").await?;

    let (message, responder) =
        connection::recv_message_of_type::<StartClientWorkerMsg, _>(&mut orchestrator_con).await?;

    let replicas_endpoints = message.replicas;
    let start_time = *message.start_time.as_ref();

    let (send_adjust_config_reqs, recv_adjust_config_reqs) = mpsc::channel(1000);

    start_net_to_channel(recv_config_update, send_adjust_config_reqs);

    let (send_primary_update, recv_primary_update) = mpsc::channel(1000);

    start_net_to_channel(recv_primary_update_net, send_primary_update);

    let client_stop = CancellationToken::new();

    let (sender_stats_live, recv_stats_live) = mpsc::channel(1000000);

    let application_client_emulator_config = config.application_client_emulator(my_id);

    let args = ApplicationClientEmulatorArguments {
        start_time,
        stop: client_stop.clone(),
        rng: SeedRng::from_rng(&mut seed_rng).unwrap(),
        live_config: recv_adjust_config_reqs,
        sender_stats_live,
    };

    let shared = SharedFactory::new(
        replicas_endpoints.into(),
        client_stop.clone(),
        my_id.as_u64(),
        application_client_emulator_config.client_workers,
    );

    let client_join = start_client_emulator::<ABC, APP>(
        application_client_emulator_config,
        args,
        shared,
        &mut seed_rng,
        recv_primary_update,
    );

    start_stats_to_net(recv_stats_live, send_stats);

    responder.reply(()).await?;

    let (StopMsg, responder) =
        connection::recv_message_of_type::<StopMsg, _>(&mut orchestrator_con).await?;

    // Gracefully shutdown clients.
    info!("received stop command");
    client_stop.cancel();
    info!("waiting for client results");
    let stats = client_join.await??;
    info!("clients stopped");
    responder.reply(()).await?;

    info!("waiting for stats command");
    let (client_stats_msg, responder) =
        connection::recv_message_of_type::<CollectClientStatsMsg, _>(&mut orchestrator_con).await?;
    if let Some(run_id) = client_stats_msg.1 {
        client_stats_msg
            .0
            .save_client_samples(run_id, stats)
            .await?;
    } else {
        info!("no saving of stats requested");
    }
    responder.reply(()).await?;

    info!("waiting for quit command");
    let (_, responder) =
        connection::recv_message_of_type::<QuitMsg, _>(&mut orchestrator_con).await?;
    responder.reply(()).await?;
    info!("quitting");

    span.exit();
    Ok(())
}

fn start_client_emulator<ABC: AtomicBroadcast, APP: Application<Transaction = ABC::Transaction>>(
    config: ApplicationClientEmulatorConfig<APP::Config>,
    args: ApplicationClientEmulatorArguments,
    shared: SharedFactory<APP::Request, APP::Response>,
    seed: &mut SeedRng,
    mut recv_primary_update: mpsc::Receiver<Option<ReplicaId>>,
) -> JoinHandle<Result<Vec<ClientStats>>> {
    match config.smr_client {
        SmrClient::Bft => APP::ClientEmulator::start(
            BftClientFactory {
                shared,
                t: config.t,
            },
            config,
            args,
        ),
        SmrClient::Fallback => {
            let primary_hint = CurrentPrimary::new();
            {
                let primary_hint = primary_hint.clone();
                tokio::spawn(async move {
                    while let Some(primary) = recv_primary_update.recv().await {
                        primary_hint.set(primary);
                    }
                });
            }
            APP::ClientEmulator::start(
                FallbackClientFactory {
                    shared,
                    primary_hint,
                    rng: EndRng::from_rng(seed).unwrap(),
                    fallback_timeout: config.fallback_timeout,
                },
                config,
                args,
            )
        }
    }
}

pub(crate) fn start_stats_to_net(channel: mpsc::Receiver<Micros>, net: FramedWriteQuinn) {
    async fn run(mut channel: mpsc::Receiver<Micros>, mut net: FramedWriteQuinn) {
        let mut interval = time::interval(Duration::from_millis(100));
        interval.set_missed_tick_behavior(time::MissedTickBehavior::Delay);
        while let Some(data) = channel.recv().await {
            let mut count = 1;
            let mut sum = data;

            loop {
                select! {
                    maybe_data = channel.recv() => {
                        match maybe_data {
                            Some(data) => {
                                count += 1;
                                sum += data;
                            },
                            None => break,
                        }
                    }
                    _ = interval.tick() => {
                        break;
                    }
                }
            }

            let to_send: (u64, Micros) = (count, sum);

            let bytes = match bincode::serialize(&to_send) {
                Ok(bytes) => bytes,
                Err(e) => {
                    error!("{e}");
                    break;
                }
            };
            let bytes = Bytes::from(bytes);
            match net.send(bytes).await {
                Ok(()) => {}
                Err(e) => {
                    error!("{e}");
                    break;
                }
            }
        }
    }

    tokio::spawn(run(channel, net));
}
