use std::{
    future::Future,
    net::{Ipv6Addr, SocketAddr},
    pin::Pin,
    sync::Arc,
    time::Duration,
};

use anyhow::{anyhow, Result};
use async_trait::async_trait;
use futures::FutureExt;
use quinn::{
    ClientConfig, Connecting, Connection, Endpoint, RecvStream, SendStream, ServerConfig,
    TransportConfig, VarInt,
};
use rustls::{Certificate, PrivateKey, RootCertStore};
use serde::{Deserialize, Serialize};
use tokio::{select, sync::oneshot};
use tracing::{error, info, info_span, Instrument};

use crate::cs::{CSConfig, CSTrait, CSTraitClient, CSTraitClientConnection, CSTraitServer};

const READ_TO_END_LIMIT: usize = 128 * 1024 * 1024;

#[derive(Debug, Clone, Copy, Default)]
pub struct QuicQuinn;

impl CSTrait for QuicQuinn {
    type Config = QuicQuinnConfig;

    fn configure(self, ca: Vec<u8>, priv_key: Vec<u8>, cert: Vec<u8>) -> Self::Config {
        let mut roots = RootCertStore::empty();
        roots.add(&Certificate(ca)).unwrap();
        let mut client_config = ClientConfig::with_root_certificates(roots);

        let mut transport_config = TransportConfig::default();
        transport_config.max_idle_timeout(Some(VarInt::from_u32(30_000).into()));
        transport_config.keep_alive_interval(Some(Duration::from_secs(10)));
        client_config.transport_config(Arc::new(transport_config));

        let priv_key = PrivateKey(priv_key);
        let cert_chain = vec![Certificate(cert)];
        let mut server_config = ServerConfig::with_single_cert(cert_chain, priv_key).unwrap();
        Arc::get_mut(&mut server_config.transport)
            .expect("config was just created")
            .max_idle_timeout(Some(VarInt::from_u32(30_000).into()));

        QuicQuinnConfig {
            client_config,
            server_config,
        }
    }
}

#[derive(Debug, Clone)]
pub struct QuicQuinnConfig {
    client_config: ClientConfig,
    server_config: ServerConfig,
}

impl CSConfig for QuicQuinnConfig {
    type Client = QuicQuinnClient;

    fn client(&self) -> Self::Client {
        let mut client = Endpoint::client((Ipv6Addr::UNSPECIFIED, 0).into()).unwrap();
        client.set_default_client_config(self.client_config.clone());

        QuicQuinnClient { client }
    }

    type Server = QuicQuinnServer;

    fn server(&self, local_socket: SocketAddr) -> Self::Server {
        QuicQuinnServer {
            local_socket,
            client_config: self.client_config.clone(),
            server_config: self.server_config.clone(),
        }
    }
}

#[derive(Debug)]
pub struct QuicQuinnClient {
    client: Endpoint,
}

#[async_trait]
impl CSTraitClient for QuicQuinnClient {
    type Connection = QuicQuinnClientConnection;

    async fn connect(&self, addr: SocketAddr, server_name: &str) -> Result<Self::Connection> {
        let connection = self
            .client
            .connect(addr, server_name)?
            .await
            .map_err(|e| anyhow!("failed to connect: {}", e))?;

        Ok(QuicQuinnClientConnection { connection })
    }

    fn local_addr(&self) -> SocketAddr {
        self.client.local_addr().unwrap()
    }
}

#[derive(Debug)]
pub struct QuicQuinnClientConnection {
    connection: Connection,
}

#[async_trait]
impl CSTraitClientConnection for QuicQuinnClientConnection {
    async fn request<Request: Serialize + Send + Sync, Response: for<'a> Deserialize<'a>>(
        &mut self,
        request: Request,
    ) -> Result<Response> {
        let (mut send, mut recv) = self
            .connection
            .open_bi()
            .await
            .map_err(|e| anyhow!("failed to open stream: {}", e))?;
        send.write_all(&bincode::serialize::<Request>(&request)?)
            .await
            .map_err(|e| anyhow!("failed to send request: {}", e))?;
        send.finish().await.map_err(|e| match e {
            quinn::WriteError::ConnectionLost(e) => {
                anyhow!("failed to shutdown stream: connection lost: {}", e)
            }
            quinn::WriteError::UnknownStream => todo!(),
            _ => anyhow!("failed to shutdown stream: {}", e),
        })?;

        let resp = recv
            .read_to_end(READ_TO_END_LIMIT)
            .await
            .map_err(|e| anyhow!("failed to read response: {}", e))?;

        Ok(bincode::deserialize::<Response>(&resp)?)
    }
}

#[derive(Debug)]
pub struct QuicQuinnServer {
    local_socket: SocketAddr,
    client_config: ClientConfig,
    server_config: ServerConfig,
}

