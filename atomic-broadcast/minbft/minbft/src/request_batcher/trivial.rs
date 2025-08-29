use std::{mem, num::NonZeroUsize, time::Duration};

use crate::{client_request::RequestBatch, timeout::Timeout, ClientRequest, RequestPayload};

use super::{ControllerOutput, RequestBatcher};

#[derive(Debug)]
pub struct TrivialRequestBatcher<P, Att> {
    next_batch: Vec<ClientRequest<P, Att>>,
    timeout: Duration,
    max_size: Option<NonZeroUsize>,
    timeout_started: bool,
}

impl<P: RequestPayload, Att> TrivialRequestBatcher<P, Att> {
    pub fn new(timeout: Duration, max_size: Option<NonZeroUsize>) -> Self {
        Self {
            next_batch: Vec::new(),
            timeout,
            max_size,
            timeout_started: false,
        }
    }
}

impl<P: RequestPayload, Att: Send + Sync> RequestBatcher<P, Att> for TrivialRequestBatcher<P, Att> {
    fn batch(&mut self, request: ClientRequest<P, Att>) -> ControllerOutput<P, Att> {
        self.next_batch.push(request);

        match self.max_size {
            Some(max_size) if self.next_batch.len() >= max_size.get() => {
                self.timeout_started = false;
                let batch = mem::take(&mut self.next_batch);
                (Some(RequestBatch::new(batch.into_boxed_slice())), None)
            }
            _ => (
                None,
                if self.timeout_started {
                    None
                } else {
                    self.timeout_started = true;
                    Some(Timeout::batch(self.timeout))
                },
            ),
        }
    }

    fn timeout(&mut self) -> ControllerOutput<P, Att> {
        self.timeout_started = false;
        if self.next_batch.is_empty() {
            (None, None)
        } else {
            let batch = mem::take(&mut self.next_batch);
            (Some(RequestBatch::new(batch.into_boxed_slice())), None)
        }
    }

    fn update(&mut self, _commited_batch: &RequestBatch<P, Att>) -> ControllerOutput<P, Att> {
        (None, None)
    }
}
