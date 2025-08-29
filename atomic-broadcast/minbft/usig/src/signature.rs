use std::{collections::HashMap, fmt::Debug, marker::PhantomData};

use derivative::Derivative;
use hashbar::Hashbar;
use rand::rngs::OsRng;
use serde::{Deserialize, Serialize};

use shared_ids::ReplicaId;
use signature::{SignatureEncoding, Signer, Verifier};
use trait_alias_macro::pub_trait_alias_macro;

use crate::{Count, Counter, SignHalf, Usig, UsigError, VerifyHalf};

pub_trait_alias_macro!(SignatureType = for<'a> Deserialize<'a> + Serialize + Clone + Debug + SignatureEncoding);

#[derive(Derivative, Deserialize, Serialize)]
#[serde(bound = "")]
#[derivative(Debug(bound = ""), Clone(bound = ""))]
pub struct Signature<S: SignatureType> {
    counter: u64,
    signature: S,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PubKey<V>(V);

impl<V: AsRef<[u8]>> Hashbar for PubKey<V> {
    fn hash<H: hashbar::Hasher>(&self, hasher: &mut H) {
        hasher.update(self.0.as_ref());
    }
}

impl<S: SignatureType> Hashbar for Signature<S> {
    fn hash<H: hashbar::Hasher>(&self, hasher: &mut H) {
        hasher.update(&self.counter.to_le_bytes());
        hasher.update(self.signature.to_bytes().as_ref());
    }
}

impl<S: SignatureType> Counter for Signature<S> {
    fn counter(&self) -> Count {
        Count(self.counter)
    }
}

#[derive(Derivative)]
#[derivative(Debug(bound = ""))]
pub struct UsigSignatureSignHalf<
    Q: SignatureType,
    S: Signer<Q> + Debug,
    V: Verifier<Q> + Clone + Debug + for<'a> Deserialize<'a> + Serialize,
> {
    counter: u64,
    private_key: S,
    public_key: V,
    phantom_data: PhantomData<Q>,
}

impl<
        Q: SignatureType,
        S: Signer<Q> + Debug,
        V: Verifier<Q> + Clone + Debug + for<'a> Deserialize<'a> + Serialize,
    > UsigSignatureSignHalf<Q, S, V>
{
    pub fn new(private_key: S, public_key: V) -> Self {
        Self {
            counter: 0,
            private_key,
            public_key,
            phantom_data: PhantomData,
        }
    }
}

impl<
        Q: SignatureType,
        S: Signer<Q> + Debug,
        V: Verifier<Q> + Clone + Debug + for<'a> Deserialize<'a> + Serialize,
    > SignHalf for UsigSignatureSignHalf<Q, S, V>
{
    type Signature = Signature<Q>;
    type Attestation = PubKey<V>;

    fn sign(&mut self, message: impl AsRef<[u8]>) -> Result<Self::Signature, UsigError> {
        let counter = self.counter;
        self.counter += 1;
        let mut data = Vec::<u8>::new();
        data.extend_from_slice(&counter.to_be_bytes());
        data.extend_from_slice(message.as_ref());
        let signature = self.private_key.sign(&data);
        Ok(Signature { counter, signature })
    }

    fn attest(&mut self) -> Result<Self::Attestation, UsigError> {
        Ok(PubKey(self.public_key.clone()))
    }
}

#[derive(Derivative)]
#[derivative(Debug(bound = ""), Default(bound = ""))]
pub struct UsigSignatureVerifyHalf<
    Q: SignatureType,
    V: Verifier<Q> + Clone + Debug + for<'a> Deserialize<'a> + Serialize,
> {
    other_keys: HashMap<ReplicaId, V>,
    phantom_data: PhantomData<Q>,
}

impl<Q: SignatureType, V: Verifier<Q> + Clone + Debug + for<'a> Deserialize<'a> + Serialize>
    VerifyHalf for UsigSignatureVerifyHalf<Q, V>
{
    type Signature = Signature<Q>;
    type Attestation = PubKey<V>;

    fn verify(
        &self,
        id: ReplicaId,
        message: impl AsRef<[u8]>,
        signature: &Self::Signature,
    ) -> Result<(), UsigError> {
        if let Some(key) = self.other_keys.get(&id) {
            let mut data = Vec::<u8>::new();
            data.extend_from_slice(&signature.counter.to_be_bytes());
            data.extend_from_slice(message.as_ref());

            key.verify(&data, &signature.signature)
                .is_ok()
                .then_some(())
                .ok_or(UsigError::InvalidSignature)
        } else {
            Err(UsigError::UnknownId(id))
        }
    }

    fn add_remote_party(&mut self, id: ReplicaId, attestation: Self::Attestation) -> bool {
        self.other_keys.insert(id, attestation.0);
        true
    }
}

#[derive(Derivative)]
#[derivative(Debug(bound = ""))]
pub struct UsigSignature<
    Q: SignatureType,
    S: Signer<Q> + Debug,
    V: Verifier<Q> + Clone + Debug + for<'a> Deserialize<'a> + Serialize,
> {
    sign_half: UsigSignatureSignHalf<Q, S, V>,
    verify_half: UsigSignatureVerifyHalf<Q, V>,
}

impl<
        Q: SignatureType,
        S: Signer<Q> + Debug,
        V: Verifier<Q> + Clone + Debug + for<'a> Deserialize<'a> + Serialize,
    > UsigSignature<Q, S, V>
{
    pub fn new(private_key: S, public_key: V) -> Self {
        Self {
            sign_half: UsigSignatureSignHalf::new(private_key, public_key),
            verify_half: UsigSignatureVerifyHalf::default(),
        }
    }
}

impl<
        Q: SignatureType,
        S: Signer<Q> + Debug,
        V: Verifier<Q> + Clone + Debug + for<'a> Deserialize<'a> + Serialize + Send + 'static,
    > Usig for UsigSignature<Q, S, V>
where
    Self: Default,
{
    type Signature = Signature<Q>;
    type Attestation = PubKey<V>;

    fn sign(&mut self, message: impl AsRef<[u8]>) -> Result<Self::Signature, UsigError> {
        self.sign_half.sign(message)
    }

    fn attest(&mut self) -> Result<Self::Attestation, UsigError> {
        self.sign_half.attest()
    }

    fn verify(
        &self,
        id: ReplicaId,
        message: impl AsRef<[u8]>,
        signature: &Self::Signature,
    ) -> Result<(), UsigError> {
        self.verify_half.verify(id, message, signature)
    }

    fn add_remote_party(&mut self, id: ReplicaId, attestation: Self::Attestation) -> bool {
        self.verify_half.add_remote_party(id, attestation)
    }

    type SignHalf = UsigSignatureSignHalf<Q, S, V>;
    type VerifyHalf = UsigSignatureVerifyHalf<Q, V>;

    fn split(self) -> (Self::SignHalf, Self::VerifyHalf) {
        (self.sign_half, self.verify_half)
    }

    fn new() -> Self
    where
        Self: Sized,
    {
        Self::default()
    }
}

impl Default
    for UsigSignature<
        ed25519_dalek::Signature,
        ed25519_dalek::SigningKey,
        ed25519_dalek::VerifyingKey,
    >
{
    fn default() -> Self {
        new_ed25519()
    }
}

pub type UsigEd25519 =
    UsigSignature<ed25519_dalek::Signature, ed25519_dalek::SigningKey, ed25519_dalek::VerifyingKey>;

pub fn new_ed25519() -> UsigEd25519 {
    let keypair = ed25519_dalek::SigningKey::generate(&mut OsRng);
    let public_key = keypair.verifying_key();
    UsigSignature::new(keypair, public_key)
}

#[cfg(test)]
mod tests {
    use super::new_ed25519;
    use crate as usig;
    use crate::tests;

    tests!(new_ed25519());
}
