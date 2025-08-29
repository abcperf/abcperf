use std::{
    borrow::Cow,
    env,
    fmt::{Debug, Display},
    net::SocketAddr,
    sync::Arc,
    time::Duration,
};

use application::Application;
pub use atomic_broadcast::ABCChannels;
pub use atomic_broadcast::AtomicBroadcast;
use config::{Config, ValidatedConfig};
use ip_family::IpFamilyExt;
use orchestrator::OrchestratorOpt;
use serde::{Deserialize, Serialize};
use shared_ids::ReplicaId;
use tokio::runtime::Runtime;
use worker::OrchestratedInterStageState;

use clap::{Parser, Subcommand};

use anyhow::Result;

use quinn::{
    ClientConfig as QuinnClientConfig, Connection, Endpoint, ServerConfig as QuinnServerConfig,
    TransportConfig, VarInt,
};
use rustls::{Certificate, PrivateKey, RootCertStore};
use tracing::level_filters::LevelFilter;
use tracing_subscriber::EnvFilter;

pub mod application;
pub mod atomic_broadcast;
mod client;
pub mod config;
mod connection;
pub mod generator;
pub mod live_message;
mod local;
mod message;
mod net_channel;
mod orchestrator;
pub mod replica;
pub mod stats;
#[cfg(test)]
mod tests;
mod time;
mod worker;

pub type MainSeed = [u8; 32];
pub type SeedRng = rand_chacha::ChaCha20Rng;
pub type EndRng = rand_pcg::Pcg64Mcg;

//#[cfg(debug_assertions)]
//const DEFAULT_LEVEL: &str = "debug";
//#[cfg(not(debug_assertions))]
const DEFAULT_LEVEL: &str = "info";

/// Constant to read the maximum amount of bytes from a stream.
pub const READ_TO_END_LIMIT: usize = 128 * 1024 * 1024;
/// Constant that defines the path to the certificate authority in DER format.
pub const CERTIFICATE_AUTHORITY_DER: &[u8] = include_bytes!("../debug-certs/root-cert.der");
/// Constant that defines the path to the certificate authority in PEM format.
pub const CERTIFICATE_AUTHORITY_PEM: &[u8] = include_bytes!("../debug-certs/root-cert.pem");
/// Constant that defines the path to the private key of the QUIC server to be
/// created in DER format.
pub const PRIVATE_KEY_DER: &[u8] = include_bytes!("../debug-certs/key.der");
/// Constant that defines the path to the private key of the QUIC server to be
/// created in PEM format.
pub const PRIVATE_KEY_PEM: &[u8] = include_bytes!("../debug-certs/key.pem");
/// Constant that defines the path to the certificate of the QUIC server to be
/// created in DER format.
pub const CERTIFICATE_DER: &[u8] = include_bytes!("../debug-certs/cert.der");
/// Constant that defines the path to the certificate of the QUIC server to be
/// created in PEM format.
pub const CERTIFICATE_PEM: &[u8] = include_bytes!("../debug-certs/cert.pem");

/// Models the own arguments that ABCperf takes and with which it can be
/// configured.
#[derive(Parser)]
struct ABCperfArgs {
    /// The arguments that define which command or mode to be run.
    #[clap(subcommand)]
    command: Command,

    /// The log level to be set.
    #[clap(long, default_value = DEFAULT_LEVEL, env = "RUST_LOG")]
    log_level: String,
}

/// Enumerates the commands that may be run when running ABCperf.
#[derive(Subcommand)]
enum Command {
    /// Information about this binary.
    Info,

    /// Commands that need an async runtime.
    #[command(flatten)]
    Async(AsyncCommand),
}

/// Enumerates the async commands (modes) that may be run when running ABCperf.
#[derive(Subcommand)]
enum AsyncCommand {
    /// Run in orchestrator mode.
    Orchestrator(Box<orchestrator::OrchestratorOpt>),
    /// Run in replica mode.
    Replica(Box<replica::ReplicaOpt>),
    /// Run in client mode.
    Client(Box<client::ClientOpt>),

    Local(Box<local::LocalOpt>),
}

pub struct InterStageState {
    runtime: Runtime,
    inner: InnerInterStageState,
}

impl InterStageState {
    fn new(runtime: Runtime, inner: InnerInterStageState) -> Self {
        Self { runtime, inner }
    }
}

pub struct InnerInterStageState {
    abc_algorithm: String,
    state_machine_application: String,
    config_string: String,
    mode: SecondStageMode,
    version_info: VersionInfo,
}

enum SecondStageMode {
    Orchestrator(Box<OrchestratorOpt>),
    Orchestrated(Box<OrchestratedInterStageState>),
}

pub fn stage_1(
    bin_name: &'static str,
    bin_version: &'static str,
) -> Result<Option<InterStageState>> {
    InterStageState::stage_1(bin_name, bin_version)
}

impl InterStageState {
    fn stage_1(bin_name: &'static str, bin_version: &'static str) -> Result<Option<Self>> {
        let opt = ABCperfArgs::try_parse()?;

        #[cfg(not(feature = "tokio-console"))]
        tracing_subscriber::fmt()
            .with_env_filter(
                EnvFilter::builder()
                    .with_default_directive(LevelFilter::ERROR.into())
                    .parse_lossy(&opt.log_level),
            )
            .init();

        let abcperf_version = env!("CARGO_PKG_VERSION");

        let version_info = VersionInfo {
            abcperf_version: abcperf_version.into(),
            bin_name: bin_name.into(),
            bin_version: bin_version.into(),
        };

        let command = match opt.command {
            Command::Info => {
                print_info(version_info)?;
                return Ok(None);
            }
            Command::Async(command) => command,
        };

        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .unwrap();

        Ok(match command {
            AsyncCommand::Orchestrator(opt) => Some(orchestrator::stage_1(opt, version_info)?),
            AsyncCommand::Replica(opt) => {
                Some(runtime.block_on(worker::stage_1(*opt, version_info))?)
            }
            AsyncCommand::Client(opt) => {
                Some(runtime.block_on(worker::stage_1(*opt, version_info))?)
            }
            AsyncCommand::Local(opt) => {
                runtime.block_on(local::main(*opt))?;
                None
            }
        }
        .map(|i| InterStageState::new(runtime, i)))
    }

