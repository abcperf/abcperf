use crate::{
    vertex::{SignableVertex, SignedVertex},
    Round,
};
use hashbar::{Hashbar, Hasher};
use serde::{Deserialize, Serialize};
use shared_ids::ReplicaId;
use thiserror::Error;
use uuid::Uuid;

use super::{
    Enclave, EnclaveAutomata, EnclaveAutomataError, EnclaveAutomataExport, EnclaveTrusted,
    InitHandshakeResult,
};

#[derive(Debug, Hash, PartialEq, Eq, Clone, Copy, Serialize, Deserialize)]
pub struct EnclaveId(Uuid);

impl EnclaveId {
    pub fn random() -> Self {
        EnclaveId(Uuid::new_v4())
    }
}

impl Hashbar for EnclaveId {
    fn hash<H: Hasher>(&self, hasher: &mut H) {
        hasher.update(self.0.as_bytes());
    }
}

#[derive(Debug)]
struct SimpleEnclaveTrusted {
    enclave_id: EnclaveId,
}

impl EnclaveTrusted for SimpleEnclaveTrusted {
    type Attestation = EnclaveId;

    type Error = SimpleEnclaveError;

    type Handshake = [u8; 32];

    type Signature = EnclaveSignature;

    fn gen_seed_share(&self) -> Result<[u8; 32], Self::Error> {
        let mut dest = [0; 32];
        getrandom::getrandom(&mut dest)?;
        Ok(dest)
    }

    fn seal_seed_share(
        &self,
        _attestation: &Self::Attestation,
        share: [u8; 32],
    ) -> Result<Self::Handshake, Self::Error> {
        Ok(share)
    }

    fn sign(&mut self, _message_hash: &[u8], count: u64) -> Result<Self::Signature, Self::Error> {
        Ok(EnclaveSignature(self.enclave_id, count))
    }

    fn verify(
        attestation: &Self::Attestation,
        counter: u64,
        _message_hash: &[u8],
        signature: &Self::Signature,
    ) -> Result<bool, Self::Error> {
        Ok(counter == signature.1 && *attestation == signature.0)
    }

    fn unseal_seed_share(
        &self,
        handshake: &Self::Handshake,
        _attestation: &Self::Attestation,
    ) -> Result<[u8; 32], Self::Error> {
        Ok(*handshake)
    }

    fn get_attestation_certificate(&mut self) -> Result<Self::Attestation, Self::Error> {
        Ok(self.enclave_id)
    }
}

#[derive(Debug)]
pub struct SimpleEnclave {
    automata: EnclaveAutomata<SimpleEnclaveTrusted>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Hash, PartialEq, Eq)]
pub struct EnclaveSignature(EnclaveId, u64);

impl EnclaveSignature {
    #[cfg(test)]
    fn dummy() -> Self {
        Self(EnclaveId::random(), 0)
    }
}

impl Hashbar for EnclaveSignature {
    fn hash<H: Hasher>(&self, hasher: &mut H) {
        self.0.hash(hasher);
        hasher.update(&self.1.to_le_bytes());
    }
}

#[derive(Debug, Error)]
pub enum SimpleEnclaveError {
    #[error("GetRandom error: {0}")]
    Random(#[from] getrandom::Error),
}

impl Enclave for SimpleEnclave {
    type Attestation = EnclaveId;
    type Handshake = [u8; 32];
    type EnclaveError = EnclaveAutomataError<SimpleEnclaveError>;
    type Signature = EnclaveSignature;
    type Export = EnclaveAutomataExport<EnclaveId>;

    fn new(n: u64) -> Self {
        SimpleEnclave {
            automata: EnclaveAutomata::new(
                n,
                SimpleEnclaveTrusted {
                    enclave_id: EnclaveId::random(),
                },
            ),
        }
    }

    fn init(&mut self) -> Result<(), Self::EnclaveError> {
        self.automata.init()
    }

    fn get_n(&self) -> u64 {
        self.automata.get_n()
    }

    fn get_attestation_certificate(&mut self) -> Result<Self::Attestation, Self::EnclaveError> {
        self.automata.get_attestation_certificate()
    }

    fn verify_attestation(
        &self,
        _attestation: &Self::Attestation,
    ) -> Result<bool, Self::EnclaveError> {
        Ok(true)
    }

    fn init_peer_handshake(
        &mut self,
        peer_id: shared_ids::ReplicaId,
        attestation: Self::Attestation,
    ) -> InitHandshakeResult<Self::Handshake, Self::EnclaveError> {
        self.automata.init_peer_handshake(peer_id, attestation)
    }

    fn export(&self) -> Result<Self::Export, Self::EnclaveError> {
        self.automata.export()
    }

