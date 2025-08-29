use std::{
    sync::Arc,
    time::{Duration, Instant},
};

use tokio::sync::{mpsc, oneshot, watch};
use tracing::{error, info, info_span, Instrument};

use crate::{
    application::ApplicationConfig,
    atomic_broadcast::ABCConfig,
    config::{
        update::{ConfigUpdate, LoadConfigUpdate, ReplicaConfigUpdate},
        Config,
    },
    replica::communication::StopHandle,
    MessageDestination,
};

pub(super) struct ConfigUpdater {
    updates: Vec<(Duration, ConfigUpdate)>,
    update_load: Arc<watch::Sender<LoadConfigUpdate>>,
    update_replica: mpsc::Sender<(MessageDestination, ReplicaConfigUpdate)>,
}

impl ConfigUpdater {
    pub(super) fn from_config<ABC: ABCConfig, APP: ApplicationConfig>(
        config: Config<ABC, APP>,
        update_load: Arc<watch::Sender<LoadConfigUpdate>>,
        update_replica: mpsc::Sender<(MessageDestination, ReplicaConfigUpdate)>,
    ) -> Self {
        Self {
            updates: config.get_ordered_config_updates(),
            update_load,
            update_replica,
        }
    }

    async fn run(self, mut stop: oneshot::Receiver<()>, start_time: Instant) {
        for (time, update) in self.updates {
            let sleep_until = start_time + time;
            if Instant::now() < sleep_until {
                tokio::select! {
                    () = tokio::time::sleep_until(sleep_until.into()) => {},
                    Ok(()) = &mut stop => {
                        return;
                    }
                }
            }
            match update {
                ConfigUpdate::Replica(d, u) => {
                    info!("{:.2}s\t {}", time.as_secs_f64(), u);
                    let _ = self.update_replica.send((d, u)).await;
                }
                ConfigUpdate::Load(u) => {
                    info!("{:.2}s\t {}", time.as_secs_f64(), u);
                    let _ = self.update_load.send(u);
                }
            }
        }
        info!("All config updates applied, no pending updates");
        if stop.await.is_err() {
            error!("Tearing down config updater, stop signal lost");
        }
    }

    pub(super) fn start(self, start_time: Instant) -> StopHandle<()> {
        let span = info_span!("config_update");
        StopHandle::spawn(|stop| self.run(stop, start_time).instrument(span))
    }
}
