use std::{
    borrow::Cow,
    env,
    fmt::Debug,
    net::SocketAddr,
    sync::Arc,
    time::{Duration, Instant},
};

use crate::stats::ClientStats;
use async_trait::async_trait;
pub use atomic_broadcast::AtomicBroadcast;
pub use atomic_broadcast::AtomicBroadcastChannels;
pub use atomic_broadcast::AtomicBroadcastConfiguration;
use atomic_broadcast::{Decision, Transaction};
use config::{ClientConfig, ClientConfigExt};
use futures::Future;
use ip_family::IpFamilyExt;
use serde::{Deserialize, Serialize};
use shared_ids::{ClientId, ReplicaId};
use tokio::{
    sync::{mpsc, oneshot},
    task::JoinHandle,
};

use clap::{Args, Parser, Subcommand};

use anyhow::Result;

use quinn::{
    ClientConfig as QuinnClientConfig, Connection, Endpoint, ServerConfig, TransportConfig, VarInt,
};
use rustls::{Certificate, PrivateKey, RootCertStore};
use tracing::Level;
use tracing_subscriber::FmtSubscriber;
use trait_alias_macro::pub_trait_alias_macro;

pub mod atomic_broadcast;
mod client;
pub mod config;
mod connection;
pub mod generator;
mod message;
pub mod replica;
pub mod stats;
#[cfg(test)]
mod tests;
mod time;

pub type MainSeed = [u8; 32];
pub type SeedRng = rand_chacha::ChaCha20Rng;
pub type EndRng = rand_pcg::Pcg64Mcg;

//#[cfg(debug_assertions)]
//const DEFAULT_LEVEL: &str = "debug";
//#[cfg(not(debug_assertions))]
const DEFAULT_LEVEL: &str = "info";

pub const READ_TO_END_LIMIT: usize = 128 * 1024 * 1024;
pub const CERTIFICATE_AUTHORITY: &[u8] = include_bytes!("../debug-certs/root-cert.der");
pub const PRIVATE_KEY: &[u8] = include_bytes!("../debug-certs/key.der");
pub const CERTIFICATE: &[u8] = include_bytes!("../debug-certs/cert.der");

#[derive(Parser)]
struct ABCperfOpt<O: Args> {
    #[clap(flatten)]
    algo_opt: O,

    #[clap(flatten)]
    own_opt: ABCperfArgs,
}

#[doc(hidden)]
#[derive(Args)]
pub struct ABCperfArgs {
    #[clap(subcommand)]
    command: Command,

    #[clap(long, default_value = DEFAULT_LEVEL)]
    log_level: Level,
}

#[derive(Subcommand)]
enum Command {
    /// Information about this binary
    Info,

    /// Commands that need a async runtime
    #[command(flatten)]
    Async(AsyncCommand),
}

#[derive(Subcommand)]
enum AsyncCommand {
    /// Run in client mode
    Client(Box<client::ClientOpt>),
    /// Run in replica mode
    Replica(Box<replica::ReplicaOpt>),
}

/// Used to integrate a client emulator into abcperf
pub trait ClientEmulator {
    /// Custom configuration for the client emulator
    type Config: ClientConfigExt;

    /// Run the client emulator
    fn start(
        self,
        config: ClientConfig<Self::Config>,
        replicas: Vec<(ReplicaId, SocketAddr)>,
        seed: &mut SeedRng,
        start_time: Instant,
    ) -> (oneshot::Sender<()>, JoinHandle<Result<Vec<ClientStats>>>);
}

/// Used to integrate a server into abcperf
#[async_trait]
pub trait Server: 'static {
    /// Requests send to the algorithm by the server
    type AlgoRequest: Transaction;

    /// Responses recived from the algorithm by the server
    type AlgoResponse: Decision;

    /// Messages the server sends between replicas
    type ReplicaMessage: ServerReplicaMessage;

    /// Custom configuration for the server
    type Config: ClientConfigExt;

    #[allow(clippy::too_many_arguments)]
    async fn run<F: Send + Future<Output = ()>>(
        self,
        config: ClientConfig<Self::Config>,
        requests: mpsc::Sender<(ClientId, Self::AlgoRequest)>,
        responses: mpsc::Receiver<Self::AlgoResponse>,
        exit: oneshot::Receiver<()>,
        ready: oneshot::Sender<SocketAddr>,
        local_socket: SocketAddr,
        replica_send: impl 'static + Sync + Send + Fn(MessageDestination, Self::ReplicaMessage) -> F,
        replica_recv: mpsc::Receiver<(MessageType, ReplicaId, Self::ReplicaMessage)>,
    );
}

pub_trait_alias_macro!(ServerReplicaMessage = for<'a> Deserialize<'a> + Serialize + Send + Debug);

pub fn main<
    A: AtomicBroadcast,
    S: Server<AlgoRequest = A::Transaction, AlgoResponse = A::Decision>,
    C: ClientEmulator<Config = S::Config>,
    N: Into<Cow<'static, str>>,
