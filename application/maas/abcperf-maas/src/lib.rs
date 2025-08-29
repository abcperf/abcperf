use std::num::NonZeroU64;

use abcperf::{
    application::{client::SMRClientFactory, Application},
    config::DistributionConfig,
};
use client::MaasApplicationClient;
use payload::{EncrypteTransaction, Request, Response};
use serde::{Deserialize, Serialize};
use server::{MaasReplicaMessage, MaasServer};

mod client;
mod payload;
mod server;

#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct Config {
    maximum_client_count: NonZeroU64,
    distribution: DistributionConfig,
}

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

pub struct MaasApplication {}

impl Application for MaasApplication {
    type Request = Request;

    type Response = Response;

    type Transaction = EncrypteTransaction;

    type ReplicaMessage = MaasReplicaMessage;

    type Config = Config;

    type ClientEmulator<CF: SMRClientFactory<Self::Request, Self::Response>> =
        MaasApplicationClient;

    type Server = MaasServer;
}

#[cfg(test)]
mod tests {
    use sgx_crypto::aes::gcm::{Aad, AesGcm, Nonce};
    use sgx_types::types::Key128bit;

    static AES_KEY: Key128bit = [0; 16];

    #[test]
    fn result_bincode() {
        let payload: Result<(), ()> = Ok(());
        let payload_bytes = bincode::serialize(&payload).unwrap();
        let reference: &[u8] = &[0, 0, 0, 0];
        assert_eq!(payload_bytes.as_slice(), reference);

        let payload: Result<(), ()> = Err(());
        let payload_bytes = bincode::serialize(&payload).unwrap();
        let reference: &[u8] = &[1, 0, 0, 0];
        assert_eq!(payload_bytes.as_slice(), reference);
    }

    #[test]
    fn result_encrypt() {
        let clear_text: &[u8] = &[0, 0, 0, 0];

        let mut encrypted_bytes = vec![0; clear_text.len()];

        let mac = AesGcm::new(&AES_KEY, Nonce::zeroed(), Aad::from(&[]))
            .unwrap()
            .encrypt(clear_text, encrypted_bytes.as_mut_slice())
            .unwrap();

        let reference: &[u8] = &[3, 136, 218, 206];
        assert_eq!(encrypted_bytes.as_slice(), reference);

        let reference: &[u8] = &[
            166, 137, 108, 53, 93, 28, 41, 112, 155, 157, 12, 73, 2, 35, 127, 27,
        ];
        assert_eq!(mac.as_slice(), reference);
    }
}
