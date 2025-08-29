use std::collections::HashSet;

use hashbar::Hashbar;
use serde::{Deserialize, Serialize};
use shared_ids::ReplicaId;

use crate::{Count, Counter, SignHalf, Usig, UsigError, VerifyHalf};

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Signature(u64);

impl Hashbar for Signature {
    fn hash<H: hashbar::Hasher>(&self, hasher: &mut H) {
        hasher.update(&self.0.to_le_bytes());
    }
}

impl Signature {
    pub fn fake(counter: u64) -> Self {
        Self(counter)
    }
}

impl Counter for Signature {
    fn counter(&self) -> Count {
        Count(self.0)
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, Hash)]
pub struct DummyAttestation;

impl Hashbar for DummyAttestation {
    fn hash<H: hashbar::Hasher>(&self, hasher: &mut H) {
        hasher.update(&[0]);
    }
}

#[derive(Default, Debug)]
pub struct UsigNoOpSignHalf {
    counter: u64,
}

impl SignHalf for UsigNoOpSignHalf {
    type Signature = Signature;
    type Attestation = DummyAttestation;

    fn sign(&mut self, message: impl AsRef<[u8]>) -> Result<Self::Signature, UsigError> {
        let _ = message.as_ref();
        let counter = self.counter;
        self.counter += 1;
        Ok(Signature(counter))
    }

    fn attest(&mut self) -> Result<Self::Attestation, UsigError> {
        Ok(DummyAttestation)
    }
}

#[derive(Default, Debug)]
pub struct UsigNoOpVerifyHalf {
    ids: HashSet<ReplicaId>,
}

impl VerifyHalf for UsigNoOpVerifyHalf {
    type Signature = Signature;
    type Attestation = DummyAttestation;

    fn verify(
        &self,
        id: ReplicaId,
        message: impl AsRef<[u8]>,
        _signature: &Self::Signature,
    ) -> Result<(), UsigError> {
        if self.ids.contains(&id) {
            let _ = message.as_ref();
            Ok(())
        } else {
            Err(UsigError::UnknownId(id))
        }
    }

    fn add_remote_party(&mut self, id: ReplicaId, _attestation: Self::Attestation) -> bool {
        self.ids.insert(id);
        true
    }
}

#[derive(Default, Debug)]
pub struct UsigNoOp {
    sign_half: UsigNoOpSignHalf,
    verify_half: UsigNoOpVerifyHalf,
}

impl Usig for UsigNoOp {
    type Signature = Signature;
    type Attestation = DummyAttestation;

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

    type SignHalf = UsigNoOpSignHalf;
    type VerifyHalf = UsigNoOpVerifyHalf;

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

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;

    type Bin = &'static [u8];

    const MESSAGE_1: Bin = b"message one";
    const MESSAGE_2: Bin = b"message two";
    const ID: ReplicaId = ReplicaId::from_u64(0);

    fn new_usig() -> UsigNoOp {
        UsigNoOp::default()
    }

    fn new_usig_individual() -> (UsigNoOpSignHalf, UsigNoOpVerifyHalf) {
        (UsigNoOpSignHalf::default(), UsigNoOpVerifyHalf::default())
    }

