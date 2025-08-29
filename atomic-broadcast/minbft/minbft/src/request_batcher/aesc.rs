use std::{
    cmp::min,
    collections::{HashMap, VecDeque},
    num::NonZeroUsize,
    time::{Duration, Instant},
};

use rand::{thread_rng, Rng};
use shared_ids::ClientId;
use tracing::info;

use crate::{
    client_request::RequestBatch, timeout::Timeout, ClientRequest, RequestId, RequestPayload,
};

use super::{ControllerOutput, RequestBatcher};

#[derive(Clone, Copy, Debug)]
enum Direction {
    Increasing,
    Descreasing,
}

#[derive(Clone, Copy, Debug)]
#[allow(clippy::upper_case_acronyms)]
struct ESO {
    waiting_time: Duration,
    perf_index: Duration,
    probing_direction: Direction,
}

impl ESO {
    fn new() -> Self {
        Self {
            waiting_time: Duration::ZERO,
            perf_index: Duration::MAX,
            probing_direction: Direction::Increasing,
        }
    }

    fn update(&mut self, new_perf_index: Duration) -> Direction {
        //d = generateNewDitheringValue()

        //Keep direction if value improved else flip direction
        self.probing_direction = if self.perf_index >= new_perf_index {
            self.probing_direction
        } else {
            match self.probing_direction {
                Direction::Increasing => Direction::Descreasing,
                Direction::Descreasing => Direction::Increasing,
            }
        };

        //Range is completly undefined_probably less than gamma_0
        let pertupation = thread_rng().gen_range(-1i64..1i64);

        //Constant for now
        //In Milliseconds
        let gamma = 1;

        let delta: i64 = match self.probing_direction {
            Direction::Increasing => gamma,
            Direction::Descreasing => -gamma,
        } + pertupation;

        self.perf_index = new_perf_index;

        let delta_duration = Duration::from_millis(delta.unsigned_abs());
        if delta.is_positive() {
            self.waiting_time = self.waiting_time.saturating_add(delta_duration);
            Direction::Increasing
        } else {
            self.waiting_time = self.waiting_time.saturating_sub(delta_duration);
            Direction::Descreasing
        }
    }

    fn get_timeout(&self) -> Duration {
        self.waiting_time
    }

    fn set_timeout(&mut self, new_duration: Duration) {
        self.waiting_time = new_duration;
    }
}

type BatchId = (ClientId, RequestId);

#[derive(Clone, Debug)]
#[allow(clippy::upper_case_acronyms)]
pub struct AESC<P, Att> {
    requests: VecDeque<ClientRequest<P, Att>>,
    optimizers: Vec<ESO>,
    issue_events: HashMap<BatchId, Instant>,
    last_batch: Option<Instant>,
    max_inflight: NonZeroUsize,
}

impl<P: RequestPayload, Att> AESC<P, Att> {
    pub fn new(max_batch_size: NonZeroUsize, max_inflight: NonZeroUsize) -> Self {
        let optimizers = vec![ESO::new(); usize::from(max_batch_size)];

        Self {
            requests: VecDeque::new(),
            optimizers,
            issue_events: HashMap::new(),
            last_batch: None,
            max_inflight,
        }
    }

    fn get_next_batch(&mut self) -> ControllerOutput<P, Att> {
        if self.requests.is_empty() {
            return (None, None);
        }

        if self.issue_events.len() >= self.max_inflight.get() {
            return (None, None);
        }

        //Get the matching batch timeout or get the biggest one
        let eso = self
            .optimizers
            .get(self.requests.len() - 1)
            .or_else(|| self.optimizers.last())
            .expect("Max batch size cant be 0");

        let time_since_last = self
            .last_batch
            .as_ref()
            .map(|event| Instant::now().duration_since(*event))
            .unwrap_or(Duration::MAX);

        if time_since_last >= eso.get_timeout() {
            info!(
                "Sending {} after {:?} needed {:?}",
                self.requests.len(),
                time_since_last,
                eso.get_timeout()
            );

            let max_batch_size = self.optimizers.len();
            let batch: Vec<_> = self
                .requests
                .drain(0..min(max_batch_size, self.requests.len()))
                .collect();

            let first_req = batch.first().expect("Batches cant be empty");

            let now = Instant::now();
            self.last_batch = Some(now);

            let batch_id = (first_req.client, first_req.id());
            self.issue_events.insert(batch_id, now);

            (Some(RequestBatch::new(batch.into_boxed_slice())), None)
        } else {
            let next_timeout = eso.get_timeout().saturating_sub(time_since_last);
            (None, Some(Timeout::batch(next_timeout)))
        }
    }
}

impl<P: RequestPayload, Att: Send + Sync> RequestBatcher<P, Att> for AESC<P, Att> {
    fn batch(&mut self, new_request: ClientRequest<P, Att>) -> ControllerOutput<P, Att> {
        self.requests.push_back(new_request);
        self.get_next_batch()
    }

    fn timeout(&mut self) -> ControllerOutput<P, Att> {
        self.get_next_batch()
    }

    fn update(&mut self, batch: &RequestBatch<P, Att>) -> ControllerOutput<P, Att> {
        let batch_id = match batch.first() {
            Some(req) => (req.client, req.id()),
            None => return self.get_next_batch(),
        };

        let issue_event = match self.issue_events.remove(&batch_id) {
            Some(event) => event,
            None => return self.get_next_batch(),
        };

        let latency = Instant::now().duration_since(issue_event);
        let batch_size = batch.len();

        info!("{} {:?} {:?}", batch_size, latency, batch_id);

        let optimizer = self
            .optimizers
            .get_mut(batch_size - 1)
            .expect("No optimizer for batch size");
        let update_direction = optimizer.update(latency);
        let new_timeout = optimizer.get_timeout();

        //Enforce monotonicity
        match update_direction {
            Direction::Increasing => {
                for other_optimizer in &mut self.optimizers[batch_size - 1..] {
                    let timeout = other_optimizer.get_timeout();
                    if timeout < new_timeout {
                        other_optimizer.set_timeout(new_timeout);
                    }
                }
            }
            Direction::Descreasing => {
                for other_optimizer in &mut self.optimizers[..batch_size - 1] {
                    let timeout = other_optimizer.get_timeout();
                    if timeout > new_timeout {
                        other_optimizer.set_timeout(new_timeout);
                    }
                }
            }
        }

        self.get_next_batch()
    }
}
