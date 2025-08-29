use procfs::Current;
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, HashSet},
    env,
    ops::{AddAssign, Div, Sub},
    time::{Duration, Instant},
};
use sysinfo::{Networks, System};

use crate::{config::BasicConfig, VersionInfo, USER_VAR};

/// Models the metric microseconds.
#[derive(Deserialize, Serialize, Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
#[repr(transparent)]
#[serde(transparent)]
pub struct Micros(u64);

impl Micros {
    pub fn as_secs_f64(&self) -> f64 {
        self.0 as f64 / 1_000_000.0
    }
}

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

impl From<u64> for Micros {
    fn from(micros: u64) -> Self {
        Self(micros)
    }
}

impl Sub<Micros> for Micros {
    type Output = Micros;

    fn sub(self, rhs: Micros) -> Self::Output {
        Self(self.0 - rhs.0)
    }
}

impl AddAssign<Micros> for Micros {
    fn add_assign(&mut self, rhs: Micros) {
        self.0 += rhs.0;
    }
}

impl Div<u64> for Micros {
    type Output = Micros;

    fn div(self, rhs: u64) -> Self::Output {
        assert!(rhs > 0);
        Self(self.0 / rhs)
    }
}

/// Models a timestamp.
#[derive(Deserialize, Serialize, Clone, Copy, Debug, PartialEq, PartialOrd)]
#[repr(transparent)]
#[serde(transparent)]
pub struct Timestamp {
    /// The microseconds elapsed since a set start.
    micros_since_start: Micros,
}

impl Timestamp {
    /// Creates a new [Timestamp] from the provided start and using a provided
    /// time.
    ///
    /// # Arguments
    ///
    /// * `start` - The [Instant] that should be used as the start time.
    /// * `time` - The [Instant] with which the duration since `start` should
    ///            be calculated.
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

/// Models the statistics gathered from a client used in ABCperf.
#[derive(Deserialize, Serialize, Debug)]
pub struct ClientStats {
    /// The samples of each request.
    pub samples: Vec<ClientSample>,
}

/// Models a sample from a client request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClientSample {
    /// The timestamp of the sample that defines the time when a request
    /// was sent.
    pub timestamp: Timestamp,
    /// The amount of time that was required to process the client request.
    pub processing_time: Micros,
}

/// Models the information on the CPU of the host.
#[derive(Debug, Clone)]
pub struct CpuInfo {
    /// The common information on the CPU of a host.
    pub common: HashMap<String, String>,
    /// The amount of cores of the host's CPU.
    pub cores: Vec<HashMap<String, String>>,
}

/// Models the information on the kernel of the host.
#[derive(Debug, Clone)]
pub struct KernelInfo {
    /// The version of the kernel of the host.
    pub version: String,
    /// The flags of the kernel of the host.
    pub flags: HashSet<String>,
    /// The extra information on the kernel of the host.
    pub extra: String,
}

/// Models the information on the host.
#[derive(Deserialize, Serialize, Clone, Debug)]
#[serde(into = "FlatHostInfo", from = "FlatHostInfo")]
pub struct HostInfo {
    /// The information on the hostname.
    pub hostname: String,
    /// The information on the user.
    pub user: String,
    /// The information on the maximum amount of memory available.
    pub memory: u64,
    /// The information on the maximum amount of swap available.
    pub swap: u64,
    /// The information on the CPU of the host.
    pub cpu: CpuInfo,
    /// The information on the kernel of the host.
    pub kernel: KernelInfo,
}

impl From<FlatHostInfo> for HostInfo {
    fn from(value: FlatHostInfo) -> Self {
        let FlatHostInfo {
            hostname,
            user,
            memory,
            swap,
            cpu_common,
            cpu_cores,
            kernel_version,
            kernel_flags,
            kernel_extra,
        } = value;

        Self {
            hostname,
            user,
            memory,
            swap,
            cpu: CpuInfo {
                common: cpu_common,
                cores: cpu_cores,
            },
            kernel: KernelInfo {
                version: kernel_version,
                flags: kernel_flags,
                extra: kernel_extra,
            },
        }
    }
}

