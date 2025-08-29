mod aesc;
mod ema;
mod trivial;

pub(super) use aesc::AESC;
pub(super) use ema::EMABatcher;
pub(super) use trivial::TrivialRequestBatcher;

use crate::{client_request::RequestBatch, timeout::Timeout, ClientRequest};

type ControllerOutput<P, Att> = (Option<RequestBatch<P, Att>>, Option<Timeout>);

pub(super) trait RequestBatcher<P, Att>: Send + Sync {
    fn batch(&mut self, request: ClientRequest<P, Att>) -> ControllerOutput<P, Att>;
    fn timeout(&mut self) -> ControllerOutput<P, Att>;
    fn update(&mut self, commited_batch: &RequestBatch<P, Att>) -> ControllerOutput<P, Att>;
}
