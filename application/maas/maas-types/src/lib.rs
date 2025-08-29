#![no_std]
use serde::{Deserialize, Serialize};

use sgx_types::types::Key128bit;

pub type Pseu = u128; // TODO maybe UUID?
pub type Provider = u64;
pub type Secret = u64; //Box<[u8]>;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClientRequest {
    pub inner: InnerClientRequest,
    pub response_key: Key128bit,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum InnerClientRequest {
    PreRegister,
    Register(RegisterMsg),
    Checkin(CheckinMsg),
    Checkout(CheckoutMsg),
    // TripHistory, TODO
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegisterMsg {
    pub cack: Cack,
    pub provider: Provider,
    pub short_term_secret: Secret,
    pub long_term_secret: Secret,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Cack {
    pub pseu: Pseu,
    pub sig: (), // TODO type
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CheckinMsg {
    pub interaction: Interaction,
    pub long_term_secret: Secret,
    pub next_short_term_secret: Secret,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CheckoutMsg {
    pub interaction: Interaction,
    pub long_term_secret: Secret,
    pub next_short_term_secret: Secret,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Interaction {
    // TODO whats pk_vk?
    pub short_term_secret: Secret,
    pub timestamp: (),
    pub location: (),
    pub sig: (),
}