    #[test]
    fn as_ref() {
        struct Input<F: Fn()>(F);
        impl<F: Fn()> AsRef<[u8]> for Input<F> {
            fn as_ref(&self) -> &[u8] {
                self.0();
                &[]
            }
        }

        let into_called = AtomicUsize::new(0);
        let called = || {
            into_called.fetch_add(1, Ordering::SeqCst);
        };

        let mut usig = new_usig();
        usig.attest().unwrap();
        assert!(usig.add_remote_party(ID, DummyAttestation));

        let signature = usig.sign(Input(called)).unwrap();
        assert_eq!(into_called.load(Ordering::SeqCst), 1);

        assert!(usig.verify(ID, Input(called), &signature).is_ok());
        assert_eq!(into_called.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn valid() {
        let mut usig = new_usig();
        usig.attest().unwrap();
        assert!(usig.add_remote_party(ID, DummyAttestation));
        let signature = usig.sign(MESSAGE_1).unwrap();
        assert!(usig.verify(ID, MESSAGE_1, &signature).is_ok());
    }

    #[test]
    fn double_sig() {
        let mut usig = new_usig();
        usig.attest().unwrap();
        assert!(usig.add_remote_party(ID, DummyAttestation));
        let signature_1 = usig.sign(MESSAGE_1).unwrap();
        let signature_2 = usig.sign(MESSAGE_1).unwrap();
        assert!(usig.verify(ID, MESSAGE_1, &signature_1).is_ok());
        assert!(usig.verify(ID, MESSAGE_1, &signature_2).is_ok());
        assert_eq!(signature_1.counter() + 1, signature_2.counter());
    }

    #[test]
    fn valid_iteration() {
        let mut usig = new_usig();
        usig.attest().unwrap();
        assert!(usig.add_remote_party(ID, DummyAttestation));
        let initial_count = usig.sign(MESSAGE_1).unwrap().counter();
        let mut prev_counter = initial_count;
        for _ in 0..100 {
            let counter = usig.sign(MESSAGE_1).unwrap().counter();
            assert_eq!(prev_counter + 1, counter);
            prev_counter = counter;
        }
        let signature = usig.sign(MESSAGE_2).unwrap();
        assert_eq!(initial_count + 101, signature.counter());
        assert!(usig.verify(ID, MESSAGE_2, &signature).is_ok());
    }

    #[test]
    fn attest_after() {
        let mut usig = new_usig();
        let signature = usig.sign(MESSAGE_1).unwrap();
        usig.attest().unwrap();
        assert!(usig.add_remote_party(ID, DummyAttestation));
        assert!(usig.verify(ID, MESSAGE_1, &signature).is_ok());
    }

    #[test]
    fn no_id() {
        let mut usig = new_usig();
        let signature = usig.sign(MESSAGE_1).unwrap();
        assert!(matches!(
            usig.verify(ID, MESSAGE_1, &signature),
            Err(UsigError::UnknownId(ID))
        ));
    }

    #[test]
    fn no_id_1() {
        let mut usig = new_usig();
        let signature = usig.sign(MESSAGE_1).unwrap();
        assert!(matches!(
            usig.verify(ReplicaId::from_u64(1), MESSAGE_1, &signature),
            Err(UsigError::UnknownId(id)) if id == ReplicaId::from_u64(1)
        ));
    }

    #[test]
    fn wrong_id() {
        let mut usig = new_usig();
        usig.attest().unwrap();
        assert!(usig.add_remote_party(ReplicaId::from_u64(1), DummyAttestation));
        let signature = usig.sign(MESSAGE_1).unwrap();
        assert!(usig
            .verify(ReplicaId::from_u64(1), MESSAGE_1, &signature)
            .is_ok());
        assert!(matches!(
            usig.verify(ReplicaId::from_u64(0), MESSAGE_1, &signature),
            Err(UsigError::UnknownId(id)) if id == ReplicaId::from_u64(0)
        ));
    }

    #[test]
    fn as_ref_split() {
        struct Input<F: Fn()>(F);
        impl<F: Fn()> AsRef<[u8]> for Input<F> {
            fn as_ref(&self) -> &[u8] {
                self.0();
                &[]
            }
        }

        let into_called = AtomicUsize::new(0);
        let called = || {
            into_called.fetch_add(1, Ordering::SeqCst);
        };

        let (mut sign, mut verify) = new_usig().split();
        sign.attest().unwrap();
        assert!(verify.add_remote_party(ID, DummyAttestation));

        let signature = sign.sign(Input(called)).unwrap();
        assert_eq!(into_called.load(Ordering::SeqCst), 1);

        assert!(verify.verify(ID, Input(called), &signature).is_ok());
        assert_eq!(into_called.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn valid_split() {
        let (mut sign, mut verify) = new_usig().split();
        sign.attest().unwrap();
        assert!(verify.add_remote_party(ID, DummyAttestation));
        let signature = sign.sign(MESSAGE_1).unwrap();
        assert!(verify.verify(ID, MESSAGE_1, &signature).is_ok());
    }

    #[test]
    fn double_sig_split() {
        let (mut sign, mut verify) = new_usig().split();
        sign.attest().unwrap();
        assert!(verify.add_remote_party(ID, DummyAttestation));
        let signature_1 = sign.sign(MESSAGE_1).unwrap();
        let signature_2 = sign.sign(MESSAGE_1).unwrap();
        assert!(verify.verify(ID, MESSAGE_1, &signature_1).is_ok());
        assert!(verify.verify(ID, MESSAGE_1, &signature_2).is_ok());
        assert_eq!(signature_1.counter() + 1, signature_2.counter());
    }

    #[test]
    fn valid_iteration_split() {
        let (mut sign, mut verify) = new_usig().split();
        sign.attest().unwrap();
        assert!(verify.add_remote_party(ID, DummyAttestation));
        let initial_count = sign.sign(MESSAGE_1).unwrap().counter();
        let mut prev_counter = initial_count;
        for _ in 0..100 {
            let counter = sign.sign(MESSAGE_1).unwrap().counter();
            assert_eq!(prev_counter + 1, counter);
            prev_counter = counter;
        }
        let signature = sign.sign(MESSAGE_2).unwrap();
        assert_eq!(initial_count + 101, signature.counter());
        assert!(verify.verify(ID, MESSAGE_2, &signature).is_ok());
    }

    #[test]
    fn attest_after_split() {
        let (mut sign, mut verify) = new_usig().split();
        let signature = sign.sign(MESSAGE_1).unwrap();
        sign.attest().unwrap();
        assert!(verify.add_remote_party(ID, DummyAttestation));
        assert!(verify.verify(ID, MESSAGE_1, &signature).is_ok());
    }

    #[test]
    fn no_id_split() {
        let (mut sign, verify) = new_usig().split();
        let signature = sign.sign(MESSAGE_1).unwrap();
        assert!(matches!(
            verify.verify(ID, MESSAGE_1, &signature),
            Err(UsigError::UnknownId(ID))
        ));
    }

    #[test]
    fn no_id_1_split() {
        let (mut sign, verify) = new_usig().split();
        let signature = sign.sign(MESSAGE_1).unwrap();
        assert!(matches!(
            verify.verify(ReplicaId::from_u64(1), MESSAGE_1, &signature),
            Err(UsigError::UnknownId(id)) if id == ReplicaId::from_u64(1)
        ));
    }

    #[test]
    fn wrong_id_split() {
        let (mut sign, mut verify) = new_usig().split();
        sign.attest().unwrap();
        assert!(verify.add_remote_party(ReplicaId::from_u64(1), DummyAttestation));
        let signature = sign.sign(MESSAGE_1).unwrap();
        assert!(verify
            .verify(ReplicaId::from_u64(1), MESSAGE_1, &signature)
            .is_ok());
        assert!(matches!(
            verify.verify(ReplicaId::from_u64(0), MESSAGE_1, &signature),
            Err(UsigError::UnknownId(id)) if id == ReplicaId::from_u64(0)
        ));
    }

    #[test]
    fn as_ref_individual() {
        struct Input<F: Fn()>(F);
        impl<F: Fn()> AsRef<[u8]> for Input<F> {
            fn as_ref(&self) -> &[u8] {
                self.0();
                &[]
            }
        }

        let into_called = AtomicUsize::new(0);
        let called = || {
            into_called.fetch_add(1, Ordering::SeqCst);
        };

        let (mut sign, mut verify) = new_usig_individual();
        sign.attest().unwrap();
        assert!(verify.add_remote_party(ID, DummyAttestation));

        let signature = sign.sign(Input(called)).unwrap();
        assert_eq!(into_called.load(Ordering::SeqCst), 1);

        assert!(verify.verify(ID, Input(called), &signature).is_ok());
        assert_eq!(into_called.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn valid_individual() {
        let (mut sign, mut verify) = new_usig_individual();
        sign.attest().unwrap();
        assert!(verify.add_remote_party(ID, DummyAttestation));
        let signature = sign.sign(MESSAGE_1).unwrap();
        assert!(verify.verify(ID, MESSAGE_1, &signature).is_ok());
    }

    #[test]
    fn double_sig_individual() {
        let (mut sign, mut verify) = new_usig_individual();
        sign.attest().unwrap();
        assert!(verify.add_remote_party(ID, DummyAttestation));
        let signature_1 = sign.sign(MESSAGE_1).unwrap();
        let signature_2 = sign.sign(MESSAGE_1).unwrap();
        assert!(verify.verify(ID, MESSAGE_1, &signature_1).is_ok());
        assert!(verify.verify(ID, MESSAGE_1, &signature_2).is_ok());
        assert_eq!(signature_1.counter() + 1, signature_2.counter());
    }

    #[test]
    fn valid_iteration_individual() {
        let (mut sign, mut verify) = new_usig_individual();
        sign.attest().unwrap();
        assert!(verify.add_remote_party(ID, DummyAttestation));
        let initial_count = sign.sign(MESSAGE_1).unwrap().counter();
        let mut prev_counter = initial_count;
        for _ in 0..100 {
            let counter = sign.sign(MESSAGE_1).unwrap().counter();
            assert_eq!(prev_counter + 1, counter);
            prev_counter = counter;
        }
        let signature = sign.sign(MESSAGE_2).unwrap();
        assert_eq!(initial_count + 101, signature.counter());
        assert!(verify.verify(ID, MESSAGE_2, &signature).is_ok());
    }

    #[test]
    fn attest_after_individual() {
        let (mut sign, mut verify) = new_usig_individual();
        let signature = sign.sign(MESSAGE_1).unwrap();
        sign.attest().unwrap();
        assert!(verify.add_remote_party(ID, DummyAttestation));
        assert!(verify.verify(ID, MESSAGE_1, &signature).is_ok());
    }

    #[test]
    fn no_id_individual() {
        let (mut sign, verify) = new_usig_individual();
        let signature = sign.sign(MESSAGE_1).unwrap();
        assert!(matches!(
            verify.verify(ID, MESSAGE_1, &signature),
            Err(UsigError::UnknownId(ID))
        ));
    }

    #[test]
    fn no_id_1_individual() {
        let (mut sign, verify) = new_usig_individual();
        let signature = sign.sign(MESSAGE_1).unwrap();
        assert!(matches!(
            verify.verify(ReplicaId::from_u64(1), MESSAGE_1, &signature),
            Err(UsigError::UnknownId(id)) if id == ReplicaId::from_u64(1)
        ));
    }

    #[test]
    fn wrong_id_individual() {
        let (mut sign, mut verify) = new_usig_individual();
        sign.attest().unwrap();
        assert!(verify.add_remote_party(ReplicaId::from_u64(1), DummyAttestation));
        let signature = sign.sign(MESSAGE_1).unwrap();
        assert!(verify
            .verify(ReplicaId::from_u64(1), MESSAGE_1, &signature)
            .is_ok());
        assert!(matches!(
            verify.verify(ReplicaId::from_u64(0), MESSAGE_1, &signature),
            Err(UsigError::UnknownId(id)) if id == ReplicaId::from_u64(0)
        ));
    }
}
