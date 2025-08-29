use anyhow::{bail, ensure, Error, Result};
use itertools::Itertools;
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, HashSet},
    fmt::Debug,
    fs::File,
    num::NonZeroU64,
    ops::Deref,
    path::Path,
    str::FromStr,
    time::Duration,
};

use derivative::Derivative;
use shared_ids::{ClientId, IdIter, ReplicaId};

use crate::{
    application::{smr_client::SmrClient, ApplicationConfig},
    atomic_broadcast::ABCConfig,
    client::ClientWorkerId,
};

use self::update::{
    C2RNetworkConfigUpdate, FaultEmulationConfigUpdate, Invocateable, LoadConfigUpdate,
    R2RNetworkConfigUpdate,
};

pub(crate) mod update;

#[derive(Deserialize, Serialize, Derivative)]
#[serde(bound = "", try_from = "Config<ABC, APP>", into = "Config<ABC, APP>")]
#[derivative(Clone(bound = ""), Debug(bound = ""))]
#[repr(transparent)]
pub(super) struct ValidatedConfig<
    ABC: ABCConfig = EmptyConfig,
    APP: ApplicationConfig = EmptyConfig,
>(Config<ABC, APP>);

impl<ABC: ABCConfig, APP: ApplicationConfig> From<ValidatedConfig<ABC, APP>> for Config<ABC, APP> {
    fn from(config: ValidatedConfig<ABC, APP>) -> Self {
        config.0
    }
}