    fn recover(&mut self, export: Self::Export) -> Result<(), Self::EnclaveError> {
        self.automata.recover(export)
    }

    fn replace_peer(
        &mut self,
        peer_id: ReplicaId,
        attestation: &Self::Attestation,
        max_round: Round,
    ) -> Result<bool, Self::EnclaveError> {
        self.automata.replace_peer(peer_id, attestation, max_round)
    }

    fn complete_peer_handshake(
        &mut self,
        peer_id: shared_ids::ReplicaId,
        handshake: &Self::Handshake,
    ) -> super::HandshakeResult<Self::EnclaveError> {
        self.automata.complete_peer_handshake(peer_id, handshake)
    }

    fn sign(
        &mut self,
        message: SignableVertex,
    ) -> Result<(Self::Signature, u64), Self::EnclaveError> {
        self.automata.sign(message.to_hash().as_ref())
    }

    fn verify(
        &self,
        peer_id: ReplicaId,
        counter: u64,
        message: SignableVertex,
        signature: &Self::Signature,
    ) -> Result<bool, Self::EnclaveError> {
        self.automata.verify(peer_id, counter, message, signature)
    }

    fn is_ready(&self) -> bool {
        self.automata.is_ready()
    }

    fn toss(
        &mut self,
        proof: Vec<SignedVertex<Self::Signature>>,
    ) -> Result<ReplicaId, Self::EnclaveError> {
        self.automata.toss(proof)
    }
}

#[cfg(test)]
mod tests {
    use std::ops::Deref;

    use hashbar::Hashbar;
    use id_set::ReplicaSet;
    use shared_ids::ReplicaId;

    use crate::vertex::Vertex;
    use crate::{
        enclave::simple::{EnclaveAutomataError, EnclaveId},
        enclave::{Enclave, HandshakeResult, InitHandshakeResult},
    };

    use super::{EnclaveSignature, SimpleEnclave};

    struct HashableDummy;
    impl Hashbar for HashableDummy {
        fn hash<H: blake2::digest::Update>(&self, _hasher: &mut H) {}
    }

    fn make_vertex(counter: u64, replica: u64) -> Vertex<HashableDummy, EnclaveSignature> {
        Vertex::new(
            counter.into(),
            ReplicaId::from_u64(replica),
            ReplicaSet::default(),
            Vec::new(),
            |_| Ok::<_, ()>((EnclaveSignature::dummy(), 0)),
        )
        .unwrap()
    }

    fn make_vertex_with_enclave(
        counter: u64,
        replica: u64,
        enclave: &mut SimpleEnclave,
    ) -> Vertex<HashableDummy, EnclaveSignature> {
        Vertex::new(
            counter.into(),
            ReplicaId::from_u64(replica),
            ReplicaSet::default(),
            Vec::new(),
            |sig| enclave.sign(sig.clone()),
        )
        .unwrap()
    }

    #[test]
    fn incomplete_setup() {
        let mut enclave = SimpleEnclave::new(3);
        assert_eq!(enclave.automata.quorum, 2);
        assert_eq!(enclave.automata.seed_share, None);
        assert!(enclave.init().is_ok());
        assert_ne!(enclave.automata.seed_share, None);
        assert!(enclave.get_attestation_certificate().is_ok());
        assert!(matches!(
            enclave.init_peer_handshake(ReplicaId::from_u64(3), EnclaveId::random()),
            InitHandshakeResult::EnclaveError(EnclaveAutomataError::InvalidPeerId)
        ));
        assert!(matches!(
            enclave.complete_peer_handshake(ReplicaId::from_u64(1), &[0; 32]),
            HandshakeResult::MissingAttestation
        ));
        assert!(matches!(
            enclave.init_peer_handshake(ReplicaId::from_u64(1), EnclaveId::random()),
            InitHandshakeResult::Ok(_)
        ));
        assert!(matches!(
            enclave.init_peer_handshake(ReplicaId::from_u64(1), EnclaveId::random()),
            InitHandshakeResult::DuplicateHello
        ));
        assert!(matches!(
            enclave.complete_peer_handshake(ReplicaId::from_u64(3), &[0; 32]),
            HandshakeResult::EnclaveError(EnclaveAutomataError::InvalidPeerId)
        ));
        assert_eq!(enclave.automata.seed, [0; 32]);
        assert!(matches!(
            enclave.complete_peer_handshake(
                ReplicaId::from_u64(1),
                &enclave.automata.seed_share.unwrap()
            ),
            HandshakeResult::Ok
        ));
        assert_eq!(enclave.automata.seed_share, Some(enclave.automata.seed));
        assert!(matches!(
            enclave.init_peer_handshake(ReplicaId::from_u64(2), EnclaveId::random()),
            InitHandshakeResult::Ok(_)
        ));
        assert!(matches!(
            enclave.complete_peer_handshake(
                ReplicaId::from_u64(2),
                &enclave.automata.seed_share.unwrap().clone()
            ),
            HandshakeResult::Ok
        ));
        assert_eq!(enclave.automata.seed, [0; 32]);
        assert!(matches!(
            enclave.complete_peer_handshake(ReplicaId::from_u64(1), &[0; 32]),
            HandshakeResult::DuplicateReply
        ));
        assert!(matches!(
            enclave.sign(make_vertex(0, 0).deref().deref().clone()),
            Err(EnclaveAutomataError::NotReady)
        ));
        assert!(matches!(
            enclave.verify(
                ReplicaId::from_u64(0),
                1,
                make_vertex(0, 0).deref().deref().clone(),
                &EnclaveSignature::dummy()
            ),
            Err(EnclaveAutomataError::NotReady)
        ));
        assert!(matches!(
            enclave.verify(
                ReplicaId::from_u64(3),
                1,
                make_vertex(0, 0).deref().deref().clone(),
                &EnclaveSignature::dummy()
            ),
            Err(EnclaveAutomataError::InvalidPeerId)
        ));
    }

