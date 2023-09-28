use std::{
    cmp,
    collections::HashMap,
    iter,
    num::NonZeroUsize,
    time::{Duration, Instant},
};

use futures::Future;
use rand::SeedableRng;
use rand_distr::{Distribution, Exp, Normal, Uniform};
use serde::{Deserialize, Serialize};
use tokio::{
    select,
    sync::{mpsc, oneshot},
    task::JoinHandle,
    time,
};
use tracing::Instrument;

use crate::{config::DistributionConfig, EndRng, SeedRng};

const PARALLEL_CLIENT_CHANNEL_LOWER_BOUND: usize = 10;

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct GeneratorConfig {
    #[serde(flatten)]
    distribution: DistributionConfig,

    #[serde(flatten)]
    inner: InnerGeneratorConfig,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
struct InnerGeneratorConfig {
    maximum_client_count: NonZeroUsize,
    parallel_requests: NonZeroUsize,
}

impl From<GeneratorConfig> for HashMap<String, String> {
    fn from(config: GeneratorConfig) -> Self {
        let GeneratorConfig {
            distribution,
            inner:
                InnerGeneratorConfig {
                    maximum_client_count,
                    parallel_requests,
                },
        } = config;
        let (name, mut map) = distribution.to_output();
        assert!(map
            .insert(
                "maximum_client_count".to_owned(),
                maximum_client_count.to_string(),
            )
            .is_none());
        assert!(map
            .insert(
                "parallel_requests".to_owned(),
                parallel_requests.to_string(),
            )
            .is_none());
        assert!(map.insert("distribution_type".to_owned(), name).is_none());
        map
    }
}

pub fn start_generator(
    stop: oneshot::Receiver<()>,
    config: GeneratorConfig,
    seed: &mut SeedRng,
) -> (
    JoinHandle<()>,
    async_channel::Receiver<()>,
    mpsc::Receiver<()>,
) {
    let (new_request_send, new_request_recv) =
        async_channel::bounded(PARALLEL_CLIENT_CHANNEL_LOWER_BOUND);
    let (new_consumer_send, new_consumer_recv) =
        mpsc::channel(config.inner.maximum_client_count.get());

    if config.inner.parallel_requests > config.inner.maximum_client_count {
        panic!("Parallel requests must not be higher than maximum client count!");
    }

    let join = match config.distribution {
        DistributionConfig::Constant { delay } => {
            let duration = Duration::from_secs_f64(delay);
            let iter = iter::repeat(duration);
            tokio::spawn(
                run_generator(
                    config.inner,
                    iter,
                    stop,
                    new_request_send,
                    new_consumer_send,
                )
                .in_current_span(),
            )
        }
        DistributionConfig::Uniform { delay } => tokio::spawn(
            run_generator_with_dist(
                config.inner,
                Uniform::new(0f64, 2f64 * delay),
                seed,
                stop,
                new_request_send,
                new_consumer_send,
            )
            .in_current_span(),
        ),
        DistributionConfig::Normal { delay, std_dev } => tokio::spawn(
            run_generator_with_dist(
                config.inner,
                Normal::new(delay, std_dev).unwrap(),
                seed,
                stop,
                new_request_send,
                new_consumer_send,
            )
            .in_current_span(),
        ),
        DistributionConfig::Exponential { delay } => tokio::spawn(
            run_generator_with_dist(
                config.inner,
                Exp::new(1f64 / delay).unwrap(),
                seed,
                stop,
                new_request_send,
                new_consumer_send,
            )
            .in_current_span(),
        ),
    };

    (join, new_request_recv, new_consumer_recv)
}

fn run_generator_with_dist(
    config: InnerGeneratorConfig,
    dist: impl Distribution<f64>,
    seed: &mut SeedRng,
    stop_handle: oneshot::Receiver<()>,
    new_request: async_channel::Sender<()>,
    new_consumer: mpsc::Sender<()>,
) -> impl Future<Output = ()> {
    let rng = EndRng::from_rng(seed).expect("seed rng should always work");
    let iter = dist
        .sample_iter(rng)
        .map(clamp_min)
        .map(Duration::from_secs_f64);

    run_generator(config, iter, stop_handle, new_request, new_consumer)
}

fn clamp_min(number: f64) -> f64 {
    number.max(0f64)
}

async fn run_generator(
    config: InnerGeneratorConfig,
    mut iter: impl Iterator<Item = Duration>,
    mut stop_handle: oneshot::Receiver<()>,
    new_request: async_channel::Sender<()>,
    new_consumer: mpsc::Sender<()>,
) {
    let mut consumer_count = 0;

    for _ in 0..cmp::max(
        config.parallel_requests.get(),
        PARALLEL_CLIENT_CHANNEL_LOWER_BOUND,
    ) {
        new_consumer.send(()).await.unwrap();
        consumer_count += 1;
    }

    let mut time = Instant::now();

    loop {
        select! {
            biased;
            result = &mut stop_handle => {
                break result;
            }
            () = time::sleep_until(time.into()) => {
                for _ in 0..config.parallel_requests.get() {
                    if new_request.is_full() && consumer_count < config.maximum_client_count.get() {

                        new_consumer.send(()).await.unwrap();
                        consumer_count += 1;
                    }

                    new_request.send(()).await.unwrap();
                }

                time += iter.next().unwrap();
            }
        }
    }
    .unwrap();
}