impl<ABC: ABCConfig, APP: ApplicationConfig> TryFrom<Config<ABC, APP>>
    for ValidatedConfig<ABC, APP>
{
    type Error = anyhow::Error;

    fn try_from(config: Config<ABC, APP>) -> Result<Self> {
        let first = if let Some(first) = config.orchestrator_request_generation.load.first() {
            first
        } else {
            bail!("no load configured")
        };

        ensure!(
            first.invocation_time == 0f64,
            "first entry should have invocation_time 0"
        );

        for (a, b) in config
            .orchestrator_request_generation
            .load
            .iter()
            .tuple_windows()
        {
            ensure!(
                a.invocation_time < b.invocation_time,
                "invocation_time should be stricly incrementing"
            );
        }

        for load in &config.orchestrator_request_generation.load {
            ensure!(
                load.config.ticks_per_second.get() <= 1000 * config.basic.client_workers.get(),
                "more then 1000 ticks_per_second per client worker is not supported"
            );
        }

        for (a, b) in config.network.client_to_replica.iter().tuple_windows() {
            ensure!(
                a.invocation_time < b.invocation_time,
                "invocation_time should be stricly incrementing"
            );
        }

        for (a, b) in config.network.replica_to_replica.iter().tuple_windows() {
            ensure!(
                a.invocation_time < b.invocation_time,
                "invocation_time should be stricly incrementing"
            );
        }

        for (a, b) in config.fault_emulation.iter().tuple_windows() {
            ensure!(
                a.invocation_time < b.invocation_time,
                "invocation_time should be stricly incrementing"
            );
        }

        let mut ids = HashSet::new();
        for load in &config.fault_emulation {
            ensure!(
                load.config.omission_chance >= 0.0,
                "omission chance should be >= 0"
            );
            ensure!(
                load.config.omission_chance <= 1.0,
                "omission chance should be <= 1"
            );
            ids.insert(load.config.replica_id);
            ensure!(
                load.config.replica_id.as_u64() < config.n.get(),
                "invalid replica id"
            );
        }
        ensure!(ids.len() as u64 <= config.t, "more than t faulty replicas");

        Ok(ValidatedConfig(config))
    }
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub(super) struct EmptyConfig {}

/// Models the configuration of ABCperf.
#[derive(Deserialize, Serialize, Derivative)]
#[serde(bound = "")]
#[derivative(Clone(bound = ""), Debug(bound = ""))]
pub(super) struct Config<ABC: ABCConfig = EmptyConfig, APP: ApplicationConfig = EmptyConfig> {
    #[serde(flatten)]
    pub(super) basic: BasicConfig,

    /// The duration that the experiment should run for.
    pub(super) experiment_duration: Option<f64>,

    pub(super) abc: ABCConfigWrapper<ABC>,

    pub(super) fault_emulation: Vec<Invocateable<FaultEmulationConfigUpdate>>,

    pub(super) application: ApplicationConfigWrapper<APP>,

    pub(super) orchestrator_request_generation: OrchestratorRequestGenerationConfig,

    pub(super) network: NetworkConfig,
}

impl<ABC: ABCConfig, APP: ApplicationConfig> Deref for Config<ABC, APP> {
    type Target = NTConfig;

    fn deref(&self) -> &Self::Target {
        &self.basic.nt
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BasicConfig {
    #[serde(flatten)]
    pub(super) nt: NTConfig,

    pub(super) client_workers: NonZeroU64,

    pub(super) experiment_label: String,

    pub(super) run_tags: HashMap<String, String>,
}

impl Deref for BasicConfig {
    type Target = NTConfig;

    fn deref(&self) -> &Self::Target {
        &self.nt
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NTConfig {
    /// The total amount of replicas in the algorithm.
    pub n: NonZeroU64,
    /// The maximum amount of faulty replicas possible.
    /// At the maximum amount, the algorithm still guarantees to make progress.
    pub t: u64,
}

impl NTConfig {
    /// Returns the IDs of all replicas in the algorithm.
    pub fn replicas(&self) -> impl Iterator<Item = ReplicaId> {
        IdIter::default().take(self.n.get().try_into().unwrap())
    }
}

#[derive(Deserialize, Serialize, Derivative)]
#[serde(bound = "")]
#[derivative(Clone(bound = ""), Debug(bound = ""))]
pub(super) struct ABCConfigWrapper<ABC: ABCConfig> {
    pub(super) algorithm: String,
    pub(super) config: ABC,
}

#[derive(Deserialize, Serialize, Derivative)]
#[serde(bound = "")]
#[derivative(Clone(bound = ""), Debug(bound = ""))]
pub(super) struct ApplicationConfigWrapper<APP: ApplicationConfig> {
    pub(super) name: String,
    pub(super) config: APP,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct OrchestratorRequestGenerationConfig {
    pub(super) load: Vec<Invocateable<LoadConfigUpdate>>,
    pub(super) smr_client: SmrClient,
    pub(crate) fallback_timeout: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct NetworkConfig {
    pub(super) replica_to_replica: Vec<Invocateable<R2RNetworkConfigUpdate>>,
    pub(super) client_to_replica: Vec<Invocateable<C2RNetworkConfigUpdate>>,
}

impl<ABC: ABCConfig, APP: ApplicationConfig> ValidatedConfig<ABC, APP> {
    pub(super) fn load(path: impl AsRef<Path>) -> Result<Self> {
        Ok(serde_yaml::from_reader(File::open(path)?)?)
    }
}

impl<ABC: ABCConfig, APP: ApplicationConfig> FromStr for ValidatedConfig<ABC, APP> {
    type Err = Error;

    fn from_str(config: &str) -> Result<Self> {
        Ok(serde_yaml::from_str(config)?)
    }
}

impl<ABC: ABCConfig, APP: ApplicationConfig> Config<ABC, APP> {
    /// Creates the [AtomicBroadcastConfiguration] for the specified replica.
    ///
    /// # Arguments
    ///
    /// * `replica_id` - The ID of the replica for which the
    ///                  [AtomicBroadcastConfiguration] should be created.
    pub(super) fn algo(&self, replica_id: ReplicaId) -> AtomicBroadcastConfiguration<ABC> {
        AtomicBroadcastConfiguration {
            extension: self.abc.config.clone(),
            nt: self.basic.nt.clone(),
            replica_id,
        }
    }

    /// Creates the [ClientEmulatorConfig] using the [Config].
    pub(super) fn application_client_emulator(
        &self,
        worker_id: ClientWorkerId,
    ) -> ApplicationClientEmulatorConfig<APP> {
        ApplicationClientEmulatorConfig {
            nt: self.basic.nt.clone(),
            worker_id,
            client_workers: self.basic.client_workers,
            application: self.application.config.clone(),
            smr_client: self.orchestrator_request_generation.smr_client.clone(),
            fallback_timeout: Duration::from_secs_f64(
                self.orchestrator_request_generation.fallback_timeout,
            ),
        }
    }

    pub(super) fn server(&self, replica_id: ReplicaId) -> ApplicationServerConfig<APP> {
        ApplicationServerConfig {
            nt: self.basic.nt.clone(),
            replica_id,
            application: self.application.config.clone(),
        }
    }

    /// Returns the [Duration] from the set `experiment_duration`
    /// in [Config].
    pub(super) fn experiment_duration(&self) -> Option<Duration> {
        self.experiment_duration.map(Duration::from_secs_f64)
    }
}

/// Models the configuration of the clients.
pub struct ApplicationClientEmulatorConfig<APP: ApplicationConfig> {
    pub nt: NTConfig,
    pub worker_id: ClientWorkerId,
    pub client_workers: NonZeroU64,
    pub application: APP,
    pub(crate) fallback_timeout: Duration,
    pub(crate) smr_client: SmrClient,
}

impl<APP: ApplicationConfig> Deref for ApplicationClientEmulatorConfig<APP> {
    type Target = NTConfig;

    fn deref(&self) -> &Self::Target {
        &self.nt
    }
}

impl<APP: ApplicationConfig> ApplicationClientEmulatorConfig<APP> {
    pub fn client_ids(&self) -> impl Iterator<Item = ClientId> {
        IdIter::default()
            .skip(self.worker_id.as_u64() as usize)
            .step_by(self.client_workers.get() as usize)
    }
}

/// Models the configuration of the server.
pub struct ApplicationServerConfig<APP: ApplicationConfig> {
    /// The ID of the replica.
    pub replica_id: ReplicaId,
    pub nt: NTConfig,
    pub application: APP,
}

impl<APP: ApplicationConfig> Deref for ApplicationServerConfig<APP> {
    type Target = NTConfig;

    fn deref(&self) -> &Self::Target {
        &self.nt
    }
}

/// Models the configuration options of the [AtomicBroadcast].
pub struct AtomicBroadcastConfiguration<A: ABCConfig> {
    /// The ID of the replica.
    pub replica_id: ReplicaId,
    pub nt: NTConfig,
    /// The extension of the configuration.
    pub extension: A,
}

impl<A: ABCConfig> Deref for AtomicBroadcastConfiguration<A> {
    type Target = NTConfig;

    fn deref(&self) -> &Self::Target {
        &self.nt
    }
}

impl<A: ABCConfig> AtomicBroadcastConfiguration<A> {
    /// Returns the IDs of all replicas in the algorithm.
    pub fn replicas(&self) -> impl Iterator<Item = ReplicaId> {
        IdIter::default().take(self.nt.n.get().try_into().unwrap())
    }
}

/// Enumerates the possible distributions used for the simulation of
/// network latency.
#[derive(Deserialize, Serialize, Clone, Debug)]
pub enum DistributionConfig {
    /// Models the constant distribution for the network latency.
    #[serde(rename = "constant")]
    Constant,

    /// Models the uniform distribution for the network latency.
    #[serde(rename = "uniform")]
    Uniform,

    /// Models the normal distribution for the network latency.
    #[serde(rename = "normal")]
    Normal { std_dev: f64 },

    /// Models the exponential distribution for the network latency.
    #[serde(rename = "exponential")]
    Exponential,
}
