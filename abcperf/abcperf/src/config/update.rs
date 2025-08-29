use std::{fmt::Display, num::NonZeroU64, time::Duration};

use serde::{Deserialize, Serialize};
use shared_ids::ReplicaId;

use crate::{application::ApplicationConfig, atomic_broadcast::ABCConfig, MessageDestination};

use super::Config;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct Invocateable<C> {
    pub(crate) invocation_time: f64,
    #[serde(flatten)]
    pub(crate) config: C,
}

pub(crate) enum ConfigUpdate {
    Replica(MessageDestination, ReplicaConfigUpdate),
    Load(LoadConfigUpdate),
}

#[derive(Debug, Serialize, Deserialize)]
pub(crate) enum ReplicaConfigUpdate {
    R2RNetwork(R2RNetworkConfigUpdate),
    C2RNetwork(C2RNetworkConfigUpdate),
    FaultEmulation(FaultEmulationConfigUpdate),
}

impl Display for ReplicaConfigUpdate {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::R2RNetwork(R2RNetworkConfigUpdate(NetworkConfigUpdate {
                latency,
                jitter,
                packet_loss,
            })) => f.write_fmt(format_args!(
                "Network Replicas: {latency}ms ± {jitter}ms, loss {packet_loss:.3}%"
            )),
            Self::C2RNetwork(C2RNetworkConfigUpdate(NetworkConfigUpdate {
                latency,
                jitter,
                packet_loss,
            })) => f.write_fmt(format_args!(
                "Network Clients: {latency}ms ± {jitter}ms, loss {packet_loss:.3}%"
            )),
            Self::FaultEmulation(FaultEmulationConfigUpdate {
                replica_id,
                omission_chance,
            }) => {
                let drop = if *omission_chance == 0.0 {
                    "is correct".to_string()
                } else {
                    let print = omission_chance * 100.0;
                    format!("drops {print}%")
                };
                f.write_fmt(format_args!(
                    "Fault: Replica {} {}",
                    replica_id.as_u64(),
                    drop
                ))
            }
        }
    }
}

pub(crate) trait AsUpdate {
    fn as_update(&self) -> ConfigUpdate;
}

impl<C: Clone + AsUpdate> Invocateable<C> {
    fn duration_tuple(&self) -> (Duration, ConfigUpdate) {
        (
            Duration::from_secs_f64(self.invocation_time),
            self.config.as_update(),
        )
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[repr(transparent)]
#[serde(transparent)]
pub(crate) struct R2RNetworkConfigUpdate(pub NetworkConfigUpdate);

impl AsUpdate for R2RNetworkConfigUpdate {
    fn as_update(&self) -> ConfigUpdate {
        ConfigUpdate::Replica(
            MessageDestination::Broadcast,
            ReplicaConfigUpdate::R2RNetwork(self.clone()),
        )
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[repr(transparent)]
#[serde(transparent)]
pub(crate) struct C2RNetworkConfigUpdate(pub NetworkConfigUpdate);

impl AsUpdate for C2RNetworkConfigUpdate {
    fn as_update(&self) -> ConfigUpdate {
        ConfigUpdate::Replica(
            MessageDestination::Broadcast,
            ReplicaConfigUpdate::C2RNetwork(self.clone()),
        )
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct NetworkConfigUpdate {
    pub(crate) latency: u16,
    pub(crate) jitter: u16,
    pub(crate) packet_loss: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct FaultEmulationConfigUpdate {
    pub(crate) replica_id: ReplicaId,
    pub(crate) omission_chance: f64,
}

impl AsUpdate for FaultEmulationConfigUpdate {
    fn as_update(&self) -> ConfigUpdate {
        ConfigUpdate::Replica(
            MessageDestination::Unicast(self.replica_id),
            ReplicaConfigUpdate::FaultEmulation(self.clone()),
        )
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LoadConfigUpdate {
    pub ticks_per_second: NonZeroU64,
    pub requests_per_tick: NonZeroU64,
}

impl AsUpdate for LoadConfigUpdate {
    fn as_update(&self) -> ConfigUpdate {
        ConfigUpdate::Load(self.clone())
    }
}

impl Display for LoadConfigUpdate {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_fmt(format_args!(
            "Load: {} req/s ({} ticks/s, {} req/tick)",
            self.ticks_per_second.get() * self.requests_per_tick.get(),
            self.ticks_per_second,
            self.requests_per_tick
        ))
    }
}

impl<ABC: ABCConfig, APP: ApplicationConfig> Config<ABC, APP> {
    pub(crate) fn get_ordered_config_updates(&self) -> Vec<(Duration, ConfigUpdate)> {
        let mut updates: Vec<(Duration, ConfigUpdate)> = self
            .fault_emulation
            .iter()
            .map(Invocateable::duration_tuple)
            .chain(
                self.orchestrator_request_generation
                    .load
                    .iter()
                    .map(Invocateable::duration_tuple),
            )
            .chain(
                self.network
                    .client_to_replica
                    .iter()
                    .map(Invocateable::duration_tuple),
            )
            .chain(
                self.network
                    .replica_to_replica
                    .iter()
                    .map(Invocateable::duration_tuple),
            )
            .collect();

        updates.sort_by_key(|(d, _)| *d);

        updates
    }
}
