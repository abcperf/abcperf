use serde::{Deserialize, Serialize};
use sgx_types::types::{Ec256PublicKey, Ec256Signature};

/// Effectively `sgx_ec256_signature_t` with a `Serialize` and `Deserialize` implementation
#[derive(Copy, Clone, Debug, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub struct RawSignature {
    #[serde(with = "serde_bytes")]
    xy: [u8; 64],
}

impl AsRef<[u8]> for RawSignature {
    fn as_ref(&self) -> &[u8] {
        &self.xy
    }
}

impl From<Ec256Signature> for RawSignature {
    fn from(sig: Ec256Signature) -> Self {
        let mut vec = Vec::with_capacity(64);
        for e in [sig.x, sig.y].iter().flatten() {
            vec.extend_from_slice(&e.to_le_bytes());
        }
        Self {
            xy: vec.try_into().unwrap(),
        }
    }
}

macro_rules! u32_from_u8_array {
    ($xy:expr) => {{
        let [a1, a2, a3, a4, xy @ ..] = $xy;
        let a = u32::from_le_bytes([a1, a2, a3, a4]);
        (a, xy)
    }};
}

macro_rules! u32_array_from_u8_array {
    ($xy:expr) => {{
        let xy = $xy;
        let (a1, xy) = u32_from_u8_array!(xy);
        let (a2, xy) = u32_from_u8_array!(xy);
        let (a3, xy) = u32_from_u8_array!(xy);
        let (a4, xy) = u32_from_u8_array!(xy);
        let (a5, xy) = u32_from_u8_array!(xy);
        let (a6, xy) = u32_from_u8_array!(xy);
        let (a7, xy) = u32_from_u8_array!(xy);
        let (a8, xy) = u32_from_u8_array!(xy);
        let a = [a1, a2, a3, a4, a5, a6, a7, a8];
        (a, xy)
    }};
}

impl From<RawSignature> for Ec256Signature {
    fn from(sig: RawSignature) -> Self {
        let (x, y) = u32_array_from_u8_array!(sig.xy);
        let (y, []) = u32_array_from_u8_array!(y);

        Self { x, y }
    }
}

/// Effectively `sgx_ec256_public_t` with a `Serialize` and `Deserialize` implementation
#[derive(Copy, Clone, Debug, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub struct PubKey {
    pub gx: [u8; 32],
    pub gy: [u8; 32],
}

impl PubKey {
    pub fn as_bytes_for_cert(&self) -> [u8; 64] {
        let mut data = [0; 64];

        let mut pk_gx = self.gx;
        pk_gx.reverse();
        data[..32].clone_from_slice(&pk_gx);

        let mut pk_gy = self.gy;
        pk_gy.reverse();
        data[32..].clone_from_slice(&pk_gy);

        data
    }
}

impl From<Ec256PublicKey> for PubKey {
    fn from(key: Ec256PublicKey) -> Self {
        let Ec256PublicKey { gx, gy } = key;
        Self { gx, gy }
    }
}

impl From<PubKey> for Ec256PublicKey {
    fn from(key: PubKey) -> Self {
        let PubKey { gx, gy } = key;
        Self { gx, gy }
    }
}
