use std::{fmt::Display, future::Future, net::SocketAddr, time::Duration};

use futures::{stream::FuturesUnordered, FutureExt};
use rand::{seq::SliceRandom, Rng, SeedableRng};
use shared_ids::{ClientId, ReplicaId};
use tokio::{select, time};
use tokio_stream::StreamExt;
use tracing::{debug, debug_span, Instrument};

use crate::{
    application::{
        client::{SMRClient, SMRClientError, SMRClientFactory},
        ApplicationRequest, ApplicationResponse,
    },
    EndRng,
};

use super::{CurrentPrimary, SharedClient, SharedFactory};

pub struct FallbackClient<Req: ApplicationRequest, Resp: ApplicationResponse> {
    shared: SharedClient<Req, Resp>,
    default: ReplicaId,
    primary_hint: CurrentPrimary,
    fallback_timeout: Duration,
    rng: EndRng,
}

impl<Req: ApplicationRequest, Resp: ApplicationResponse> SMRClient<Req, Resp>
    for FallbackClient<Req, Resp>
{
    async fn send_request(&mut self, request: Req) -> Result<Resp, SMRClientError> {
        let first_replica = self.primary_hint.get().unwrap_or(self.default);
        let request_id = self.shared.get_next_request_id();
        let fut = get_one_with_fallback(
            &self.shared.replicas,
            first_replica,
            self.fallback_timeout,
            |addr| {
                self.shared
                    .client
                    .request(self.shared.client_id, request_id, addr, request.clone())
            },
            &mut self.rng,
        );
        select! {
            biased;
            () = self.shared.stop.cancelled() => {
                Err(SMRClientError::Cancelled)
            }
            res = fut => {
                match res {
                    Some((id, resp)) => {
                        self.default = id;
                        Ok(resp)
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

async fn get_one_with_fallback<T, E: Display, F: Future<Output = Result<T, E>>>(
    replicas: &[(ReplicaId, SocketAddr)],
    first_replica: ReplicaId,
    fallback_timeout: Duration,
    mut req: impl FnMut(SocketAddr) -> F,
    rng: &mut EndRng,
) -> Option<(ReplicaId, T)> {
    let first = replicas
        .iter()
        .find(|(id, _)| *id == first_replica)
        .expect("first replica not found");
    let mut replicas = replicas
        .iter()
        .filter(|(id, _)| *id != first_replica)
        .collect::<Vec<_>>();
    replicas.shuffle(rng);
    replicas.push(first);

    let mut futures = replicas.into_iter().rev().copied().map(|(id, addr)| {
        let span = debug_span!("to", replica = id.as_u64());
        req(addr).map(move |r| (id, r)).instrument(span)
    });

    let mut running = FuturesUnordered::new();
    running.push(futures.next().expect("we have at least one future"));

    for future in futures {
        let sleep = time::sleep(fallback_timeout);
        tokio::select! {
            _ = sleep => {
                debug!("fallback timeout");
            }
            res = running.next() => {
                match res {
                    Some((id, Ok(t))) => {
                        debug!("response from {:?}", id);
                        return Some((id, t));
                    }
                    Some((id, Err(e))) => {
                        debug!("error from {:?}: {}", id, e);
                    }
                    None => unreachable!(),
                }
            }
        }
        running.push(future);
    }

    while let Some((id, res)) = running.next().await {
        match res {
            Ok(t) => {
                debug!("response from {:?}", id);
                return Some((id, t));
            }
            Err(e) => {
                debug!("error from {:?}: {}", id, e);
            }
        }
    }

    debug!("no replica return a valid response");
    None
}

pub struct FallbackClientFactory<Req: ApplicationRequest, Resp: ApplicationResponse> {
    pub(crate) shared: SharedFactory<Req, Resp>,
    pub(crate) primary_hint: CurrentPrimary,
    pub(crate) rng: EndRng,
    pub(crate) fallback_timeout: Duration,
}

impl<Req: ApplicationRequest, Resp: ApplicationResponse> SMRClientFactory<Req, Resp>
    for FallbackClientFactory<Req, Resp>
{
    type Client = FallbackClient<Req, Resp>;

    fn create_client(&mut self) -> Self::Client {
        let shared = self.shared.create_shared_client();
        let default = ReplicaId::from_u64(self.rng.gen_range(0..shared.replicas.len()) as u64);
        let primary_hint = self.primary_hint.clone();
        let fallback_timeout = self.fallback_timeout;
        let rng = EndRng::from_rng(&mut self.rng).expect("seed rng should always work");

        FallbackClient {
            shared,
            default,
            primary_hint,
            fallback_timeout,
            rng,
        }
    }
}
