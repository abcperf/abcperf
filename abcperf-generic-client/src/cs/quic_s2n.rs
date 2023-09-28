use std::{
    future::Future,
    net::{Ipv6Addr, SocketAddr},
    pin::Pin,
};

use anyhow::{anyhow, Result};
use async_trait::async_trait;
use futures::FutureExt;
use s2n_quic::{
    client::Connect, provider::tls, stream::BidirectionalStream, Client, Connection, Server,
};
use serde::{Deserialize, Serialize};
use tokio::{io::AsyncReadExt, select, sync::oneshot};
use tracing::{error, info, info_span, Instrument};

use crate::cs::{CSConfig, CSTrait, CSTraitClient, CSTraitClientConnection, CSTraitServer};

#[derive(Debug, Clone, Copy, Default)]
pub struct QuicS2N;

impl CSTrait for QuicS2N {
    type Config = QuicS2NConfig;

    fn configure(self, ca: Vec<u8>, priv_key: Vec<u8>, cert: Vec<u8>) -> Self::Config {
        QuicS2NConfig {
            tls_client_config: tls::default::Client::builder()
                .with_certificate(ca)
                .unwrap()
                .build()
                .unwrap(),
            tls_server_config: tls::default::Server::builder()
                .with_certificate(cert, priv_key)
                .unwrap()
                .build()
                .unwrap(),
        }
    }
}

#[derive(Clone)]
pub struct QuicS2NConfig {
    tls_client_config: tls::default::Client,
    tls_server_config: tls::default::Server,
}

impl CSConfig for QuicS2NConfig {
    type Client = QuicS2NClient;

    fn client(&self) -> Self::Client {
        let client = Client::builder()
            .with_tls(self.tls_client_config.clone())
            .unwrap()
            .with_io((Ipv6Addr::UNSPECIFIED, 0))
            .unwrap()
            .start()
            .unwrap();

        QuicS2NClient { client }
    }

    type Server = QuicS2NServer;

    fn server(&self, local_socket: SocketAddr) -> Self::Server {
        QuicS2NServer {
            local_socket,
            tls_server_config: self.tls_server_config.clone(),
        }
    }
}

#[derive(Debug)]
pub struct QuicS2NClient {
    client: Client,
}

#[async_trait]
impl CSTraitClient for QuicS2NClient {
    type Connection = QuicS2NClientConnection;

    async fn connect(&self, addr: SocketAddr, server_name: &str) -> Result<Self::Connection> {
        let connection = self
            .client
            .connect(Connect::new(addr).with_server_name(server_name))
            .await
            .map_err(|e| anyhow!("failed to connect: {}", e))?;

        Ok(QuicS2NClientConnection { connection })
    }

    fn local_addr(&self) -> SocketAddr {
        self.client.local_addr().unwrap()
    }
}

#[derive(Debug)]
pub struct QuicS2NClientConnection {
    connection: Connection,
}

#[async_trait]
impl CSTraitClientConnection for QuicS2NClientConnection {
    async fn request<Request: Serialize + Send + Sync, Response: for<'a> Deserialize<'a>>(
        &mut self,
        request: Request,
    ) -> Result<Response> {
        let mut stream = self
            .connection
            .open_bidirectional_stream()
            .await
            .map_err(|e| anyhow!("failed to open stream: {}", e))?;
        stream
            .send(bincode::serialize::<Request>(&request)?.into())
            .await?;
        stream.finish()?;

        let mut resp = Vec::new();
        stream
            .read_to_end(&mut resp)
            .await
            .map_err(|e| anyhow!("failed to read response: {}", e))?;

        Ok(bincode::deserialize::<Response>(&resp)?)
    }
}

pub struct QuicS2NServer {
    local_socket: SocketAddr,
    tls_server_config: tls::default::Server,
}

impl CSTraitServer for QuicS2NServer {
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
        let endpoint = Server::builder()
            .with_tls(self.tls_server_config)
            .unwrap()
            .with_io(self.local_socket)
            .unwrap()
            .start()
            .unwrap();

        let local_addr = endpoint.local_addr().unwrap();

        info!("listening on {}", local_addr);

        let join_handle = tokio::spawn(Self::run::<S, Request, Response, Fut, F>(
            endpoint, shared, on_request, exit,
        ));

        (local_addr, Box::pin(join_handle.map(|_| ())))
    }
}

impl QuicS2NServer {
    async fn run<
        S: Clone + Send + Sync + 'static,
        Request: for<'a> Deserialize<'a> + Send + 'static,
        Response: Serialize + Send + 'static,
        Fut: Future<Output = Option<Response>> + Send + 'static,
        F: Fn(S, Request) -> Fut + Send + Sync + Clone + 'static,
    >(
        mut endpoint: Server,
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
                            error!("connection failed: {reason}", reason = e.to_string())
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
        mut connection: Connection,
        shared: S,
        on_request: F,
    ) -> Result<()> {
        let span = info_span!(
            "connection",
            remote = %connection.remote_addr().unwrap(),
        );
        async {
            info!("established");

            // Each stream initiated by the client constitutes a new request.
            loop {
                let stream = connection.accept_bidirectional_stream().await;
                let stream = match stream {
                    Err(e) => {
                        return Err(e);
                    }
                    Ok(Some(s)) => s,
                    Ok(None) => todo!(),
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
        mut stream: BidirectionalStream,
        shared: S,
        on_request: F,
    ) -> Result<()> {
        let mut req = Vec::new();
        stream
            .read_to_end(&mut req)
            .await
            .map_err(|e| anyhow!("failed reading request: {}", e))?;
        let req = req;

        let req =
            bincode::deserialize::<Request>(&req).expect("request was not of expected format");

        let response = on_request(shared, req).await;

        if let Some(response) = response {
            let response = bincode::serialize::<Response>(&response)?;

            stream
                .send(response.into())
                .await
                .map_err(|e| anyhow!("failed to send response: {}", e))?;
        }

        stream.finish()?;

        info!("complete");

        Ok(())
    }
}
