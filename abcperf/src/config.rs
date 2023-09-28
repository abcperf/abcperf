use std::{collections::HashMap, fmt::Debug, num::NonZeroU64, time::Duration};

pub use crate::atomic_broadcast::AtomicBroadcastConfiguration;
use crate::atomic_broadcast::ConfigurationExtension;
use rand::{seq::IteratorRandom, Rng};
use serde::{Deserialize, Serialize};

use derivative::Derivative;
use shared_ids::{IdIter, ReplicaId};
use trait_alias_macro::pub_trait_alias_macro;

#[derive(Deserialize, Serialize, Derivative)]
#[serde(bound = "")]
#[derivative(Clone(bound = ""), Debug(bound = ""))]
pub(super) struct Config<A: ConfigurationExtension, C: ClientConfigExt> {
    pub(super) algo: A,
    pub(super) n: NonZeroU64,
    pub(super) t: u64,
    pub(super) f: u64,
    pub(super) omission_chance: f64,
    pub(super) client: C,
    pub(super) experiment_duration: f64,
    pub(super) replicas: ReplicasConfig,
}

#[derive(Deserialize, Serialize, Clone, Debug)]
pub(super) struct ReplicasConfig {
    //pub(super) network: Option<NetworkConfig>,
    pub(super) sample_delay: f64,
}

impl ReplicasConfig {
    pub(super) fn sample_delay(&self) -> Duration {
        Duration::from_secs_f64(self.sample_delay)
    }
}

impl<A: ConfigurationExtension, C: ClientConfigExt> Config<A, C> {
    pub(super) fn algo(&self, replica_id: ReplicaId) -> AtomicBroadcastConfiguration<A> {
        AtomicBroadcastConfiguration {
            extension: self.algo.clone(),
            n: self.n,
            t: self.t,
            replica_id,
        }
    }

    pub(super) fn client(&self) -> ClientConfig<C> {
        ClientConfig {
            client: self.client.clone(),
            t: self.t,
            n: self.n,
        }
    }

    pub(super) fn replicas(&self) -> impl Iterator<Item = ReplicaId> {
        IdIter::default().take(self.n.get().try_into().unwrap())
    }

    pub(super) fn gen_faulty(&self, rng: &mut impl Rng) -> Vec<ReplicaId> {
        let faulty = self.replicas().choose_multiple(rng, self.f as usize);

        assert_eq!(faulty.len(), self.f as usize);

        faulty
    }

    pub(super) fn experiment_duration(&self) -> Duration {
        Duration::from_secs_f64(self.experiment_duration)
    }
}

pub_trait_alias_macro!(ClientConfigExt = for<'a> Deserialize<'a> + Serialize + Send + Clone + Debug + 'static + Sync + Into<HashMap<String, String>>);

pub struct ClientConfig<C: ClientConfigExt> {
    pub client: C,
    pub t: u64,
    pub n: NonZeroU64,
}

#[derive(Deserialize, Serialize, Clone, Debug)]
pub struct NetworkConfig {
    pub latency: DistributionConfig,
    pub drop_chance: f64,
    pub duplicate_chance: f64,
}

impl From<NetworkConfig> for HashMap<String, String> {
    fn from(config: NetworkConfig) -> Self {
        let NetworkConfig {
            latency,
            drop_chance,
            duplicate_chance,
        } = config;
        let (name, mut map) = latency.to_output();
        assert!(map.insert("distribution_type".to_owned(), name).is_none());
        assert!(map
            .insert("drop_chance".to_owned(), drop_chance.to_string())
            .is_none());
        assert!(map
            .insert("duplicate_chance".to_owned(), duplicate_chance.to_string())
            .is_none());
        map
    }
}

#[derive(Deserialize, Serialize, Clone, Debug)]
pub enum DistributionConfig {
    #[serde(rename = "constant")]
    Constant { delay: f64 },

    #[serde(rename = "uniform")]
    Uniform { delay: f64 },

    #[serde(rename = "normal")]
    Normal { delay: f64, std_dev: f64 },

    #[serde(rename = "exponential")]
    Exponential { delay: f64 },
}

impl DistributionConfig {
    pub(super) fn to_output(&self) -> (String, HashMap<String, String>) {
        let mut map = HashMap::new();
        let name = match self {
            DistributionConfig::Constant { delay } => {
                map.insert("delay".to_owned(), delay.to_string());
                "constant"
            }
            DistributionConfig::Uniform { delay } => {
                map.insert("delay".to_owned(), delay.to_string());
                "uniform"
            }
            DistributionConfig::Normal { delay, std_dev } => {
                map.insert("delay".to_owned(), delay.to_string());
                map.insert("std_dev".to_owned(), std_dev.to_string());
                "normal"
            }
            DistributionConfig::Exponential { delay } => {
                map.insert("delay".to_owned(), delay.to_string());
                "exponential"
            }
        };
        (name.to_owned(), map)
    }
}
