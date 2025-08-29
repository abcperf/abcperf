use std::fmt::Debug;

use client::{ApplicationClientEmulator, SMRClientFactory};
use serde::{de::DeserializeOwned, Serialize};
use server::ApplicationServer;
use trait_alias_macro::pub_trait_alias_macro;

use crate::atomic_broadcast::ABCTransaction;

pub mod client;
pub mod server;
pub mod smr_client;

pub_trait_alias_macro!(ApplicationRequest = Debug + Send + Sync + Serialize + DeserializeOwned + Clone + 'static);
pub_trait_alias_macro!(ApplicationResponse = Debug + Send + Sync + Serialize + DeserializeOwned + Eq + 'static);
pub_trait_alias_macro!(ApplicationConfig = Debug + Serialize + DeserializeOwned + Clone);
pub_trait_alias_macro!(ApplicationReplicaMessage = Debug + Send + Serialize + DeserializeOwned);

pub trait Application: 'static {
    type Request: ApplicationRequest;
    type Response: ApplicationResponse;
    type Transaction: ABCTransaction;
    type ReplicaMessage: ApplicationReplicaMessage;
    type Config: ApplicationConfig;
    type ClientEmulator<CF: SMRClientFactory<Self::Request, Self::Response>>: ApplicationClientEmulator<
        CF,
        Request = Self::Request,
        Response = Self::Response,
        Config = Self::Config,
    >;
    type Server: ApplicationServer<
        Request = Self::Request,
        Response = Self::Response,
        Transaction = Self::Transaction,
        ReplicaMessage = Self::ReplicaMessage,
        Config = Self::Config,
    >;
}
