use std::future::Future;
use std::net::SocketAddr;
use std::pin::Pin;

use crate::cs::{json, CSConfig};
use anyhow::{anyhow, Result};
use async_trait::async_trait;
use bytes::Bytes;
use futures::FutureExt;
use reqwest::{
    header::{HeaderValue, CONTENT_TYPE},
    Client, StatusCode, Url,
};
use serde::{Deserialize, Serialize};
use tokio::sync::oneshot;
use warp::Filter;

use crate::cs::{CSTrait, CSTraitClient, CSTraitClientConnection, CSTraitServer};

#[derive(Debug, Clone, Copy, Default)]
pub struct HttpWarp;

impl CSTrait for HttpWarp {
    type Config = HttpWarpConfig;

    fn configure(self, _ca: Vec<u8>, _priv_key: Vec<u8>, _cert: Vec<u8>) -> Self::Config {
        HttpWarpConfig
    }
}

#[derive(Clone)]
pub struct HttpWarpConfig;

impl CSConfig for HttpWarpConfig {
    type Client = HttpWarpClient;

    type Server = HttpWarpServer;

    fn client(&self) -> Self::Client {
        let client = Client::builder()
            .no_gzip() // we want no compression since dummy payloads contain only zeros
            .no_brotli() // we want no compression since dummy payloads contain only zeros
            .no_deflate() // we want no compression since dummy payloads contain only zeros
            .no_proxy() // always use direct connection
            .http2_prior_knowledge()
            .build()
            .expect("static config always valid");

        HttpWarpClient { client }
    }

    fn server(&self, local_socket: SocketAddr) -> Self::Server {
        HttpWarpServer { local_socket }
    }
}

#[derive(Debug)]
pub struct HttpWarpClient {
    client: Client,
}

#[async_trait]
impl CSTraitClient for HttpWarpClient {
    type Connection = HttpWarpClientConnection;

    async fn connect(&self, addr: SocketAddr, _server_name: &str) -> Result<Self::Connection> {
        let mut url = Url::parse("http://somehost/").expect("valid url");
        url.set_ip_host(addr.ip()).expect("valid host");
        url.set_port(Some(addr.port())).expect("valid port");

        Ok(HttpWarpClientConnection {
            client: self.client.clone(),
            url,
        })
    }

    fn local_addr(&self) -> SocketAddr {
        unimplemented!()
    }
}

#[derive(Debug)]
pub struct HttpWarpClientConnection {
    url: Url,
    client: Client,
}

#[async_trait]
impl CSTraitClientConnection for HttpWarpClientConnection {
    async fn request<Request: Serialize + Send + Sync, Response: for<'a> Deserialize<'a>>(
        &mut self,
        request: Request,
    ) -> Result<Response> {
        let body = json::to_vec::<Request>(&request);

        let response = self
            .client
            .post(self.url.clone())
            .header(CONTENT_TYPE, HeaderValue::from_static("application/json"))
            .body(body)
            .send()
            .await?;

        let status_code = response.status();
        if status_code != StatusCode::OK {
            let body = response.text().await?;
            return Err(anyhow!(
                "wrong status code it was {:?} {}",
                status_code,
                body
            ));
        }

        let content_type = response.headers().get(CONTENT_TYPE);
        if content_type != Some(&HeaderValue::from_static("application/json")) {
            return Err(anyhow!("content type not json it was {:?}", content_type));
        }

        let response = response.bytes().await?;

        let response = json::from_slice::<Response>(&response)?;

        Ok(response)
    }
}

#[derive(Debug)]
pub struct HttpWarpServer {
    local_socket: SocketAddr,
}

impl CSTraitServer for HttpWarpServer {
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
        let send = warp::post()
            .and(warp::header::exact("content-type", "application/json"))
            .and(warp::body::bytes())
            .and_then(move |body: Bytes| {
                let shared = shared.clone();
                let on_request = on_request.clone();
                async move {
                    let input = match json::from_slice::<Request>(&body) {
                        Ok(input) => input,
                        Err(err) => {
                            tracing::warn!("request json body error: {}", err);
                            return Err(warp::reject::reject());
                        }
                    };
                    match on_request(shared, input).await {
                        Some(resp) => Ok(resp),
                        None => Err(warp::reject::reject()),
                    }
                }
            })
            .map(|o| {
                let mut res = warp::reply::Response::new(json::to_vec::<Response>(&o).into());
                res.headers_mut()
                    .insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
                res
            });

        let (addr, server) =
            warp::serve(send).bind_with_graceful_shutdown(self.local_socket, async {
                exit.await.ok();
            });

        let join_handle = tokio::spawn(server);

        (addr, Box::pin(join_handle.map(|_| ())))
    }
}
