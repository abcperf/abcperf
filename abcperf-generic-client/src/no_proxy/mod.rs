use std::{collections::HashMap, fmt::Debug, marker::PhantomData, net::SocketAddr, time::Instant};

use crate::cs::{typed::TypedCSTrait, CSTrait};
use abcperf::stats::ClientStats;
use anyhow::Result;
use rand::SeedableRng;
use serde::{Deserialize, Serialize};
use shared_ids::ReplicaId;
use std::time::Duration;
use tokio::{sync::oneshot, task::JoinHandle};
use tracing::Instrument;

use abcperf::{config::ClientConfig, ClientEmulator, EndRng, SeedRng};

use crate::{CustomClientConfig, Generate};

use self::{generator::GeneratorClient, n_clients::NClientsClient};

mod generator;
mod n_clients;

const CLIENT_STUCK_AFTER: Duration = Duration::from_secs(10); // TODO maybe config option

pub struct GenericClient<
    I: Generate<C> + Serialize + for<'a> Deserialize<'a> + Debug + Send + Sync + 'static,
    O: Serialize + for<'a> Deserialize<'a> + Send + Sync + 'static,
    C: for<'a> Deserialize<'a>
        + Serialize
        + Send
        + Clone
        + Debug
        + 'static
        + Sync
        + Into<HashMap<String, String>>
        + AsRef<CustomClientConfig>,
    CS: CSTrait,
> {
    phantom_data: PhantomData<(I, O, C)>,
    cs: TypedCSTrait<CS, I, O>,
}

impl<
        I: Generate<C> + Serialize + for<'a> Deserialize<'a> + Debug + Send + Sync + 'static,
        O: Serialize + for<'a> Deserialize<'a> + Send + Sync + 'static,
        C: for<'a> Deserialize<'a>
            + Serialize
            + Send
            + Clone
            + Debug
            + 'static
            + Sync
            + Into<HashMap<String, String>>
            + AsRef<CustomClientConfig>,
        CS: CSTrait,
    > GenericClient<I, O, C, CS>
{
    pub fn new(cs: TypedCSTrait<CS, I, O>) -> Self {
        Self {
            phantom_data: Default::default(),
            cs,
        }
    }
}

impl<
        I: Generate<C> + Serialize + for<'a> Deserialize<'a> + Debug + Send + Sync + 'static + Clone,
        O: Serialize + for<'a> Deserialize<'a> + Send + Sync + 'static,
        C: for<'a> Deserialize<'a>
            + Serialize
            + Send
            + Clone
            + Debug
            + 'static
            + Sync
            + Into<HashMap<String, String>>
            + AsRef<CustomClientConfig>,
        CS: CSTrait,
    > ClientEmulator for GenericClient<I, O, C, CS>
{
    type Config = C;

    fn start(
        self,
        config: ClientConfig<Self::Config>,
        replicas: Vec<(ReplicaId, SocketAddr)>,
        seed: &mut SeedRng,
        start_time: Instant,
    ) -> (oneshot::Sender<()>, JoinHandle<Result<Vec<ClientStats>>>) {
        let (sender, receiver) = oneshot::channel();

        let t = config.t;

        let replicas = replicas.into_iter().collect();

        let join = match config.client.as_ref() {
            CustomClientConfig::NClients { number } => {
                let n_clients =
                    NClientsClient::<I, O, C, CS>::new(*number, replicas, config, self.cs);
                tokio::spawn(
                    n_clients
                        .run(
                            start_time,
                            receiver,
                            EndRng::from_rng(seed).expect("seed rng should always work"),
                            t,
                        )
                        .in_current_span(),
                )
            }
            CustomClientConfig::Distribution(generator_config) => {
                let n_clients = GeneratorClient::<I, O, C, CS>::new(
                    replicas,
                    generator_config.clone(),
                    config,
                    seed,
                    receiver,
                    self.cs,
                );
                tokio::spawn(
                    n_clients
                        .run(
                            start_time,
                            EndRng::from_rng(seed).expect("seed rng should always work"),
                            t,
                        )
                        .in_current_span(),
                )
            }
        };
        (sender, join)
    }
}
