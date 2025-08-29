use std::collections::HashSet;

use abcperf::application::server::ApplicationServer;
use abcperf::application::server::ApplicationServerChannels;
use abcperf::application::server::ApplicationServerStarted;
use abcperf::config::ApplicationServerConfig;
use abcperf::MessageDestination;
use maas_tee::enclave::{Enclave, MAttestation, ProposedSecret};
use maas_tee::{TO_BFT_SAFE, TO_CLIENT_SAFE};

use anyhow::Result;

use serde::{Deserialize, Serialize};
use shared_ids::ClientId;
use shared_ids::RequestId;
use std::sync::Arc;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tokio_stream::wrappers::UnboundedReceiverStream;
use tracing::error;

use crate::payload::EncrypteTransaction;
use crate::payload::EncryptedRequest;
use crate::payload::Request;
use crate::payload::{EncryptedResponse, InnerRequest, InnerResponse, Response};
use crate::Config;
use futures::{stream, Future, StreamExt};
use tracing::Instrument;

pub struct MaasServer {}

impl ApplicationServer for MaasServer {
    type Request = Request;
    type Response = Response;
    type Transaction = EncrypteTransaction;
    type ReplicaMessage = MaasReplicaMessage;
    type Config = Config;

    fn start<F: Send + Future<Output = ()> + 'static>(
        config: ApplicationServerConfig<Self::Config>,
        channels: ApplicationServerChannels<Self>,
        send_to_replica: impl 'static + Sync + Send + Fn(MessageDestination, Self::ReplicaMessage) -> F,
    ) -> ApplicationServerStarted<Self> {
        let (abc_transaction_input_send, abc_transaction_input_recv) = mpsc::channel(1000);
        let (abc_transaction_output_send, abc_transaction_output_recv) = mpsc::channel(1000);

        let application_server_join_handle = tokio::spawn(Self::run(
            config,
            channels,
            abc_transaction_input_send,
            abc_transaction_output_recv,
            send_to_replica,
        ));

        ApplicationServerStarted {
            abc_transaction_input_channel: abc_transaction_input_recv,
            abc_transaction_output_channel: abc_transaction_output_send,
            application_server_join_handle,
        }
    }
}