impl From<HostInfo> for FlatHostInfo {
    fn from(value: HostInfo) -> Self {
        let HostInfo {
            hostname,
            user,
            memory,
            swap,
            cpu:
                CpuInfo {
                    common: cpu_common,
                    cores: cpu_cores,
                },
            kernel:
                KernelInfo {
                    version: kernel_version,
                    flags: kernel_flags,
                    extra: kernel_extra,
                },
        } = value;

        Self {
            hostname,
            user,
            memory,
            swap,
            cpu_common,
            cpu_cores,
            kernel_version,
            kernel_flags,
            kernel_extra,
        }
    }
}

#[derive(Deserialize, Serialize, Debug)]
struct FlatHostInfo {
    /// The information on the hostname.
    pub hostname: String,
    /// The information on the user.
    pub user: String,
    /// The information on the maximum amount of memory available.
    pub memory: u64,
    /// The information on the maximum amount of swap available.
    pub swap: u64,
    /// The common information on the CPU of a host.
    pub cpu_common: HashMap<String, String>,
    /// The amount of cores of the host's CPU.
    pub cpu_cores: Vec<HashMap<String, String>>,
    /// The version of the kernel of the host.
    pub kernel_version: String,
    /// The flags of the kernel of the host.
    pub kernel_flags: HashSet<String>,
    /// The extra information on the kernel of the host.
    pub kernel_extra: String,
}

