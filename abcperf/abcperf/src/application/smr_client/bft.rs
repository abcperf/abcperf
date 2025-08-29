use std::{fmt::Display, future::Future, net::SocketAddr};

use futures::{stream::FuturesUnordered, FutureExt};
use shared_ids::{ClientId, ReplicaId};
use tokio::select;
use tokio_stream::StreamExt;
use tracing::{debug, debug_span, Instrument};

use crate::application::{
    client::{SMRClient, SMRClientError, SMRClientFactory},
    ApplicationRequest, ApplicationResponse,
};

use super::{SharedClient, SharedFactory};

pub struct BftClient<Req: ApplicationRequest, Resp: ApplicationResponse> {
    shared: SharedClient<Req, Resp>,
    t: u64,
}

impl<Req: ApplicationRequest, Resp: ApplicationResponse> SMRClient<Req, Resp>
    for BftClient<Req, Resp>
{
    async fn send_request(&mut self, request: Req) -> Result<Resp, SMRClientError> {
        let request_id = self.shared.get_next_request_id();
        let fut = get_t_plus_one(&self.shared.replicas, self.t, |addr| {
            self.shared
                .client
                .request(self.shared.client_id, request_id, addr, request.clone())
        });
        select! {
            biased;
            () = self.shared.stop.cancelled() => {
                Err(SMRClientError::Cancelled)
            }
            res = fut => {
                match res {
                    Some(resp) => {
                        let mut resp = Vec::from(resp);
                        for resp in resp.windows(2) {
                            if resp[0] != resp[1] {
                                return Err(SMRClientError::GotAtLeastOneInvalidResponse);
                            }
                        }
                        Ok(resp.pop().unwrap())
                    },
                    None => {
                        Err(SMRClientError::NotEnoughValidResponse)
                    }
                }
            }
        }
    }

    fn client_id(&self) -> ClientId {
        self.shared.client_id
    }
}

pub async fn get_t_plus_one<T, E: Display, F: Future<Output = Result<T, E>>>(
    replicas: &[(ReplicaId, SocketAddr)],
    t: u64,
    mut req: impl FnMut(SocketAddr) -> F,
) -> Option<Box<[T]>> {
    let t = t.try_into().unwrap();

    let futures_unordered: FuturesUnordered<_> = replicas
        .iter()
        .copied()
        .map(|(id, addr)| {
            let span = debug_span!("to", replica = id.as_u64());
            req(addr).map(move |r| (id, r)).instrument(span)
        })
        .collect();

    assert!(futures_unordered.len() > t);

    let responses: Vec<T> = futures_unordered
        .filter_map(|(id, r)| match r {
            Ok(t) => {
                debug!("got response from {:?}", id);
                Some(t)
            }
            Err(e) => {
                debug!("got error from {:?}: {}", id, e);
                None
            }
        })
        .take(t + 1)
        .collect()
        .await;

    (responses.len() == t + 1).then(|| responses.into())
}

pub struct BftClientFactory<Req: ApplicationRequest, Resp: ApplicationResponse> {
    pub(crate) shared: SharedFactory<Req, Resp>,
    pub(crate) t: u64,
}

impl<Req: ApplicationRequest, Resp: ApplicationResponse> SMRClientFactory<Req, Resp>
    for BftClientFactory<Req, Resp>
{
    type Client = BftClient<Req, Resp>;

    fn create_client(&mut self) -> Self::Client {
        let shared = self.shared.create_shared_client();
        let t = self.t;
        BftClient { shared, t }
    }
}
