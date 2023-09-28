use std::{future::Future, net::SocketAddr, pin::Pin};

use async_trait::async_trait;

use anyhow::Result;
use serde::{Deserialize, Serialize};
use tokio::sync::oneshot;

pub mod http_warp;
mod json;
pub mod quic_quinn;
pub mod quic_s2n;
pub mod typed;

pub trait CSTrait: Clone + Copy + Send + Sync + 'static + Default {
    type Config: CSConfig;

    fn configure(self, ca: Vec<u8>, priv_key: Vec<u8>, cert: Vec<u8>) -> Self::Config;
}

pub trait CSConfig: Clone + Send + Sync {
    type Client: CSTraitClient;
    type Server: CSTraitServer;

    fn client(&self) -> Self::Client;
    fn server(&self, local_socket: SocketAddr) -> Self::Server;
}

#[async_trait]
pub trait CSTraitClient: Send + Sync {
    type Connection: CSTraitClientConnection;

    async fn connect(&self, addr: SocketAddr, server_name: &str) -> Result<Self::Connection>;
    fn local_addr(&self) -> SocketAddr;
}

#[async_trait]
pub trait CSTraitClientConnection: Send + Sync {
    async fn request<Request: Serialize + Send + Sync, Response: for<'a> Deserialize<'a>>(
        &mut self,
        request: Request,
    ) -> Result<Response>;
}

pub trait CSTraitServer: Send {
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
    ) -> (SocketAddr, Pin<Box<dyn Future<Output = ()> + Send>>);
}