impl HostInfo {
    /// Collects the information on the host.
    /// Creates a [HostInfo] with the collected information.
    pub(crate) fn collect() -> Self {
        let cpu_info = procfs::CpuInfo::current().expect("should be available");
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
        let mem_info = procfs::Meminfo::current().expect("should be available");
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

/// Models the statistics gathered from a replica used in ABCperf.
#[derive(Deserialize, Serialize, Debug)]
pub struct ReplicaStats {
    /// The samples regarding the receival and processing of a request.
    pub samples: Vec<ReplicaSample>,
    /// The amount of ticks per second.
    pub ticks_per_second: u64,
    /// The information on the host.
    pub host_info: HostInfo,
}

pub struct AlgoStats {
    pub(crate) round_transitions: Vec<Timestamp>,
}

impl AlgoStats {
    pub(crate) fn new(data: Vec<Instant>, start: Instant) -> Self {
        let round_transitions = data
            .into_iter()
            .filter_map(|time| Timestamp::new(start, time))
            .collect();
        Self { round_transitions }
    }
}

/// Models the statistics gathered live from a replica to send to the
/// Orchestrator.
#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct ReplicaLiveStats {
    /// The global CPU usage of the replica.
    pub global_cpu_usage: f32,
    /// The memory usage of the replica (in gigabytes).
    pub mem_usage_gb: f64,
    /// The total amount of data received over all network interfaces except
    /// localhost (in megabytes).
    pub bandwidth_rx_mb: f64,
    /// The total amount of data transmitted over all network interfaces except
    /// localhost (in megabytes).
    pub bandwidth_tx_mb: f64,
}

impl ReplicaLiveStats {
    pub fn new(sys: &mut System, net: &mut Networks) -> Self {
        // Refresh system information.
        sys.refresh_cpu_usage();
        sys.refresh_memory();
        net.refresh();

        // Retrieve information on global CPU and memory usage.
        let global_cpu_usage = sys.global_cpu_usage();
        let mem_usage_gb = sys.used_memory() as f64 / 1_000_000_000.0;

        // Retrieve information on bandwidth.
        // Panic if replica does not have interface named lo.
        let mut bandwidth_rx_mb = 0.0;
        let mut bandwidth_tx_mb = 0.0;
        let mut has_loopback = false;
        for (interface_name, data) in net.list() {
            if interface_name == "lo" {
                has_loopback = true;
                continue;
            }
            bandwidth_rx_mb += data.received() as f64 * 8.0 / 1_000_000.0;
            bandwidth_tx_mb += data.transmitted() as f64 * 8.0 / 1_000_000.0;
        }
        if !has_loopback {
            panic!("Replica did not have an interface named lo (loopback).");
        }

        Self {
            global_cpu_usage,
            mem_usage_gb,
            bandwidth_rx_mb,
            bandwidth_tx_mb,
        }
    }
}

/// Models a sample from a replica regarding a client request.
#[derive(Deserialize, Serialize, Debug, Clone)]
#[serde(into = "FlatReplicaSample", from = "FlatReplicaSample")]
pub struct ReplicaSample {
    /// The timestamp of the sample that defines the time when a request
    /// was received.
    pub timestamp: Timestamp,
    /// The statistics gathered from the replica.
    pub stats: ReplicaSampleStats,
}

impl From<ReplicaSample> for FlatReplicaSample {
    fn from(value: ReplicaSample) -> Self {
        let ReplicaSample {
            timestamp,
            stats:
                ReplicaSampleStats {
                    memory_usage_in_bytes,
                    memory_used_by_stats,
                    user_mode_ticks,
                    kernel_mode_ticks,
                    network:
                        NetworkSample {
                            bytes:
                                RxTx {
                                    rx: bytes_rx,
                                    tx: bytes_tx,
                                },
                            packets:
                                RxTx {
                                    rx: packets_rx,
                                    tx: packets_tx,
                                },
                        },
                    messages:
                        MessageCounts {
                            algo:
                                MessageCount {
                                    unicast:
                                        RxTx {
                                            rx: messages_algo_unicast_rx,
                                            tx: messages_algo_unicast_tx,
                                        },
                                    broadcast:
                                        RxTx {
                                            rx: messages_algo_broadcast_rx,
                                            tx: messages_algo_broadcast_tx,
                                        },
                                },
                            server:
                                MessageCount {
                                    unicast:
                                        RxTx {
                                            rx: messages_server_unicast_rx,
                                            tx: messages_server_unicast_tx,
                                        },
                                    broadcast:
                                        RxTx {
                                            rx: messages_server_broadcast_rx,
                                            tx: messages_server_broadcast_tx,
                                        },
                                },
                        },
                },
        } = value;

        Self {
            timestamp,
            memory_usage_in_bytes,
            memory_used_by_stats,
            user_mode_ticks,
            kernel_mode_ticks,
            bytes_rx,
            bytes_tx,
            packets_rx,
            packets_tx,
            messages_algo_unicast_rx,
            messages_algo_unicast_tx,
            messages_algo_broadcast_rx,
            messages_algo_broadcast_tx,
            messages_server_unicast_rx,
            messages_server_unicast_tx,
            messages_server_broadcast_rx,
            messages_server_broadcast_tx,
        }
    }
}

impl From<FlatReplicaSample> for ReplicaSample {
    fn from(value: FlatReplicaSample) -> Self {
        let FlatReplicaSample {
            timestamp,
            memory_usage_in_bytes,
            memory_used_by_stats,
            user_mode_ticks,
            kernel_mode_ticks,
            bytes_rx,
            bytes_tx,
            packets_rx,
            packets_tx,
            messages_algo_unicast_rx,
            messages_algo_unicast_tx,
            messages_algo_broadcast_rx,
            messages_algo_broadcast_tx,
            messages_server_unicast_rx,
            messages_server_unicast_tx,
            messages_server_broadcast_rx,
            messages_server_broadcast_tx,
        } = value;
        Self {
            timestamp,
            stats: ReplicaSampleStats {
                memory_usage_in_bytes,
                memory_used_by_stats,
                user_mode_ticks,
                kernel_mode_ticks,
                network: NetworkSample {
                    bytes: RxTx {
                        rx: bytes_rx,
                        tx: bytes_tx,
                    },
                    packets: RxTx {
                        rx: packets_rx,
                        tx: packets_tx,
                    },
                },
                messages: MessageCounts {
                    algo: MessageCount {
                        unicast: RxTx {
                            rx: messages_algo_unicast_rx,
                            tx: messages_algo_unicast_tx,
                        },
                        broadcast: RxTx {
                            rx: messages_algo_broadcast_rx,
                            tx: messages_algo_broadcast_tx,
                        },
                    },
                    server: MessageCount {
                        unicast: RxTx {
                            rx: messages_server_unicast_rx,
                            tx: messages_server_unicast_tx,
                        },
                        broadcast: RxTx {
                            rx: messages_server_broadcast_rx,
                            tx: messages_server_broadcast_tx,
                        },
                    },
                },
            },
        }
    }
}

#[derive(Deserialize, Serialize, Debug)]
struct FlatReplicaSample {
    /// The timestamp of the sample that defines the time when a request
    /// was received.
    pub timestamp: Timestamp,
    /// The memory usage in bytes of the replica regarding the receival and
    /// processing of a client request.
    pub memory_usage_in_bytes: u64,
    /// The memory usage in bytes of the replica solely regarding the samples
    /// saved, i.e. all [ReplicaSampleStats].
    pub memory_used_by_stats: u64,
    /// The total amount of ticks in user mode.
    pub user_mode_ticks: u64,
    /// The total amount of ticks in kernel mode.
    pub kernel_mode_ticks: u64,

