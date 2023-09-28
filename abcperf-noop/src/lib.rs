use std::{collections::HashMap, time::Duration};

use abcperf_client_proxy::Signature;
use abcperf_generic_client::cs::{typed::TypedCSTrait, CSTrait};
use abcperf_generic_client::{proxy::ConfigTrait, CustomClientConfig};
use payload::{ReqPayload, RespPayload};
use serde::{Deserialize, Serialize};
use server::NoopServer;
use shared_ids::ClientId;

mod payload;
mod server;

#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct MyClientConfig {
    request_payload: u64,
    response_payload: u64,

    mode: CustomClientConfig,

    fallback_timeout: f64,
}

impl ConfigTrait for MyClientConfig {
    fn fallback_timeout(&self) -> Duration {
        Duration::from_secs_f64(self.fallback_timeout)
    }
}

impl From<MyClientConfig> for HashMap<String, String> {
    fn from(config: MyClientConfig) -> Self {
        let MyClientConfig {
            request_payload,
            response_payload,
            mode,
            fallback_timeout,
        } = config;

        let mut map: Self = mode.into();
        assert!(map
            .insert("request_payload".to_string(), request_payload.to_string())
            .is_none());
        assert!(map
            .insert("response_payload".to_string(), response_payload.to_string())
            .is_none());
        assert!(map
            .insert("fallback_timeout".to_string(), fallback_timeout.to_string())
            .is_none());
        map
    }
}

impl AsRef<CustomClientConfig> for MyClientConfig {
    fn as_ref(&self) -> &CustomClientConfig {
        &self.mode
    }
}

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

pub type NoopServerProxy<CS> = NoopServer<CS, ((ClientId, ReqPayload), Box<[Signature]>)>;
pub fn server<CS: CSTrait>(cs: TypedCSTrait<CS, ReqPayload, RespPayload>) -> NoopServerProxy<CS> {
    NoopServerProxy::new(cs)
}

pub type NoopServerNoProxy<CS> = NoopServer<CS, (ClientId, ReqPayload)>;
pub fn server_no_proxy<CS: CSTrait>(
    cs: TypedCSTrait<CS, ReqPayload, RespPayload>,
) -> NoopServerNoProxy<CS> {
    NoopServerNoProxy::new(cs)
}

pub type ClientEmulatorProxy<CS> =
    abcperf_generic_client::proxy::GenericClient<ReqPayload, RespPayload, MyClientConfig, CS>;
pub fn client_emulator<CS: CSTrait>(
    cs: TypedCSTrait<CS, ReqPayload, RespPayload>,
) -> ClientEmulatorProxy<CS> {
    ClientEmulatorProxy::new(cs)
}

pub type ClientEmulatorNoProxy<CS> =
    abcperf_generic_client::no_proxy::GenericClient<ReqPayload, RespPayload, MyClientConfig, CS>;
pub fn client_emulator_no_proxy<CS: CSTrait>(
    cs: TypedCSTrait<CS, ReqPayload, RespPayload>,
) -> ClientEmulatorNoProxy<CS> {
    ClientEmulatorNoProxy::new(cs)
}
