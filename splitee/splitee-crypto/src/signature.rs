use ed25519_dalek::ed25519::signature::Signer;

use crate::{
    hash::HashValue,
    math::{PrivateKey, PublicKey},
};

pub struct Signature {
    signature: ed25519_dalek::Signature,
}

impl Signature {
    pub fn sign(hash: &HashValue, priv_key: &PrivateKey) -> Self {
        let signature = priv_key.inner.sign(hash.as_ref());
        Self { signature }
    }

    pub fn verify(&self, hash: &HashValue, pub_key: &PublicKey) -> bool {
        pub_key
            .inner
            .verify_strict(hash.as_ref(), &self.signature)
            .is_ok()
    }
}