    #[test]
    #[should_panic]
    fn handshake_wihout_init() {
        let mut enclave = SimpleEnclave::new(3);
        enclave.init_peer_handshake(ReplicaId::from_u64(1), EnclaveId::random());
    }

    #[test]
    #[should_panic]
    fn invalid_peer_num() {
        let _ = SimpleEnclave::new(2);
    }

    fn instances(n: u64) -> Vec<SimpleEnclave> {
        let mut instances = Vec::with_capacity(n as usize);
        let mut atts = Vec::with_capacity(n as usize);
        for _ in 0..n {
            let mut ins = SimpleEnclave::new(n);

            assert!(ins.init().is_ok());
            let att = ins.get_attestation_certificate().unwrap();
            instances.push(ins);
            atts.push(att);
        }
        let mut shares = Vec::with_capacity(n as usize);
        for instance in instances.iter_mut() {
            for i in 0u64..n {
                if let InitHandshakeResult::Ok(share) =
                    instance.init_peer_handshake(ReplicaId::from_u64(i), atts[i as usize])
                {
                    shares.push(share);
                } else {
                    panic!("Expected valid handshake result");
                }
            }
        }

        for instance in instances.iter_mut() {
            for i in 0u64..n {
                assert!(matches!(
                    instance.complete_peer_handshake(ReplicaId::from_u64(i), &shares[i as usize]),
                    HandshakeResult::Ok
                ));
            }
        }

        for instance1 in instances.iter() {
            for instance2 in instances.iter() {
                assert_eq!(instance1.automata.seed, instance2.automata.seed);
            }
        }
        instances
    }

    #[test]
    fn setup() {
        for i in 3..=100 {
            for instance in instances(i) {
                assert!(instance.is_ready())
            }
        }
    }

    #[test]
    fn test_rand() {
        for i in 3..=100 {
            let mut instances = instances(i);
            for _ in 0..100 {
                let mut rands = Vec::new();
                for instance in instances.iter_mut() {
                    rands.push(instance.automata.rand());
                }
                assert_eq!(rands.len(), i as usize);
                assert!(rands[0] < i);
                for r1 in rands.iter() {
                    for r2 in rands.iter() {
                        assert_eq!(r1, r2);
                    }
                }
            }
        }
    }

    #[test]
    fn sign_attempt() {
        let mut instances = instances(3);
        assert!(matches!(
            instances[0].sign(make_vertex(0, 0).deref().deref().clone()),
            Ok((_, 0,))
        ));
    }

    #[test]
    fn integrate_three() {
        let mut instances = instances(3);
        let _v0_1 = make_vertex_with_enclave(1, 0, &mut instances[0]);
        let _v0_2 = make_vertex_with_enclave(2, 0, &mut instances[0]);
        let _v0_3 = make_vertex_with_enclave(3, 0, &mut instances[0]);
        let v0_4 = make_vertex_with_enclave(4, 0, &mut instances[0]);

        let _v1_1 = make_vertex_with_enclave(1, 1, &mut instances[1]);
        let _v1_2 = make_vertex_with_enclave(2, 1, &mut instances[1]);
        let _v1_3 = make_vertex_with_enclave(3, 1, &mut instances[1]);
        let v1_4 = make_vertex_with_enclave(4, 1, &mut instances[1]);

        let _v2_1 = make_vertex_with_enclave(1, 2, &mut instances[2]);

        let toss = instances[0].toss(vec![v0_4.clone(), v1_4.clone()]);
        assert!(toss.unwrap().as_u64() < 3);
    }
}
