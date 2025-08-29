use std::{
    mem,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};

use crate::stats::{
    HostInfo, MessageCount, MessageCounts, NetworkSample, ReplicaSample, ReplicaSampleStats,
    ReplicaStats, RxTx, Timestamp,
};
use crate::MessageType;
use procfs::{net::DeviceStatus, process::Process, WithCurrentSystemInfo};
use tokio::{select, sync::oneshot, task::JoinHandle, time};
use tracing::Instrument;

type Result = anyhow::Result<ReplicaStats>;

pub(super) struct StatsSampler {
    handle: JoinHandle<Result>,
    stop: oneshot::Sender<Instant>,
}

impl StatsSampler {
    pub(super) fn new(period: Duration, message_counter: Arc<MessageCounter>) -> Self {
        let (send, recv) = oneshot::channel();
        let handle = tokio::spawn(run(period, message_counter, recv).in_current_span());
        Self { handle, stop: send }
    }
    pub(super) async fn stop(self, start_time: Instant) -> Result {
        self.stop.send(start_time).unwrap();
        self.handle.await?
    }
}

pub(super) struct PerTypeMessageCounter {
    rx: AtomicUsize,
    tx: AtomicUsize,
}

impl PerTypeMessageCounter {
    fn new() -> Self {
        Self {
            rx: AtomicUsize::new(0),
            tx: AtomicUsize::new(0),
        }
    }

    pub(super) fn rx(&self) {
        self.rx.fetch_add(1, Ordering::Relaxed);
    }

    pub(super) fn tx(&self) {
        self.tx.fetch_add(1, Ordering::Relaxed);
    }

    fn sample(&self) -> RxTx {
        RxTx {
            rx: self.rx.load(Ordering::Relaxed) as u64,
            tx: self.tx.load(Ordering::Relaxed) as u64,
        }
    }
}

pub(super) struct PerTraitMessageCounter {
    unicast: PerTypeMessageCounter,
    broadcast: PerTypeMessageCounter,
}

impl PerTraitMessageCounter {
    fn new() -> Self {
        Self {
            unicast: PerTypeMessageCounter::new(),
            broadcast: PerTypeMessageCounter::new(),
        }
    }

    pub(super) fn message_type(
        &self,
        message_type: impl Into<MessageType>,
    ) -> &PerTypeMessageCounter {
        let message_type = message_type.into();
        match message_type {
            MessageType::Unicast => &self.unicast,
            MessageType::Broadcast => &self.broadcast,
        }
    }

    fn sample(&self) -> MessageCount {
        MessageCount {
            unicast: self.unicast.sample(),
            broadcast: self.broadcast.sample(),
        }
    }
}

pub(super) struct MessageCounter {
    algo: PerTraitMessageCounter,
    server: PerTraitMessageCounter,
}

impl MessageCounter {
    pub(super) fn new() -> Arc<Self> {
        Arc::new(Self {
            algo: PerTraitMessageCounter::new(),
            server: PerTraitMessageCounter::new(),
        })
    }

    pub(super) fn algo(&self) -> &PerTraitMessageCounter {
        &self.algo
    }

    pub(super) fn server(&self) -> &PerTraitMessageCounter {
        &self.server
    }

    fn sample(&self) -> MessageCounts {
        MessageCounts {
            algo: self.algo.sample(),
            server: self.server.sample(),
        }
    }
}

type SampleVecContent = (Instant, ReplicaSampleStats);

fn stats(
    vec_capacity: usize,
    message_counter: &MessageCounter,
) -> anyhow::Result<ReplicaSampleStats> {
    let process = Process::myself()?;
    let stat = process.stat().unwrap();
    Ok(ReplicaSampleStats {
        memory_usage_in_bytes: stat.rss_bytes().get(),
        user_mode_ticks: stat.utime,
        kernel_mode_ticks: stat.stime,
        memory_used_by_stats: (mem::size_of::<SampleVecContent>() * vec_capacity) as u64,
        network: process
            .dev_status()
            .unwrap()
            .into_iter()
            .filter(|(name, _)| name != "lo")
            .map(|(_, dev)| dev.into())
            .sum(),
        messages: message_counter.sample(),
    })
}

impl From<DeviceStatus> for NetworkSample {
    fn from(dev: DeviceStatus) -> Self {
        Self {
            bytes: RxTx {
                rx: dev.recv_bytes,
                tx: dev.sent_bytes,
            },
            packets: RxTx {
                rx: dev.recv_packets,
                tx: dev.sent_packets,
            },
        }
    }
}

async fn run(
    period: Duration,
    message_counter: Arc<MessageCounter>,
    mut stop: oneshot::Receiver<Instant>,
) -> Result {
    let ticks_per_second = procfs::ticks_per_second();
    let mut interval = time::interval(period);
    let mut samples = Vec::<SampleVecContent>::new();
    let start_time = loop {
        select! {
            instant = interval.tick() => {
                let capacity = samples.capacity();
                let capacity = if capacity == samples.len() {
                    samples.reserve(1);
                    samples.capacity()
                } else {
                    capacity
                };
                samples.push((instant.into(), stats(capacity, &message_counter)?));
            }
            start_time = &mut stop => {
                break start_time?;
            }
        }
    };
    let samples = samples
        .into_iter()
        .filter_map(|(time, stats)| {
            Some(ReplicaSample {
                timestamp: Timestamp::new(start_time, time)?,
                stats,
            })
        })
        .collect();
    Ok(ReplicaStats {
        samples,
        ticks_per_second,
        host_info: HostInfo::collect(),
    })
}
