use std::time::{Duration, Instant};

use abcperf::stats::{ClientSample, ClientStats, Micros, Timestamp};
use abcperf::{
    application::client::{
        ApplicationClientEmulator, ApplicationClientEmulatorArguments, SMRClient, SMRClientError,
        SMRClientFactory,
    },
    config::ApplicationClientEmulatorConfig,
    generator::start_generator,
};
use futures::future::{self, Either};
use maas_types::{CheckinMsg, CheckoutMsg, ClientRequest, InnerClientRequest, Interaction};

use anyhow::anyhow;

use sgx_crypto::ecc::{EcKeyPair, EcPrivateKey, EcPublicKey};
use sgx_types::types::Ec256PublicKey;
use sgx_types::types::Key128bit;
use shared_ids::ClientId;
use tokio::{
    sync::{
        mpsc::{self, error::TrySendError},
        OnceCell,
    },
    task::JoinHandle,
};
use tracing::{debug, info, warn, Instrument};

use anyhow::Result;

use crate::{
    payload::{EncryptedRequest, InnerRequest, InnerResponse, Request, Response},
    Config,
};

pub struct MaasApplicationClient {}

impl<CF: SMRClientFactory<Request, Response>> ApplicationClientEmulator<CF>
    for MaasApplicationClient
{
    type Request = Request;
    type Response = Response;
    type Config = Config;

    fn start(
        client_factory: CF,
        config: ApplicationClientEmulatorConfig<Self::Config>,
        args: ApplicationClientEmulatorArguments,
    ) -> JoinHandle<Result<Vec<ClientStats>>> {
        tokio::spawn(Self::run(client_factory, config, args))
    }
}

impl MaasApplicationClient {
    async fn run<CF: SMRClientFactory<Request, Response>>(
        mut client_factory: CF,
        config: ApplicationClientEmulatorConfig<Config>,
        args: ApplicationClientEmulatorArguments,
    ) -> Result<Vec<ClientStats>> {
        let ApplicationClientEmulatorArguments {
            start_time,
            stop,
            mut rng,
            live_config,
            sender_stats_live,
        } = args;

        let (generator_join_handel, new_request, mut new_consumer) = start_generator(
            stop,
            config.application.maximum_client_count,
            config.application.distribution,
            &mut rng,
            live_config,
            config.client_workers,
            config.worker_id,
        );

        let mut client_join_handles = Vec::new();

        while let Some(()) = new_consumer.recv().await {
            let client = client_factory.create_client();

            client_join_handles.push(tokio::spawn(
                Self::client_loop(client, new_request.clone(), sender_stats_live.clone())
                    .in_current_span(),
            ));
        }

        info!("Stopped creating new clients");

        let clients = future::join_all(client_join_handles.into_iter()).await;

        debug!("Collected all clients' results");

        generator_join_handel.await?;

        clients
            .into_iter()
            .map(|e| {
                Ok(ClientStats {
                    samples: e?
                        .into_iter()
                        .filter_map(|(i, d)| {
                            Timestamp::new(start_time, i).map(|timestamp| ClientSample {
                                timestamp,
                                processing_time: d.into(),
                            })
                        })
                        .collect(),
                })
            })
            .collect::<Result<Vec<_>>>()
    }

    async fn client_loop<C: SMRClient<Request, Response>>(
        mut client: C,
        receiver: async_channel::Receiver<()>,
        sender_stats_live: mpsc::Sender<Micros>,
    ) -> Vec<(Instant, Duration)> {
        let mut last_log = Instant::now();
        let mut elapsed_vec = Vec::new();

        let my_key = EcKeyPair::create().unwrap();
        let (my_priv_key, my_pub_key) = my_key.into();

        let shared_key = OnceCell::new();
        let mut is_checkin = true;

        while let Ok(()) = receiver.recv().await {
            let client = &mut client;
            let key = match shared_key
                .get_or_try_init(|| async {
                    let resp: Response = client
                        .send_request(InnerRequest::RequestRemoteAttestation.into())
                        .await?;
                    let attestation = match resp.0 {
                        InnerResponse::RemoteAttestation(r) => r,
                        _ => return Err(anyhow!("invalid response")),
                    };
                    attestation.verify()?;

                    let server_pub_key = Ec256PublicKey::from(attestation.shared_pub_key).into();

                    let enc_key = gen_shared_key(&my_priv_key, &server_pub_key);
                    Ok(enc_key)
                })
                .await
            {
                Ok(key) => key,
                Err(e) => {
                    warn!("client aborted because of error: {e}");
                    break;
                }
            };

            let payload =
                client_request(is_checkin, client.client_id(), *shared_key.get().unwrap());

            is_checkin = !is_checkin;

            let start = Instant::now();

            match Self::single_request(client, &my_pub_key, payload, *key).await {
                Ok(resp) => resp,
                Err(Either::Left(SMRClientError::Cancelled)) => {
                    break;
                }
                Err(Either::Left(e)) => {
                    warn!("client aborted because of error: {e}");
                    break;
                }
                Err(Either::Right(e)) => {
                    warn!("client aborted because of error: {e}");
                    break;
                }
            }

            let elapsed = start.elapsed();
            match sender_stats_live.try_send(elapsed.into()) {
                Ok(_) | Err(TrySendError::Closed(_)) => {}
                Err(TrySendError::Full(_)) => {
                    let now = Instant::now();
                    if now.duration_since(last_log) > Duration::from_secs(10) {
                        warn!("too many requests, live stats inaccurate");
                        last_log = now;
                    }
                }
            }
            elapsed_vec.push((start, elapsed));
        }
        elapsed_vec
    }

    async fn single_request<C: SMRClient<Request, Response>>(
        client: &mut C,
        my_pub_key: &EcPublicKey,
        client_req: ClientRequest,
        key: Key128bit,
    ) -> Result<(), Either<SMRClientError, anyhow::Error>> {
        let transaction =
            EncryptedRequest::from_client_request(client_req, key, my_pub_key.public_key().into())
                .map_err(Either::Right)?;

        let response: Response = client
            .send_request(transaction.into())
            .await
            .map_err(Either::Left)?;

        let response = match response.0 {
            InnerResponse::Encrypted(r) => r,
            _ => return Err(Either::Right(anyhow!("invalid response"))),
        };

        response
            .decrypt(key)
            .map_err(|_| Either::Right(anyhow!("failed to decrypt response")))?;

        Ok(())
    }
}

fn client_request(is_checkin: bool, client_id: ClientId, shared_key: Key128bit) -> ClientRequest {
    let inner = if is_checkin {
        InnerClientRequest::Checkin(CheckinMsg {
            interaction: Interaction {
                short_term_secret: 0,
                timestamp: (),
                location: (),
                sig: (),
            },
            long_term_secret: client_id.as_u64(),
            next_short_term_secret: 0,
        })
    } else {
        InnerClientRequest::Checkout(CheckoutMsg {
            interaction: Interaction {
                short_term_secret: 0,
                timestamp: (),
                location: (),
                sig: (),
            },
            long_term_secret: client_id.as_u64(),
            next_short_term_secret: 0,
        })
    };
    ClientRequest {
        inner,
        response_key: shared_key,
    }
}

// TODO: dedup
fn gen_shared_key(priv_key: &EcPrivateKey, pub_key: &EcPublicKey) -> Key128bit {
    priv_key
        .shared_key(pub_key)
        .unwrap()
        .derive_key(b"some label")
        .unwrap()
        .key
}
