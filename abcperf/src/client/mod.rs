use std::{fs::File, net::SocketAddr, path::PathBuf};

use crate::{atomic_broadcast::ConfigurationExtension, time::SharedTime};
use crate::{message::StartTimeMsg, stats::Stats};

use clap::Args;
use rand::{rngs::OsRng, Rng, SeedableRng};

use tokio::{select, signal, sync::oneshot, time};

use tracing::{debug, error, error_span, info};

use anyhow::{anyhow, Result};

pub(crate) mod save;

use crate::{
    config::{ClientConfigExt, Config},
    connection::{ReplicaConnection, ReplicaConnections},
    message::{CollectStatsMsg, InitMsg, PeerPortMsg, QuitMsg, SetupPeersMsg, StartMsg, StopMsg},
    AtomicBroadcast, ClientEmulator, MainSeed, SeedRng, VersionInfo,
};

use self::save::SaveOpt;

#[derive(Args)]
pub(super) struct ClientOpt {
    #[clap(flatten)]
    inner: ClientInnerOpt,

    /// config file to use
    config: PathBuf,
}

#[derive(Args)]
pub(super) struct ClientInnerOpt {
    #[clap(flatten)]
    save: SaveOpt,

    #[clap(long, default_value = "")]
    description: String,

    /// the rng seed that should be used (defaults to a random seed)
    #[clap(long, value_parser = decode_seed)]
    seed: Option<MainSeed>,

    /// port the client listens on for incoming connections from replicas
    listen: SocketAddr,
}

impl ClientInnerOpt {
    #[cfg(test)]
    pub(crate) fn new(
        save: SaveOpt,
        description: String,
        seed: Option<MainSeed>,
        listen: impl Into<SocketAddr>,
    ) -> Self {
        Self {
            save,
            description,
            seed,
            listen: listen.into(),
        }
    }
}

fn decode_seed(input: &str) -> Result<MainSeed> {
    base64::decode_config(input, base64::URL_SAFE)?
        .try_into()
        .map_err(|v: Vec<u8>| anyhow!("invalid seed length: {}", v.len()))
}

/// main method for when started in client mode
pub(super) async fn main<A: AtomicBroadcast, C: ClientEmulator>(
    opt: ClientOpt,
    algo_info: VersionInfo,
    client_emulator: C,
) -> Result<()> {
    let client_span = error_span!("client-emulator").entered();

    let config = serde_yaml::from_reader(File::open(opt.config)?)?;

    let out = run::<A::Config, C>(opt.inner, config, algo_info, client_emulator, None).await;

    client_span.exit();

    out
}
pub(super) async fn run<AC: ConfigurationExtension, C: ClientEmulator>(
    opt: ClientInnerOpt,
    config: Config<AC, C::Config>,
    algo_info: VersionInfo,
    client_emulator: C,
    addr_info: Option<oneshot::Sender<SocketAddr>>,
) -> Result<()> {
    let seed: MainSeed = opt.seed.unwrap_or_else(|| OsRng::default().gen());
    let seed_string = base64::encode_config(seed, base64::URL_SAFE);
    info!("using seed: '{}'", seed_string);
    let mut seed_rng = SeedRng::from_seed(seed);

    let endpoint = crate::quic_server(opt.listen)?;
    let local_addr = endpoint.local_addr()?;
    info!("listening on {}", local_addr);
    if let Some(addr_info) = addr_info {
        let _ = addr_info.send(local_addr);
    }

    let mut replicas =
        wait_for_replicas::<AC, C::Config>(endpoint, &config, &algo_info, &mut seed_rng).await?;

    info!("all peers connected");

    let ports = replicas.send_all(PeerPortMsg).await?;
    let addresses = replicas
        .remote_addresses()
        .zip(ports.into_iter().map(|r| r.incoming_peer_connection_port))
        .map(SocketAddr::from)
        .collect();

    let results = replicas.send_all(SetupPeersMsg(addresses)).await?;
    for (id, res) in results.into_iter().enumerate() {
        res.map_err(|e| anyhow!("peer {} failed to connect to other peers: {}", id, e))?
    }
    info!("all peer connected to each other");

    info!("starting replicas");
    let web_ports = replicas.send_all(StartMsg).await?;

    let replicas_endpoints: Vec<_> = replicas
        .ids()
        .zip(
            replicas
                .remote_addresses()
                .zip(web_ports.iter().map(|r| r.server_port))
                .map(SocketAddr::from),
        )
        .collect();

    info!("starting benchmark");
    let start_time = SharedTime::synced_now();
    let (client_stop, client_join) = client_emulator.start(
        config.client(),
        replicas_endpoints,
        &mut seed_rng,
        start_time.into(),
    );

    replicas.send_all(StartTimeMsg(start_time)).await?;

    let duration = select! {
        ctrl_c = signal::ctrl_c() => {
            ctrl_c?;
            info!("stop requested by user");
            start_time.elapsed()
        }
        _ = time::sleep_until((start_time + config.experiment_duration()).into()) => {
            info!("stopping because {}s passed", config.experiment_duration);
            config.experiment_duration()
        }
    };
    info!("experiment ran for: {:?}", duration);

    let end_time = start_time + duration;

    info!("stopping clients");
    let _ = client_stop.send(());

    let clients = client_join.await??;
    info!("clients stopped");

    info!("stopping benchmark");
    replicas.send_all(StopMsg).await?;

    info!("collecting stats");
    let replicas_stats = replicas.send_all(CollectStatsMsg).await?;

    let results = Stats::new(
        opt.description,
        seed_string,
        start_time.unix(),
        end_time.unix(),
        clients,
        replicas_stats,
        config,
        algo_info,
    );
    opt.save.save_results(results).await?;

    info!("quitting");
    replicas.send_all(QuitMsg).await?;

    Ok(())
}

async fn wait_for_replicas<AC: ConfigurationExtension, CC: ClientConfigExt>(
    endpoint: quinn::Endpoint,
    config: &Config<AC, CC>,
    algo_info: &VersionInfo,
    seed: &mut SeedRng,
) -> Result<ReplicaConnections> {
    let faulty = config.gen_faulty(seed);
    info!("selected faulty replicas: {:?}", faulty.as_slice());

    let mut replicas = Vec::new();
    for replica_id in config.replicas() {
        let faulty = faulty.contains(&replica_id);
        let seed = seed.gen();
        loop {
            let conn = if let Some(conn) = endpoint.accept().await {
                debug!("connection incoming");
                conn.await?
            } else {
                return Err(anyhow!(
                    "missing replica connection, only got {} need {}",
                    replicas.len(),
                    config.n
                ));
            };
            debug!("connection established");

            let addr = conn.remote_address();

            let mut connection = ReplicaConnection::new(replica_id, conn);

            let response = connection
                .send(InitMsg::new(
                    replica_id,
                    config,
                    algo_info.clone(),
                    seed,
                    faulty,
                ))
                .await?;
            match response {
                Ok(_) => {
                    info!("assigned {:?} to peer {}", replica_id, addr);
                    replicas.push(connection);
                    if replicas.len() as u64 == config.n.get() {
                        return Ok(ReplicaConnections::new(replicas));
                    }
                }
                Err(e) => {
                    error!("connection with {} failed: {}", addr, e);
                    continue;
                }
            }
            break;
        }
    }
    unreachable!("we should not run out of ids")
}
