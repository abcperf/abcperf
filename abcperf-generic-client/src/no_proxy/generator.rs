use std::{
    fmt::Debug,
    marker::PhantomData,
    net::SocketAddr,
    sync::Arc,
    time::{Duration, Instant},
};

use crate::cs::{
    typed::{TypedCSTrait, TypedCSTraitClient},
    CSConfig, CSTrait,
};
use abcperf::stats::{ClientSample, ClientStats, Timestamp};
use anyhow::Result;
use futures::future;
use rand::SeedableRng;
use serde::{Deserialize, Serialize};
use shared_ids::{AnyId, ClientId, IdIter, ReplicaId};
use tokio::{
    sync::{mpsc, oneshot},
    task::JoinHandle,
    time::timeout,
};
use tracing::{error, warn, Instrument};

use abcperf::{
    config::{ClientConfig, ClientConfigExt},
    generator::{start_generator, GeneratorConfig},
    EndRng, SeedRng,
};

use crate::get_t_plus_one;

use super::{Generate, CLIENT_STUCK_AFTER};

pub(super) struct GeneratorClient<
    I: Generate<CC> + Serialize + for<'a> Deserialize<'a> + Debug + Send + Sync + 'static,
    O: Serialize + for<'a> Deserialize<'a> + Send + Sync + 'static,
    CC: ClientConfigExt,
    CS: CSTrait,
> {
    replicas: Arc<[(ReplicaId, SocketAddr)]>,
    config: ClientConfig<CC>,
    phantom_data: PhantomData<(I, O)>,
    generator_join_handel: JoinHandle<()>,
    new_request: async_channel::Receiver<()>,
    new_consumer: mpsc::Receiver<()>,
    cs: TypedCSTrait<CS, I, O>,
}

impl<
        I: Generate<CC> + Serialize + for<'a> Deserialize<'a> + Debug + Send + Sync + 'static + Clone,
        O: Serialize + for<'a> Deserialize<'a> + Send + Sync + 'static,
        CC: ClientConfigExt,
        CS: CSTrait,
    > GeneratorClient<I, O, CC, CS>
{
    pub(super) fn new(
        replicas: Arc<[(ReplicaId, SocketAddr)]>,
        generator_config: GeneratorConfig,
        config: ClientConfig<CC>,
        seed: &mut SeedRng,
        stop: oneshot::Receiver<()>,
        cs: TypedCSTrait<CS, I, O>,
    ) -> Self {
        let (generator_join_handel, new_request, new_consumer) =
            start_generator(stop, generator_config, seed);
        Self {
            replicas,
            config,
            phantom_data: PhantomData::default(),
            generator_join_handel,
            new_request,
            new_consumer,
            cs,
        }
    }

    async fn client_loop(
        mut payload_generator: I::Generator,
        receiver: async_channel::Receiver<()>,
        replicas: Arc<[(ReplicaId, SocketAddr)]>,
        mut _rng: EndRng,
        client_id: ClientId,
        client: TypedCSTraitClient<<CS::Config as CSConfig>::Client, I, O>,
        t: u64,
    ) -> Vec<(Instant, Duration, ReplicaId)> {
        let mut elapsed_vec = Vec::new();

        while let Ok(()) = receiver.recv().await {
            let payload = payload_generator.next().unwrap();

            let start = Instant::now();

            match timeout(
                CLIENT_STUCK_AFTER,
                get_t_plus_one(&replicas, t, |addr| {
                    Self::send_request_connect(&client, addr, payload.clone())
                }),
            )
            .await
            {
                Ok(Some(_)) => {}
                Ok(None) => {
                    error!("did not get t + 1 valid responses");
                    break;
                }
                Err(_) => {
                    warn!("client {:?} got stuck, stopping", client_id);
                    break;
                }
            }

            let elapsed = start.elapsed();
            elapsed_vec.push((start, elapsed, ReplicaId::from_u64(u64::MAX)));
        }
        elapsed_vec
    }

    async fn send_request_connect(
        client: &TypedCSTraitClient<<CS::Config as CSConfig>::Client, I, O>,
        addr: SocketAddr,
        request: I,
    ) -> Result<(), anyhow::Error> {
        let mut connection = client.connect(addr, "localhost").await?;
        connection.request(request).await?;
        Ok(())
    }

    pub(super) async fn run(
        mut self,
        start_time: Instant,
        mut seed_rng: EndRng,
        t: u64,
    ) -> Result<Vec<ClientStats>> {
        let mut client_join_handles = Vec::new();

        let cs = self.cs.configure_debug();

        let mut id_gen = IdIter::default();
        while let Some(()) = self.new_consumer.recv().await {
            let id = id_gen.next().expect("we should not run out client ids");

            let payload_generator = I::generator(&self.config.client, id);

            let rng = EndRng::from_rng(&mut seed_rng).expect("seed rng should always work");

            let client = cs.client();

            client_join_handles.push(tokio::spawn(
                Self::client_loop(
                    payload_generator,
                    self.new_request.clone(),
                    Arc::clone(&self.replicas),
                    rng,
                    id,
                    client,
                    t,
                )
                .in_current_span(),
            ));
        }

        let clients = future::join_all(client_join_handles.into_iter()).await;

        self.generator_join_handel.await?;

        clients
            .into_iter()
            .map(|e| {
                Ok(ClientStats {
                    samples: e?
                        .into_iter()
                        .map(|(i, d, replica_id)| ClientSample {
                            timestamp: Timestamp::new(start_time, i).unwrap(),
                            processing_time: d.into(),
                            replica_id,
                        })
                        .collect(),
                })
            })
            .collect::<Result<Vec<_>>>()
    }
}