    pub bytes_rx: u64,
    pub bytes_tx: u64,
    pub packets_rx: u64,
    pub packets_tx: u64,

    pub messages_algo_unicast_rx: u64,
    pub messages_algo_unicast_tx: u64,

    pub messages_algo_broadcast_rx: u64,
    pub messages_algo_broadcast_tx: u64,

    pub messages_server_unicast_rx: u64,
    pub messages_server_unicast_tx: u64,

    pub messages_server_broadcast_rx: u64,
    pub messages_server_broadcast_tx: u64,
}

/// Models the statistics that may be gathered from a replica regarding
/// the receival and processing of a client request.
#[derive(Debug, Clone)]
pub struct ReplicaSampleStats {
    /// The memory usage in bytes of the replica regarding the receival and
    /// processing of a client request.
    pub memory_usage_in_bytes: u64,
    /// The memory usage in bytes of the replica solely regarding the samples
    /// saved, i.e. all [ReplicaSampleStats].
    pub memory_used_by_stats: u64,
    /// The total amount of ticks in user mode.
    pub user_mode_ticks: u64,
    /// The total amount of ticks in kernel mode.
    pub kernel_mode_ticks: u64,

    pub network: NetworkSample,

    /// The amount of messages received and transmitted.
    pub messages: MessageCounts,
}

#[derive(Debug, Clone, Default, derive_more::Sum, derive_more::Add)]
pub struct NetworkSample {
    pub bytes: RxTx,
    pub packets: RxTx,
}

/// Models the counter of messages received and transmitted by the replica
/// distinguished by the origin of the messages.
#[derive(Debug, Clone)]
pub struct MessageCounts {
    /// The counter of messages regarding the algorithm.
    pub algo: MessageCount,
    /// The counter of messages regarding the application.
    pub server: MessageCount,
}
/// Models the counter of messages received and transmitted by the replica
/// distinguished by the type of message.
#[derive(Debug, Clone)]
pub struct MessageCount {
    /// The counter of received and transmitted messages of type unicast.
    pub unicast: RxTx,
    /// The counter of received and transmitted messages of type broadcast.
    pub broadcast: RxTx,
}

/// Models the counter of messages/bytes received and transmitted by the replica.
#[derive(Debug, Clone, Default, derive_more::Sum, derive_more::Add)]
pub struct RxTx {
    /// The amount of messages/bytes received by the replica.
    pub rx: u64,
    /// The amount of messages/bytes transmitted by the replica.
    pub tx: u64,
}

impl ReplicaSample {
    /// Calculates the total amount of ticks, i.e., the amount of ticks
    /// in user mode and in kernel mode.
    pub fn ticks(&self) -> u64 {
        self.stats.user_mode_ticks + self.stats.kernel_mode_ticks
    }
}

#[derive(Debug, Serialize)]
pub struct RunInfo {
    pub(crate) seed: String,

    #[serde(flatten)]
    pub(crate) version_info: VersionInfo,

    pub(crate) raw_config: String,

    #[serde(flatten)]
    pub(crate) time_info: TimeInfo,

    #[serde(flatten)]
    pub(crate) basic_config: BasicConfig,

    pub(crate) abc_algorithm: String,
    pub(crate) state_machine_application: String,
}

#[derive(Debug, Serialize)]
pub struct TimeInfo {
    pub start_time: u64,
    pub end_time: u64,
    pub experiment_duration: f64,
}