>(
    name: N,
    version: &'static str,
    algo: impl FnOnce() -> A,
    server: impl FnOnce() -> S,
    client_emulator: impl FnOnce() -> C,
) -> Result<()> {
    #[derive(Args)]
    struct Empty;

    let (Empty, abcperf_args) = parse_args()?;

    main_with_args(abcperf_args, name, version, algo, server, client_emulator)
}

/// parse the command line arguments with the provided additional args
pub fn parse_args<O: Args>() -> Result<(O, ABCperfArgs)> {
    let opt = ABCperfOpt::<O>::try_parse()?;
    Ok((opt.algo_opt, opt.own_opt))
}

/// run abcperf with the provided algorithms and command line arguments
pub fn main_with_args<
    A: AtomicBroadcast,
    S: Server<AlgoRequest = A::Transaction, AlgoResponse = A::Decision>,
    C: ClientEmulator<Config = S::Config>,
    N: Into<Cow<'static, str>>,
>(
    abcperf_args: ABCperfArgs,
    name: N,
    version: &'static str,
    algo: impl FnOnce() -> A,
    server: impl FnOnce() -> S,
    client_emulator: impl FnOnce() -> C,
) -> Result<()> {
    tracing::subscriber::set_global_default(
        FmtSubscriber::builder()
            .with_max_level(abcperf_args.log_level)
            .finish(),
    )?;

    let abcperf_version = env!("CARGO_PKG_VERSION");
    let info = VersionInfo {
        abcperf_version: abcperf_version.into(),
        name: name.into(),
        version: version.into(),
    };

    match abcperf_args.command {
        Command::Info => print_info(info),
        Command::Async(command) => {
            let rt = tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()
                .unwrap();

            match command {
                AsyncCommand::Client(opt) => {
                    rt.block_on(client::main::<A, _>(*opt, info, client_emulator()))
                }
                AsyncCommand::Replica(opt) => rt.block_on(replica::main(*opt, info, algo, server)),
            }
        }
    }
}

fn print_info(
    VersionInfo {
        name,
        version,
        abcperf_version,
    }: VersionInfo,
) -> Result<()> {
    println!("{}", name);
    println!("Version: {}", version);
    println!("ABCperf Version: {}", abcperf_version);
    Ok(())
}

fn server_config() -> Result<ServerConfig> {
    let priv_key = PrivateKey(PRIVATE_KEY.to_vec());
    let cert_chain = vec![Certificate(CERTIFICATE.to_vec())];
    let mut server_config = ServerConfig::with_single_cert(cert_chain, priv_key)?;
    Arc::get_mut(&mut server_config.transport)
        .expect("config was just created")
        .max_idle_timeout(Some(VarInt::from_u32(30_000).into()));
    Ok(server_config)
}

fn client_config() -> Result<QuinnClientConfig> {
    let mut roots = RootCertStore::empty();
    roots.add(&Certificate(CERTIFICATE_AUTHORITY.to_vec()))?;
    let mut client_config = QuinnClientConfig::with_root_certificates(roots);

    let mut transport_config = TransportConfig::default();
    transport_config.max_idle_timeout(Some(VarInt::from_u32(30_000).into()));
    transport_config.keep_alive_interval(Some(Duration::from_secs(10)));
    client_config.transport_config(Arc::new(transport_config));

    Ok(client_config)
}

async fn quic_new_client_connect(addr: SocketAddr, server_name: &str) -> Result<Connection> {
    let endpoint = quic_new_client((addr.family().unspecified(), 0))?;

    let connection = endpoint.connect(addr, server_name)?.await?;

    Ok(connection)
}

pub fn quic_new_client(bind: impl Into<SocketAddr>) -> Result<Endpoint> {
    let mut endpoint = Endpoint::client(bind.into())?;
    endpoint.set_default_client_config(crate::client_config()?);
    Ok(endpoint)
}

pub fn quic_server(addr: impl Into<SocketAddr>) -> Result<Endpoint> {
    let mut endpoint = Endpoint::server(server_config()?, addr.into())?;
    endpoint.set_default_client_config(crate::client_config()?);
    Ok(endpoint)
}

#[cfg(not(target_os = "windows"))]
const USER_VAR: &str = "USER";
#[cfg(target_os = "windows")]
const USER_VAR: &str = "USERNAME";

#[derive(Debug, Deserialize, Serialize, Eq, PartialEq, Clone)]
struct VersionInfo {
    abcperf_version: Cow<'static, str>,
    name: Cow<'static, str>,
    version: Cow<'static, str>,
}

#[derive(Debug)]
pub enum MessageDestination {
    Unicast(ReplicaId),
    Broadcast,
}

#[derive(Debug, PartialEq, Eq, Hash, Clone, Copy, Serialize, Deserialize)]
pub enum MessageType {
    Unicast,
    Broadcast,
}

impl AsRef<MessageType> for MessageDestination {
    fn as_ref(&self) -> &MessageType {
        match self {
            Self::Unicast(_) => &MessageType::Unicast,
            Self::Broadcast => &MessageType::Broadcast,
        }
    }
}

impl<T: AsRef<MessageType>> From<T> for MessageType {
    fn from(value: T) -> Self {
        *value.as_ref()
    }
}