    pub fn stage_2<ABC: AtomicBroadcast, APP: Application<Transaction = ABC::Transaction>>(
        self,
    ) -> Result<()> {
        let config: Config<ABC::Config, APP::Config> = self
            .inner
            .config_string
            .parse::<ValidatedConfig<ABC::Config, APP::Config>>()?
            .into();
        match self.inner.mode {
            SecondStageMode::Orchestrator(opt) => {
                self.runtime.block_on(orchestrator::main::<ABC, APP>(
                    *opt,
                    self.inner.version_info,
                    config,
                ))
            }
            SecondStageMode::Orchestrated(state) => {
                self.runtime
                    .block_on(worker::stage_2::<ABC, APP>(*state, config))
            }
        }
    }

    pub fn abc_algorithm(&self) -> &str {
        &self.inner.abc_algorithm
    }

    pub fn state_machine_application(&self) -> &str {
        &self.inner.state_machine_application
    }
}

/// Prints the information on ABCperf and the set algorithm via the command
/// line arguments.
///
/// # Arguments
///
/// * `VersionInfo` - The information on ABCperf and the set algorithm.
fn print_info(
    VersionInfo {
        bin_name: name,
        bin_version: version,
        abcperf_version,
    }: VersionInfo,
) -> Result<()> {
    println!("{}", name);
    println!("Version: {}", version);
    println!("ABCperf Version: {}", abcperf_version);
    Ok(())
}

/// Creates the server config for the QUIC endpoint.
fn server_config() -> Result<QuinnServerConfig> {
    let priv_key = PrivateKey(PRIVATE_KEY_DER.to_vec());
    let cert_chain = vec![Certificate(CERTIFICATE_DER.to_vec())];
    let mut server_config = QuinnServerConfig::with_single_cert(cert_chain, priv_key)?;
    Arc::get_mut(&mut server_config.transport)
        .expect("config was just created")
        .max_idle_timeout(Some(VarInt::from_u32(30_000).into()));
    Ok(server_config)
}

/// Creates the client config for the QUIC endpoint.
fn client_config() -> Result<QuinnClientConfig> {
    let mut roots = RootCertStore::empty();
    roots.add(&Certificate(CERTIFICATE_AUTHORITY_DER.to_vec()))?;
    let mut client_config = QuinnClientConfig::with_root_certificates(roots);

    let mut transport_config = TransportConfig::default();
    transport_config.max_idle_timeout(Some(Duration::from_secs(120).try_into().unwrap()));
    transport_config.keep_alive_interval(Some(Duration::from_secs(1)));
    client_config.transport_config(Arc::new(transport_config));

    Ok(client_config)
}

/// Creates a new QUIC client endpoint and connects it to the provided remote
/// address.
///
/// # Arguments
///
/// * `addr` - The address to connect the new QUIC client endpoint to.
/// * `server_name` - The name of the server that must be covered by the
///                   root certificate.
async fn quic_new_client_connect(addr: SocketAddr, server_name: &str) -> Result<Connection> {
    let endpoint = quic_new_client((addr.family().unspecified(), 0))?;

    let connection = endpoint.connect(addr, server_name)?.await?;

    Ok(connection)
}

/// Creates a new QUIC client endpoint.
///
/// # Arguments
///
/// * `addr` - The local address to bind to.
pub fn quic_new_client(addr: impl Into<SocketAddr>) -> Result<Endpoint> {
    let mut endpoint = Endpoint::client(addr.into())?;
    endpoint.set_default_client_config(crate::client_config()?);
    Ok(endpoint)
}

/// Creates a new QUIC server endpoint.
///
/// # Arguments
///
/// * `addr` - The address to bind to.
pub fn quic_new_server(addr: impl Into<SocketAddr>) -> Result<Endpoint> {
    let mut endpoint = Endpoint::server(server_config()?, addr.into())?;
    endpoint.set_default_client_config(crate::client_config()?);
    Ok(endpoint)
}

#[cfg(not(target_os = "windows"))]
const USER_VAR: &str = "USER";
#[cfg(target_os = "windows")]
const USER_VAR: &str = "USERNAME";

/// Models the information on the running ABCperf instance and algorithm.
#[derive(Debug, Deserialize, Serialize, Eq, PartialEq, Clone)]
struct VersionInfo {
    /// The ABCperf version running.
    abcperf_version: Cow<'static, str>,
    /// The name of the binary running.
    bin_name: Cow<'static, str>,
    /// The version of the binary running.
    bin_version: Cow<'static, str>,
}

/// Enumerates the possible message destinations.
#[derive(Debug)]
pub enum MessageDestination {
    /// Models the destination to a single replica.
    Unicast(ReplicaId),
    /// Models a broadcast.
    Broadcast,
}

/// Enumerates the possible message types.
#[derive(Debug, PartialEq, Eq, Hash, Clone, Copy, Serialize, Deserialize)]
pub enum MessageType {
    /// Models the type unicast.
    Unicast,
    /// Models the type broadcast.
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

impl Display for MessageDestination {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Broadcast => f.write_fmt(format_args!("All")),
            Self::Unicast(id) => f.write_fmt(format_args!("Replica {}", id.as_u64())),
        }
    }
}
