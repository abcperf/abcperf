use serde::{Deserialize, Deserializer, Serialize, Serializer};
use shared_ids::ReplicaId;
use std::{
    collections::{HashMap, HashSet},
    env,
    ops::Sub,
    time::{Duration, Instant},
};

use crate::{
    atomic_broadcast::ConfigurationExtension,
    config::{ClientConfigExt, Config, ReplicasConfig},
    VersionInfo, USER_VAR,
};

#[derive(Deserialize, Serialize, Clone, Copy, Debug)]
#[repr(transparent)]
#[serde(transparent)]
pub struct Micros(u64);

impl From<Duration> for Micros {
    fn from(duration: Duration) -> Self {
        let micros = duration.as_micros();
        assert!(micros <= u128::from(u64::MAX));
        Self(micros as u64)
    }
}

impl From<Micros> for Duration {
    fn from(micros: Micros) -> Self {
        Duration::from_micros(micros.0)
    }
}

impl From<Micros> for u64 {
    fn from(micros: Micros) -> Self {
        micros.0
    }
}

impl Sub<Micros> for Micros {
    type Output = Micros;

    fn sub(self, rhs: Micros) -> Self::Output {
        Self(self.0 - rhs.0)
    }
}

#[derive(Deserialize, Serialize, Clone, Copy, Debug)]
#[repr(transparent)]
#[serde(transparent)]
pub struct Timestamp {
    micros_since_start: Micros,
}

impl Timestamp {
    pub fn new(start: Instant, time: Instant) -> Option<Self> {
        Some(Self {
            micros_since_start: time.checked_duration_since(start)?.into(),
        })
    }
}

impl From<Timestamp> for Duration {
    fn from(timestamp: Timestamp) -> Self {
        timestamp.micros_since_start.into()
    }
}

impl From<Timestamp> for Micros {
    fn from(timestamp: Timestamp) -> Self {
        timestamp.micros_since_start
    }
}

impl From<Timestamp> for u64 {
    fn from(timestamp: Timestamp) -> Self {
        timestamp.micros_since_start.into()
    }
}

impl Sub<Timestamp> for Timestamp {
    type Output = Micros;

    fn sub(self, rhs: Timestamp) -> Self::Output {
        self.micros_since_start - rhs.micros_since_start
    }
}

#[derive(Deserialize, Serialize, Debug)]
pub struct ClientStats {
    pub samples: Vec<ClientSample>,
}

#[derive(Debug, Clone)]
pub struct ClientSample {
    pub timestamp: Timestamp,
    pub processing_time: Micros,
    pub replica_id: ReplicaId,
}

impl Serialize for ClientSample {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let Self {
            timestamp,
            processing_time,
            replica_id,
        } = self;
        (timestamp, processing_time, replica_id).serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for ClientSample {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let (timestamp, processing_time, replica_id) = Deserialize::deserialize(deserializer)?;
        Ok(Self {
            timestamp,
            processing_time,
            replica_id,
        })
    }
}

#[derive(Deserialize, Serialize, Debug)]
pub struct CpuInfo {
    pub common: HashMap<String, String>,
    pub cores: Vec<HashMap<String, String>>,
}

#[derive(Deserialize, Serialize, Debug)]
pub struct KernelInfo {
    pub version: String,
    pub flags: HashSet<String>,
    pub extra: String,
}

#[derive(Deserialize, Serialize, Debug)]
pub struct HostInfo {
    pub hostname: String,
    pub user: String,
    pub memory: u64,
    pub swap: u64,
    pub cpu: CpuInfo,
    pub kernel: KernelInfo,
}

impl HostInfo {
    pub(crate) fn collect() -> Self {
        let cpu_info = procfs::CpuInfo::new().expect("should be available");
        let cpu = CpuInfo {
            common: cpu_info.fields,
            cores: cpu_info.cpus,
        };

        let kernel_info = procfs::sys::kernel::BuildInfo::current().expect("should be available");
        let kernel = KernelInfo {
            version: kernel_info.version,
            flags: kernel_info.flags,
            extra: kernel_info.extra,
        };

        let hostname = gethostname::gethostname()
            .into_string()
            .expect("hostname should be utf8");
        let user = env::var(USER_VAR).unwrap_or_else(|_| "<unknown user>".to_string());
        let mem_info = procfs::Meminfo::new().expect("should be available");
        Self {
            hostname,
            user,
            memory: mem_info.mem_total,
            swap: mem_info.swap_total,
            cpu,
            kernel,
        }
    }
}

