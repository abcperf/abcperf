use std::{
    num::NonZeroU64,
    time::{Duration, Instant},
};

use rand::{Rng, SeedableRng};
use rand_distr::{Distribution, Exp, Normal, Uniform};
use tokio::{select, sync::mpsc, task::JoinHandle, time};
use tokio_util::sync::CancellationToken;
use tracing::{error, info, warn, Instrument};

use crate::{
    client::ClientWorkerId,
    config::{update::LoadConfigUpdate, DistributionConfig},
    EndRng, SeedRng,
};

pub type LiveConfig = LoadConfigUpdate;

#[derive(Debug)]
struct RepeatDist(f64);
impl Distribution<f64> for RepeatDist {
    fn sample<R: Rng + ?Sized>(&self, _rng: &mut R) -> f64 {
        self.0
    }
}

/// Starts the client generator using the provided [GeneratorConfig].
///
/// # Arguments
///
/// * `recv_stop` - The channel side that receives a stop signal
///            to stop the call.
/// * `config` - The [GeneratorConfig] to be used.
/// * `seed` - The [SeedRng] to be used with the underlying distribution.
/// * `recv_live_config` - The channel side that continuously receives live updates to configuration
pub fn start_generator(
    recv_stop: CancellationToken,
    maximum_client_count: NonZeroU64,
    distribution: DistributionConfig,
    seed: &mut SeedRng,
    recv_live_config: mpsc::Receiver<LiveConfig>,
    client_workers: NonZeroU64,
    worker_id: ClientWorkerId,
) -> (
    JoinHandle<()>,
    async_channel::Receiver<()>,
    mpsc::Receiver<()>,
) {
    let (new_request_send, new_request_recv) = async_channel::bounded(10_000);
    let (new_consumer_send, new_consumer_recv) = mpsc::channel(maximum_client_count.get() as usize);

    let rng = EndRng::from_rng(seed).expect("seed rng should always work");

    let join = match distribution {
        DistributionConfig::Constant {} => tokio::spawn(
            run_generator(
                maximum_client_count,
                rng,
                RepeatDist,
                recv_stop,
                new_request_send,
                new_consumer_send,
                recv_live_config,
                client_workers,
                worker_id,
            )
            .in_current_span(),
        ),
        DistributionConfig::Uniform {} => tokio::spawn(
            run_generator(
                maximum_client_count,
                rng,
                |delay| Uniform::new(0f64, 2f64 * delay),
                recv_stop,
                new_request_send,
                new_consumer_send,
                recv_live_config,
                client_workers,
                worker_id,
            )
            .in_current_span(),
        ),
        DistributionConfig::Normal { std_dev } => tokio::spawn(
            run_generator(
                maximum_client_count,
                rng,
                move |delay| Normal::new(delay, std_dev).unwrap(),
                recv_stop,
                new_request_send,
                new_consumer_send,
                recv_live_config,
                client_workers,
                worker_id,
            )
            .in_current_span(),
        ),
        DistributionConfig::Exponential {} => tokio::spawn(
            run_generator(
                maximum_client_count,
                rng,
                |delay| Exp::new(1f64 / delay).unwrap(),
                recv_stop,
                new_request_send,
                new_consumer_send,
                recv_live_config,
                client_workers,
                worker_id,
            )
            .in_current_span(),
        ),
    };

    (join, new_request_recv, new_consumer_recv)
}

/// Runs the generator by continously generating client requests and consumers.
///
/// # Arguments
///
/// * `config` - The underlying [InnerGeneratorConfig] of the [GeneratorConfig].
/// * `iter` - The iterator that contains the next amount of time to wait
///            until new requests should be generated.
/// * `recv_stop` - The channel side that receives a stop signal
///                 to stop the call.
/// * `sender_gen_new_req` - The async channel side to send a request to
///                          generate a new client request.
/// * `sender_gen_new_consumer` - The channel side to send a request to generate
///                               a new consumer.
/// * `recv_param_parallel_reqs` - The channel side that continuously receives
///                                live updates to the amount of parallel
///                                requests that should be applied to the
///                                generation of the clients.
#[allow(clippy::too_many_arguments)]
async fn run_generator<D: Distribution<f64>>(
    maximum_client_count: NonZeroU64,
    mut rng: EndRng,
    new_dist: impl Fn(f64) -> D,
    recv_stop: CancellationToken,
    sender_gen_new_req: async_channel::Sender<()>,
    new_consumer: mpsc::Sender<()>,
    mut recv_live_config: mpsc::Receiver<LiveConfig>,
    client_workers: NonZeroU64,
    worker_id: ClientWorkerId,
) {
    let multiplier = client_workers.get() as f64;
    let new_dist =
        move |delay| new_dist(delay * multiplier).map(|f| Duration::from_secs_f64(f.max(0f64)));

    let mut time = Instant::now();
    let mut last_log = Instant::now();
    let mut current_delay = 1.0;

    let mut parallel_req = 1;
    let mut iter = new_dist(current_delay);

    for _ in 0..maximum_client_count.get() {
        new_consumer.try_send(()).unwrap();
    }

    // If a stop signal is received in `recv_stop`, break out of loop.
    // Otherwise wait until the amount of time provided with the next item
    // of `iter`, and then generate both new client requests and consumers
    // (the latter only if needed).
    // Always check if an update to the amount of parallel requests
    // has been received (if so, apply it).
    loop {
        select! {
            biased;
            () = recv_stop.cancelled() => {
                info!("Stopped generation of new requests");
                return;
            }
            Some(config) = recv_live_config.recv() => {
                parallel_req = config.requests_per_tick.get();
                let delay = 1f64 / config.ticks_per_second.get() as f64;
                if current_delay != delay {
                    current_delay = delay;
                    iter = new_dist(current_delay);
                    time = Instant::now() +  Duration::from_secs_f64((worker_id.as_u64() + 1) as f64 * delay);
                }
            }
            () = time::sleep_until(time.into()) => {

                for _ in 0..parallel_req {
                    if sender_gen_new_req.is_full() {
                        let now = Instant::now();
                            if now.duration_since(last_log) > Duration::from_secs(5) {
                                error!("Client overload, dropping requests");
                                last_log = now;
                        }
                        break;
                    }
                    if sender_gen_new_req.try_send(()).is_err() {
                        warn!("Request consuming channel closed while generating requests");
                        return;
                    }
                }
                time += iter.sample(&mut rng);
            }
        }
    }
}