impl CSTraitServer for QuicQuinnServer {
    fn start<
        S: Clone + Send + Sync + 'static,
        Request: for<'a> Deserialize<'a> + Send + 'static,
        Response: Serialize + Send + 'static,
        Fut: Future<Output = Option<Response>> + Send + 'static,
        F: Fn(S, Request) -> Fut + Send + Sync + Clone + 'static,
    >(
        self,
        shared: S,
        on_request: F,
        exit: oneshot::Receiver<()>,
    ) -> (SocketAddr, Pin<Box<dyn Future<Output = ()> + Send>>) {
        let mut endpoint = Endpoint::server(self.server_config, self.local_socket).unwrap();
        endpoint.set_default_client_config(self.client_config);

        let local_addr = endpoint.local_addr().unwrap();

        info!("listening on {}", local_addr);

        let join_handle = tokio::spawn(Self::run::<S, Request, Response, Fut, F>(
            endpoint, shared, on_request, exit,
        ));

        (local_addr, Box::pin(join_handle.map(|_| ())))
    }
}

impl QuicQuinnServer {
    async fn run<
        S: Clone + Send + Sync + 'static,
        Request: for<'a> Deserialize<'a> + Send + 'static,
        Response: Serialize + Send + 'static,
        Fut: Future<Output = Option<Response>> + Send + 'static,
        F: Fn(S, Request) -> Fut + Send + Sync + Clone + 'static,
    >(
        endpoint: Endpoint,
        shared: S,
        on_request: F,
        mut exit: oneshot::Receiver<()>,
    ) {
        loop {
            select! {
                biased;
                _ = &mut exit => break,
                Some(conn) = endpoint.accept() => {
                    info!("connection incoming");
                    let fut = Self::handle_connection::<S, Request, Response, Fut, F>(
                        conn,
                        shared.clone(),
                        on_request.clone(),
                    );
                    tokio::spawn(async move {
                        if let Err(e) = fut.await {
                            error!("connection failed: {}", e.to_string())
                        }
                    });
                },
                else => break
            }
        }
    }

    async fn handle_connection<
        S: Clone + Send + 'static,
        Request: for<'a> Deserialize<'a> + Send + 'static,
        Response: Serialize + Send + 'static,
        Fut: Future<Output = Option<Response>> + Send + 'static,
        F: Fn(S, Request) -> Fut + Send + Sync + Clone + 'static,
    >(
        connection: Connecting,

        shared: S,
        on_request: F,
    ) -> Result<()> {
        let connection = connection.await?;
        let span = info_span!(
            "connection",
            remote = %connection.remote_address(),
        );
        async {
            info!("established");

            // Each stream initiated by the client constitutes a new request.
            loop {
                let stream = connection.accept_bi().await;
                let stream = match stream {
                    Err(quinn::ConnectionError::ApplicationClosed { .. }) => {
                        info!("connection closed");
                        return Ok(());
                    }

                    Err(e) => {
                        return Err(e);
                    }
                    Ok(s) => s,
                };
                let fut = Self::handle_request::<S, Request, Response, Fut, F>(
                    stream,
                    shared.clone(),
                    on_request.clone(),
                );
                tokio::spawn(
                    async move {
                        if let Err(e) = fut.await {
                            error!("failed: {reason}", reason = e.to_string());
                        }
                    }
                    .instrument(info_span!("request")),
                );
            }
        }
        .instrument(span)
        .await?;
        Ok(())
    }

    async fn handle_request<
        S,
        Request: for<'a> Deserialize<'a>,
        Response: Serialize,
        Fut: Future<Output = Option<Response>>,
        F: Fn(S, Request) -> Fut,
    >(
        (mut send_quinn, mut recv_quinn): (SendStream, RecvStream),
        shared: S,
        on_request: F,
    ) -> Result<()> {
        let req = recv_quinn
            .read_to_end(READ_TO_END_LIMIT)
            .await
            .map_err(|e| anyhow!("failed reading request: {}", e))?;

        let req =
            bincode::deserialize::<Request>(&req).expect("request was not of expected format");

        let response = on_request(shared, req).await;

        if let Some(response) = response {
            let response = bincode::serialize::<Response>(&response)?;

            send_quinn
                .write_all(&response)
                .await
                .map_err(|e| anyhow!("failed to send response: {}", e))?;
        }

        send_quinn.finish().await.map_err(|e| match e {
            quinn::WriteError::ConnectionLost(e) => {
                anyhow!("failed to shutdown stream: connection lost: {}", e)
            }
            quinn::WriteError::UnknownStream => todo!(),
            _ => anyhow!("failed to shutdown stream: {}", e),
        })?;

        info!("complete");

        Ok(())
    }
}
