use std::{fmt::Debug, future::Future, marker::PhantomData, net::SocketAddr};

use anyhow::Result;
use derivative::Derivative;
use serde::{Deserialize, Serialize};
use tokio::sync::oneshot;

use crate::cs::{CSConfig, CSTrait, CSTraitClient, CSTraitClientConnection, CSTraitServer};

#[derive(Derivative)]
#[derivative(
    Debug(bound = "CS: Debug"),
    Clone(bound = ""),
    Copy(bound = ""),
    Default(bound = "")
)]
pub struct TypedCSTrait<
    CS: CSTrait,
    Request: Serialize + Send + Sync,
    Response: for<'a> Deserialize<'a>,
> {
    cs: CS,
    phantom_data: PhantomData<(Request, Response)>,
}

impl<
        CS: CSTrait,
        Request: Serialize + for<'a> Deserialize<'a> + Send + Sync + 'static,
        Response: Serialize + for<'a> Deserialize<'a> + Send + 'static,
    > TypedCSTrait<CS, Request, Response>
{
    pub fn new(cs: CS) -> Self {
        Self {
            cs,
            phantom_data: PhantomData::default(),
        }
    }

    pub fn configure(
        self,
        ca: Vec<u8>,
        priv_key: Vec<u8>,
        cert: Vec<u8>,
    ) -> TypedCSConfig<CS::Config, Request, Response> {
        TypedCSConfig {
            cs_config: self.cs.configure(ca, priv_key, cert),
            phantom_data: self.phantom_data,
        }
    }

    pub fn configure_debug(self) -> TypedCSConfig<CS::Config, Request, Response> {
        self.configure(
            abcperf::CERTIFICATE_AUTHORITY.to_vec(),
            abcperf::PRIVATE_KEY.to_vec(),
            abcperf::CERTIFICATE.to_vec(),
        )
    }
}

#[derive(Derivative)]
#[derivative(Clone(bound = ""))]
pub struct TypedCSConfig<
    Config: CSConfig,
    Request: Serialize + Send + Sync,
    Response: for<'a> Deserialize<'a>,
> {
    cs_config: Config,
    phantom_data: PhantomData<(Request, Response)>,
}

impl<
        Config: CSConfig,
        Request: Serialize + for<'a> Deserialize<'a> + Send + Sync,
        Response: Serialize + for<'a> Deserialize<'a> + Send,
    > TypedCSConfig<Config, Request, Response>
{
    pub fn client(&self) -> TypedCSTraitClient<Config::Client, Request, Response> {
        TypedCSTraitClient {
            cs_client: self.cs_config.client(),
            phantom_data: self.phantom_data,
        }
    }

    pub fn server(
        &self,
        local_socket: impl Into<SocketAddr>,
    ) -> TypedCSTraitServer<Config::Server, Request, Response> {
        TypedCSTraitServer {
            cs_server: self.cs_config.server(local_socket.into()),
            phantom_data: PhantomData,
        }
    }
}

#[derive(Derivative)]
#[derivative(Debug(bound = "CSClient: Debug"))]
pub struct TypedCSTraitClient<
    CSClient: CSTraitClient,
    Request: Serialize + Send + Sync,
    Response: for<'a> Deserialize<'a>,
> {
    cs_client: CSClient,
    phantom_data: PhantomData<(Request, Response)>,
}

impl<
        CSClient: CSTraitClient,
        Request: Serialize + Send + Sync,
        Response: for<'a> Deserialize<'a>,
    > TypedCSTraitClient<CSClient, Request, Response>
{
    pub async fn connect(
        &self,
        addr: impl Into<SocketAddr>,
        server_name: &str,
    ) -> Result<TypedCSTraitClientConnection<CSClient::Connection, Request, Response>> {
        self.cs_client.connect(addr.into(), server_name).await.map(
            |cs_client_connection: <CSClient as CSTraitClient>::Connection| {
                TypedCSTraitClientConnection {
                    cs_client_connection,
                    phantom_data: PhantomData,
                }
            },
        )
    }

    pub fn local_addr(&self) -> SocketAddr {
        self.cs_client.local_addr()
    }
}

#[derive(Derivative)]
#[derivative(Debug(bound = "CSClientConnection: Debug"))]
pub struct TypedCSTraitClientConnection<CSClientConnection, Request, Response> {
    cs_client_connection: CSClientConnection,
    phantom_data: PhantomData<(Request, Response)>,
}

impl<
        CSClientConnection: CSTraitClientConnection,
        Request: Serialize + Send + Sync,
        Response: for<'a> Deserialize<'a>,
    > TypedCSTraitClientConnection<CSClientConnection, Request, Response>
{
    pub async fn request(&mut self, request: impl Into<Request>) -> Result<Response> {
        self.cs_client_connection.request(request.into()).await
    }
}

#[derive(Derivative)]
#[derivative(Debug(bound = "CSServer: Debug"))]
pub struct TypedCSTraitServer<
    CSServer: CSTraitServer,
    Request: for<'a> Deserialize<'a> + Send,
    Response: Serialize + Send,
> {
    cs_server: CSServer,
    phantom_data: PhantomData<(Request, Response)>,
}

impl<
        CSServer: CSTraitServer,
        Request: for<'a> Deserialize<'a> + Send + 'static,
        Response: Serialize + Send + 'static,
    > TypedCSTraitServer<CSServer, Request, Response>
{
    pub fn start<
        S: Clone + Send + Sync + 'static,
        Fut: Future<Output = Option<Response>> + Send + 'static,
        F: Fn(S, Request) -> Fut + Send + Sync + Clone + 'static,
    >(
        self,
        shared: S,
        on_request: F,
        exit: oneshot::Receiver<()>,
    ) -> (SocketAddr, impl Future<Output = ()> + Send) {
        self.cs_server.start(shared, on_request, exit)
    }
}