impl MaasServer {
    async fn run<F: Send + Future<Output = ()>>(
        config: ApplicationServerConfig<Config>,
        channels: ApplicationServerChannels<Self>,
        abc_transaction_input_channel: mpsc::Sender<(ClientId, RequestId, EncrypteTransaction)>,
        abc_transaction_output_channel: mpsc::Receiver<(ClientId, RequestId, EncrypteTransaction)>,
        replica_send: impl 'static + Sync + Send + Fn(MessageDestination, MaasReplicaMessage) -> F,
    ) -> Result<()> {
        let enclave = Arc::new(Enclave::new(config.replica_id.as_u64() as usize).unwrap());

        let ApplicationServerChannels {
            mut smr_requests,
            smr_responses,
            incoming_replica_messages: replica_recv,
            ready,
            shutdown,
        } = channels;

        replica_send(
            MessageDestination::Broadcast,
            MaasReplicaMessage::RemoteAttestation(enclave.attestation().unwrap().clone()),
        )
        .await;

        let n: u64 = config.n.into();
        let mut ma_ready_send = Some(ready);

        let local_enclave = enclave.clone();
        let (mut replica_recv, replica_recv_abort) =
            stream::abortable(UnboundedReceiverStream::new(replica_recv));
        let replica_recv = tokio::spawn(
            async move {
                let mut proposals_from = HashSet::new();
                proposals_from.insert(config.replica_id);
                while let Some((_msg_type, id, msg)) = replica_recv.next().await {
                    // ignore own messages
                    if id != config.replica_id {
                        match msg {
                            MaasReplicaMessage::RemoteAttestation(ra) => {
                                let proposed_secret = local_enclave.ma_continue(ra).unwrap();
                                replica_send(
                                    MessageDestination::Unicast(id),
                                    MaasReplicaMessage::ProposedSecret(proposed_secret),
                                )
                                .await;
                            }
                            MaasReplicaMessage::ProposedSecret(proposed_secret) => {
                                local_enclave.ma_finish(proposed_secret).unwrap();
                                proposals_from.insert(id);
                                if proposals_from.len() as u64 >= n {
                                    if let Some(ma_ready_send) = ma_ready_send.take() {
                                        ma_ready_send.send(()).unwrap();
                                    }
                                }
                            }
                        }
                    }
                }
            }
            .in_current_span(),
        );

        let (send_to_bft, recv_to_bft) = mpsc::unbounded_channel();
        let (mut recv_to_bft, recv_to_bft_abort) =
            stream::abortable(UnboundedReceiverStream::new(recv_to_bft));
        assert!(TO_BFT_SAFE
            .set(Box::new(move |c, r, p, mac| {
                let _ = send_to_bft.send((c, r, p, mac));
            }))
            .is_ok());

        let to_bft = tokio::spawn(
            async move {
                while let Some((client_id, request_id, payload, mac)) = recv_to_bft.next().await {
                    if abc_transaction_input_channel
                        .send((
                            client_id,
                            request_id,
                            EncrypteTransaction {
                                payload: payload.into(),
                                mac,
                            },
                        ))
                        .await
                        .is_err()
                    {
                        break;
                    }
                }
            }
            .in_current_span(),
        );

        let (send_to_client, recv_to_client) = mpsc::unbounded_channel();
        let (mut recv_to_client, recv_to_client_abort) =
            stream::abortable(UnboundedReceiverStream::new(recv_to_client));
        assert!(TO_CLIENT_SAFE
            .set(Box::new(move |c, r, p, mac| {
                let _ = send_to_client.send((c, r, p, mac));
            }))
            .is_ok());

        let to_client = {
            let smr_responses = smr_responses.clone();
            tokio::spawn(
                async move {
                    while let Some((client_id, request_id, enc_payload, mac)) =
                        recv_to_client.next().await
                    {
                        smr_responses
                            .send((
                                client_id,
                                request_id,
                                InnerResponse::Encrypted(EncryptedResponse { enc_payload, mac })
                                    .into(),
                            ))
                            .await
                            .unwrap();
                    }
                }
                .in_current_span(),
            )
        };

        let from_client = {
            let enclave = enclave.clone();
            tokio::spawn(async move {
                while let Some((client_id, request_id, request)) = smr_requests.recv().await {
                    match request.0 {
                        InnerRequest::Encrypted(EncryptedRequest {
                            payload,
                            mac,
                            pubkey,
                        }) => {
                            enclave
                                .from_client(client_id, request_id, pubkey, &payload, mac)
                                .unwrap();
                        }
                        InnerRequest::RequestRemoteAttestation => {
                            let to_send = enclave.shared_attestation().unwrap();
                            let _ = smr_responses
                                .send((
                                    client_id,
                                    request_id,
                                    InnerResponse::RemoteAttestation(to_send.clone()).into(),
                                ))
                                .await;
                        }
                    }
                }
            })
        };

        let (mut abc_transaction_output_channel, abc_output_abort) =
            stream::abortable(ReceiverStream::new(abc_transaction_output_channel));
        let abc_output = tokio::spawn(
            async move {
                while let Some((client_id, request_id, EncrypteTransaction { payload, mac })) =
                    abc_transaction_output_channel.next().await
                {
                    enclave
                        .from_bft(client_id, request_id, &payload, mac)
                        .unwrap();
                }
            }
            .in_current_span(),
        );

        if let Err(err) = shutdown.await {
            error!("Shutdown channel error: {}", err);
        }

        recv_to_bft_abort.abort();
        to_bft.await.unwrap();

        recv_to_client_abort.abort();
        to_client.await.unwrap();

        abc_output_abort.abort();
        abc_output.await.unwrap();

        // webserver was shutdown before so this should shutdown automatically, so just waiting
        from_client.await.unwrap();

        replica_recv_abort.abort();
        replica_recv.await.unwrap();

        Ok(())
    }
}

#[derive(Serialize, Deserialize, Debug)]
pub enum MaasReplicaMessage {
    RemoteAttestation(MAttestation),
    ProposedSecret(ProposedSecret),
}
