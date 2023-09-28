use std::{collections::HashMap, fmt::Display, iter, net::SocketAddr, time::Duration};

use abcperf::{generator::GeneratorConfig, EndRng};
use futures::{future, stream::FuturesUnordered, Future, FutureExt, StreamExt};
use rand::Rng;
use serde::{Deserialize, Serialize};
use shared_ids::{AnyId, ClientId, ReplicaId};
use tokio::time;
use tracing::{debug, debug_span, Instrument};

pub mod cs;
pub mod no_proxy;
pub mod proxy;
pub mod response;

#[derive(Deserialize, Serialize, Clone, Debug)]
pub enum CustomClientConfig {
    #[serde(rename = "n-clients")]
    NClients { number: u64 },

    #[serde(rename = "distribution")]
    Distribution(GeneratorConfig),
}

impl From<CustomClientConfig> for HashMap<String, String> {
    fn from(config: CustomClientConfig) -> Self {
        match config {
            CustomClientConfig::NClients { number } => {
                let mut map = HashMap::new();
                map.insert("type".to_owned(), "n-clients".to_owned());
                map.insert("number".to_owned(), number.to_string());
                map
            }
            CustomClientConfig::Distribution(generator_config) => {
                let mut map: HashMap<String, String> = generator_config.into();
                assert!(map
                    .insert("type".to_owned(), "distribution".to_owned())
                    .is_none());
                map
            }
        }
    }
}

/// message type that provides generators
pub trait Generate<C> {
    /// the message generator, should be a infinite iterator
    type Generator: Iterator<Item = Self> + Send;

    /// create a new infinite message generator
    fn generator(config: &C, client_id: ClientId) -> Self::Generator;
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
        .filter_map(|(id, r)| {
            future::ready(match r {
                Ok(t) => {
                    debug!("got response from {:?}", id);
                    Some(t)
                }
                Err(e) => {
                    debug!("got error from {:?}: {}", id, e);
                    None
                }
            })
        })
        .take(t + 1)
        .collect()
        .await;

    (responses.len() == t + 1).then(|| responses.into())
}

pub async fn get_one_with_fallback<T, E: Display, F: Future<Output = Result<T, E>>>(
    replicas: &[(ReplicaId, SocketAddr)],
    rng: &mut EndRng,
    fallback_timeout: Duration,
    mut req: impl FnMut(SocketAddr) -> F,
) -> Option<T> {
    let first = rng.gen_range(0..replicas.len());

    let mut iter = iter::once(&replicas[first])
        .chain(replicas[..first].iter())
        .chain(replicas[first + 1..].iter())
        .copied()
        .map(|(id, addr)| {
            let span = debug_span!("to", replica = id.as_u64());
            req(addr).map(move |r| (id, r)).instrument(span)
        });

    let mut first = Box::pin(iter.next().unwrap());
    let sleep = time::sleep(fallback_timeout);

    let mut first = tokio::select! {
        _ = sleep => {
            Some(first)
        }
        (id, res) = &mut first => {
             match res {
                Ok(t) => {
                    debug!("got response from first {:?}", id);
                    return Some(t);
                }
                Err(e) => {
                    debug!("got error from first {:?}: {}", id, e);
                    None
                }
            }
        }
    };

    let mut futures_unordered: FuturesUnordered<_> = iter.collect();

    loop {
        let (id, res) = if let Some(f) = first.as_mut() {
            tokio::select! {
                (id, res) = f => {
                    match res {
                        Ok(t) => {
                            debug!("got delayed response from first {:?}", id);
                            return Some(t);
                        }
                        Err(e) => {
                            debug!("got delayed error from first {:?}: {}", id, e);
                            first = None;
                            continue;
                        }
                    }
                }
                Some(res) = futures_unordered.next() => {
                    res
                }
                else => unreachable!()
            }
        } else {
            match futures_unordered.next().await {
                Some(res) => res,
                None => {
                    debug!("no replica return a valid response");
                    return None;
                }
            }
        };

        match res {
            Ok(t) => {
                debug!("got response from {:?}", id);
                return Some(t);
            }
            Err(e) => {
                debug!("got error from {:?}: {}", id, e);
            }
        }
    }
}
