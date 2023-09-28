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
use futures::{
    future::{self, AbortHandle, Abortable},
    stream, StreamExt,
};
use rand::SeedableRng;
use serde::{Deserialize, Serialize};
use shared_ids::{AnyId, ClientId, IdIter, ReplicaId};
use tokio::{sync::oneshot, task::JoinHandle, time::timeout};
use tracing::{error, warn, Instrument};

use abcperf::{
    config::{ClientConfig, ClientConfigExt},
    EndRng,
};

use crate::get_one_with_fallback;

use super::{Generate, CLIENT_STUCK_AFTER};

type StopAndJoin = (AbortHandle, JoinHandle<Vec<(Instant, Duration, ReplicaId)>>);

pub(super) struct NClientsClient<
    I: Generate<CC> + Serialize + for<'a> Deserialize<'a> + Debug + Send + Sync + 'static + Clone,
    O: Serialize + for<'a> Deserialize<'a> + Send + Sync + 'static,
    CC: ClientConfigExt,
    CS: CSTrait,
> {
    number: u64,
    replicas: Arc<[(ReplicaId, SocketAddr)]>,
    stop_and_joins: Vec<StopAndJoin>,
    config: ClientConfig<CC>,
    phantom_data: PhantomData<(I, O)>,
    cs: TypedCSTrait<CS, I, O>,
}

impl<
        I: Generate<CC> + Serialize + for<'a> Deserialize<'a> + Debug + Send + Sync + 'static + Clone,
        O: Serialize + for<'a> Deserialize<'a> + Send + Sync + 'static,
        CC: ClientConfigExt,
        CS: CSTrait,
    > NClientsClient<I, O, CC, CS>
{
    pub(super) fn new(
        number: u64,
        replicas: Arc<[(ReplicaId, SocketAddr)]>,
        config: ClientConfig<CC>,
        cs: TypedCSTrait<CS, I, O>,
    ) -> Self {
        Self {
            number,
            replicas,
            stop_and_joins: Vec::new(),
            config,
            phantom_data: PhantomData::default(),
            cs,
        }
    }

    async fn client_loop(
        mut stream: Abortable<stream::Iter<<I as Generate<CC>>::Generator>>,
        replicas: Arc<[(ReplicaId, SocketAddr)]>,
        mut rng: EndRng,
        fallback_timeout: Duration,
        client_id: ClientId,
        client: TypedCSTraitClient<<CS::Config as CSConfig>::Client, I, O>,
    ) -> Vec<(Instant, Duration, ReplicaId)> {
        let mut elapsed_vec = Vec::new();

        while let Some(request) = stream.next().await {
            let start = Instant::now();

            match timeout(
                CLIENT_STUCK_AFTER,
                get_one_with_fallback(&replicas, &mut rng, fallback_timeout, |addr| {
                    Self::send_request_connect(&client, addr, request.clone())
                }),
            )
            .await
            {
                Ok(Some(_)) => {}
                Ok(None) => {
                    error!("did not get any valid response");
                    break;
                }
                Err(_) => {
                    warn!("client {:?} got stuck, stopping", client_id);
                    break;
                }
            }

            let elapsed: Duration = start.elapsed();
            elapsed_vec.push((start, elapsed, ReplicaId::from_u64(u64::MAX))); // TODO remove ReplicaId
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
        stop: oneshot::Receiver<()>,
        mut seed_rng: EndRng,
        fallback_timeout: Duration,
    ) -> Result<Vec<ClientStats>> {
        let cs = self.cs.configure_debug();

        for client_id in IdIter::default().take(self.number.try_into().unwrap()) {
            let rng = EndRng::from_rng(&mut seed_rng).expect("seed rng should always work");
            let generator = I::generator(&self.config.client, client_id);
            let (stream, stop_handle) = stream::abortable(stream::iter(generator));
            let join_handle = tokio::spawn(
                Self::client_loop(
                    stream,
                    self.replicas.clone(),
                    rng,
                    fallback_timeout,
                    client_id,
                    cs.client(),
                )
                .in_current_span(),
            );
            self.stop_and_joins.push((stop_handle, join_handle));
        }

        stop.await.unwrap();

        for (stop, _) in &self.stop_and_joins {
            stop.abort();
        }

        let clients = future::join_all(self.stop_and_joins.into_iter().map(|(_, join)| join)).await;

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
