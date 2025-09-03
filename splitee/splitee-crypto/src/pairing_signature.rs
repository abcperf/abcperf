use crate::math::{ECPoint, PrivateKey, PublicKey};

pub struct PairingSignature {}

impl From<ECPoint> for PairingSignature {
    fn from(_point: ECPoint) -> Self {
        unimplemented!()
    }
}

impl PairingSignature {
    pub fn sign(msg: &[u8], priv_key: &PrivateKey) -> Self {
        unimplemented!()
    }

    pub fn verify(msg: &[u8], pub_key: &PublicKey, sig_to_verify: &Self) -> bool {
        unimplemented!()
    }

    pub fn verify_irgendwas(
        base: &ECPoint,
        claimed: &ECPoint,
        group_gen: &ECPoint,
        verification_key: &ECPoint,
    ) -> bool {
        unimplemented!()
    }
}
