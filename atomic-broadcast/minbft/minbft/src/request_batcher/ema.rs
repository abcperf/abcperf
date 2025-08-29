use std::{
    collections::HashMap,
    mem,
    time::{Duration, Instant},
};

use shared_ids::ClientId;

use crate::{
    client_request::RequestBatch, timeout::Timeout, ClientRequest, RequestId, RequestPayload,
};

use super::{ControllerOutput, RequestBatcher};

#[derive(Debug)]
struct TimeoutScheduler {
    timeout_started: Option<Instant>,
    timeout: Duration,
}

impl TimeoutScheduler {
    fn new() -> Self {
        Self {
            timeout_started: None,
            timeout: Duration::MAX,
        }
    }

    fn set_new_duration(&mut self, timeout: Duration) {
        self.timeout = timeout;
    }

    fn timeout(&mut self) -> Timeout {
        let now = Instant::now();
        let rem_time = if let Some(start_time) = self.timeout_started {
            let passed = now.duration_since(start_time);
            self.timeout.saturating_sub(passed)
        } else {
            self.timeout_started = Some(now);
            self.timeout
        };

        Timeout::batch(rem_time)
    }
}

#[derive(Clone, Debug)]
struct ExponentialMovingAverage {
    last_value: Option<f32>,
    decay: f32,
}

impl ExponentialMovingAverage {
    fn new(decay: f32) -> Self {
        Self {
            decay,
            last_value: None,
        }
    }

    fn update(&mut self, new_val: Duration) {
        self.last_value = Some(
            (1. - self.decay) * self.last_value.unwrap_or(new_val.as_secs_f32())
                + self.decay * new_val.as_secs_f32(),
        );
    }

    fn get(&self) -> Option<f32> {
        self.last_value
    }
}

#[derive(Debug)]
pub struct EMABatcher<P, Att> {
    next_batch: Vec<ClientRequest<P, Att>>,
    last_arrival: Instant,
    timeout_scheduler: TimeoutScheduler,
    max_batch_size: usize,
    start_times: HashMap<BatchID, Instant>,
    mean_time_to_ordering: ExponentialMovingAverage,
    mean_time_between_arrivals: ExponentialMovingAverage,
}

type BatchID = (ClientId, RequestId);

impl<P: RequestPayload, Att> EMABatcher<P, Att> {
    pub fn new(commit_decay: f32, arrival_decay: f32) -> Self {
        assert!(0. < commit_decay);
        assert!(commit_decay < 1.);

        Self {
            next_batch: Vec::new(),
            last_arrival: Instant::now(),
            timeout_scheduler: TimeoutScheduler::new(),
            max_batch_size: 1,
            start_times: HashMap::default(),
            mean_time_to_ordering: ExponentialMovingAverage::new(commit_decay),
            mean_time_between_arrivals: ExponentialMovingAverage::new(arrival_decay),
        }
    }

    fn get_next_batch(&mut self, timeout: bool) -> ControllerOutput<P, Att> {
        if self.next_batch.len() >= self.max_batch_size || (!self.next_batch.is_empty() && timeout)
        {
            let requests = mem::take(&mut self.next_batch).into_boxed_slice();
            let batch = RequestBatch::new(requests);
            let batch_id = batch_id(&batch).unwrap();
            self.start_times.insert(batch_id, Instant::now());
            (Some(batch), None)
        } else if !self.next_batch.is_empty() {
            (None, Some(self.timeout_scheduler.timeout()))
        } else {
            (None, None)
        }
    }

    fn adapt_batch_parameters(&mut self) {
        let time_between_arrivals = match self.mean_time_between_arrivals.get() {
            Some(time) => time,
            None => return,
        };

        let time_to_ordering = match self.mean_time_to_ordering.get() {
            Some(time) => time,
            None => return,
        };

        self.max_batch_size = (time_to_ordering / time_between_arrivals).ceil() as usize;

        if time_between_arrivals > time_to_ordering {
            self.max_batch_size = 1;
        } else {
            let duration = Duration::from_secs_f32(time_to_ordering);
            self.timeout_scheduler.set_new_duration(duration)
        }

        eprintln!(
            "Timeout MTA: {:.3} MTE: {:.3} BS: {} BT: {:.4}",
            time_between_arrivals,
            time_to_ordering,
            self.max_batch_size,
            self.timeout_scheduler.timeout.as_secs_f32()
        )
    }
}

fn batch_id<P: RequestPayload, Att>(batch: &RequestBatch<P, Att>) -> Option<BatchID> {
    Some((batch.first()?.client, batch.first()?.id()))
}

impl<P: RequestPayload, Att: Send + Sync> RequestBatcher<P, Att> for EMABatcher<P, Att> {
    fn batch(&mut self, request: ClientRequest<P, Att>) -> ControllerOutput<P, Att> {
        self.next_batch.push(request);
        let now = Instant::now();
        let duration = now.duration_since(self.last_arrival);
        self.last_arrival = now;

        self.mean_time_between_arrivals.update(duration);

        self.adapt_batch_parameters();
        self.get_next_batch(false)
    }

    fn timeout(&mut self) -> ControllerOutput<P, Att> {
        self.get_next_batch(true)
    }

    fn update(&mut self, commited_batch: &RequestBatch<P, Att>) -> ControllerOutput<P, Att> {
        let now = Instant::now();
        let batch_id = batch_id(commited_batch).unwrap();
        let start = self.start_times.remove(&batch_id).unwrap();
        let duration = now.duration_since(start);

        self.mean_time_to_ordering.update(duration);

        self.adapt_batch_parameters();
        self.get_next_batch(false)
    }
}
