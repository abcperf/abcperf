use std::{
    mem,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};

use crate::stats::{
    HostInfo, MessageCount, MessageCounts, ReplicaSample, ReplicaSampleStats, ReplicaStats, RxTx,
    Timestamp,
};
use crate::MessageType;
use procfs::process::Process;
use tokio::{select, task::JoinHandle};
use tokio::{sync::oneshot, time};
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
        self.rx.fetch_add(1, Ordering::Relaxed);
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

fn stats(
    vec_capacity: usize,
    message_counter: &MessageCounter,
) -> anyhow::Result<ReplicaSampleStats> {
    let process = Process::myself()?;

    Ok(ReplicaSampleStats {
        memory_usage_in_bytes: process
            .stat
            .rss_bytes()?
            .try_into()
            .expect("negative memory usage not possible"),
        user_mode_ticks: process.stat.utime,
        kernel_mode_ticks: process.stat.stime,
        memory_used_by_stats: (mem::size_of::<(Instant, ReplicaSampleStats)>() * vec_capacity)
            as u64,
        rx_bytes: 0,
        tx_bytes: 0,
        rx_datagrams: 0,
        tx_datagrams: 0,
        messages: message_counter.sample(),
    })
}

async fn run(
    period: Duration,
    message_counter: Arc<MessageCounter>,
    mut stop: oneshot::Receiver<Instant>,
) -> Result {
    let ticks_per_second = procfs::ticks_per_second()?
        .try_into()
        .expect("should be positive");

    let mut interval = time::interval(period);

    let mut samples = Vec::new();

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
                samples.push((instant, stats(capacity, &message_counter)?));
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
                timestamp: Timestamp::new(start_time, time.into())?,
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