#[derive(Deserialize, Serialize, Debug)]
pub struct ReplicaStats {
    pub samples: Vec<ReplicaSample>,
    pub ticks_per_second: u64,
    pub host_info: HostInfo,
}

#[derive(Deserialize, Serialize, Debug)]
pub struct ReplicaSample {
    pub timestamp: Timestamp,
    pub stats: ReplicaSampleStats,
}

#[derive(Deserialize, Serialize, Debug)]
pub struct ReplicaSampleStats {
    pub memory_usage_in_bytes: u64,
    pub memory_used_by_stats: u64,
    pub user_mode_ticks: u64,
    pub kernel_mode_ticks: u64,
    pub rx_bytes: u64,
    pub tx_bytes: u64,
    pub rx_datagrams: u64,
    pub tx_datagrams: u64,
    pub messages: MessageCounts,
}

#[derive(Deserialize, Serialize, Debug)]
pub struct MessageCounts {
    pub algo: MessageCount,
    pub server: MessageCount,
}

#[derive(Deserialize, Serialize, Debug)]
pub struct MessageCount {
    pub unicast: RxTx,
    pub broadcast: RxTx,
}

#[derive(Deserialize, Serialize, Debug)]
pub struct RxTx {
    pub rx: u64,
    pub tx: u64,
}

impl ReplicaSample {
    pub fn ticks(&self) -> u64 {
        self.stats.user_mode_ticks + self.stats.kernel_mode_ticks
    }
}

#[derive(Deserialize, Serialize, Debug)]
pub struct Stats {
    pub clients: Vec<ClientStats>,
    pub replicas: Vec<ReplicaStats>,
    pub client_host_info: HostInfo,
    pub abcperf_version: String,
    pub algo_version: String,
    pub start_time: u64,
    pub end_time: u64,
    pub algo_name: String,
    pub algo_config: HashMap<String, String>,
    pub client_config: HashMap<String, String>,
    pub n: u64,
    pub t: u64,
    pub f: u64,
    pub omission_chance: f64,
    pub experiment_duration: f64,
    pub sample_delay: f64,
    pub seed: String,
    pub network_latency_type: String,
    pub network_latency_config: HashMap<String, String>,
    pub network_drop_chance: f64,
    pub network_duplicate_chance: f64,
    pub description: String,
}

impl Stats {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new<A: ConfigurationExtension, C: ClientConfigExt>(
        description: String,
        seed: String,
        start_time: u64,
        end_time: u64,
        clients_stats: Vec<ClientStats>,
        replicas_stats: Vec<ReplicaStats>,
        config: Config<A, C>,
        algo_info: VersionInfo,
    ) -> Self {
        let Config {
            algo,
            n,
            t,
            f,
            omission_chance,
            client,
            experiment_duration,
            replicas: ReplicasConfig { sample_delay },
        } = config;

        let VersionInfo {
            abcperf_version,
            name: algo_name,
            version: algo_version,
        } = algo_info;

        Self {
            clients: clients_stats,
            replicas: replicas_stats,
            client_host_info: HostInfo::collect(),
            abcperf_version: abcperf_version.to_string(),
            algo_name: algo_name.to_string(),
            algo_version: algo_version.to_string(),
            start_time,
            end_time,
            algo_config: algo.into(),
            client_config: client.into(),
            n: n.into(),
            t,
            experiment_duration,
            sample_delay,
            seed,
            network_latency_type: String::new(),
            network_latency_config: HashMap::new(),
            network_drop_chance: 0f64,
            network_duplicate_chance: 0f64,
            description,
            f,
            omission_chance,
        }
    }
}
