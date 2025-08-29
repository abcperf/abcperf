use std::collections::HashSet;
use std::fmt::Debug;
use std::mem;
use std::{error::Error, hash::Hash};

use crate::{vertex::SignableVertex, vertex::SignedVertex, Round};
use hashbar::Hashbar;
use rand::distributions::Uniform;
use rand::prelude::Distribution;
use rand::SeedableRng;
use rand_chacha::ChaCha12Rng;
use rangemap::RangeInclusiveMap;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use shared_ids::ReplicaId;

pub mod simple;

pub trait Enclave {
    type Signature: Clone + Debug + for<'a> Deserialize<'a> + Serialize + Hashbar + Hash + Eq;

    type EnclaveError: Error + Send;

    type Attestation: Clone + Debug + for<'a> Deserialize<'a> + Serialize + Eq + Hashbar + Hash;

    type Handshake: Debug + for<'a> Deserialize<'a> + Serialize + Clone;

    type Export: Clone + Debug + for<'a> Deserialize<'a> + Serialize + Send;

    fn new(n: u64) -> Self;

    fn get_n(&self) -> u64;

    fn init(&mut self) -> Result<(), Self::EnclaveError>;

    fn get_attestation_certificate(&mut self) -> Result<Self::Attestation, Self::EnclaveError>;

    fn verify_attestation(
        &self,
        attestation: &Self::Attestation,
    ) -> Result<bool, Self::EnclaveError>;

    fn init_peer_handshake(
        &mut self,
        peer_id: ReplicaId,
        attestation: Self::Attestation,
    ) -> InitHandshakeResult<Self::Handshake, Self::EnclaveError>;

    fn complete_peer_handshake(
        &mut self,
        peer_id: ReplicaId,
        handshake: &Self::Handshake,
    ) -> HandshakeResult<Self::EnclaveError>;

    fn export(&self) -> Result<Self::Export, Self::EnclaveError>;

    fn recover(&mut self, export: Self::Export) -> Result<(), Self::EnclaveError>;

    fn replace_peer(
        &mut self,
        peer_id: ReplicaId,
        attestation: &Self::Attestation,
        max_round: Round,
    ) -> Result<bool, Self::EnclaveError>;

    fn sign(
        &mut self,
        payload: SignableVertex,
    ) -> Result<(Self::Signature, u64), Self::EnclaveError>;

    fn toss(
        &mut self,
        proof: Vec<SignedVertex<Self::Signature>>,
    ) -> Result<ReplicaId, Self::EnclaveError>;

    fn verify(
        &self,
        peer_id: ReplicaId,
        counter: u64,
        message: SignableVertex,
        signature: &Self::Signature,
    ) -> Result<bool, Self::EnclaveError>;

    fn is_ready(&self) -> bool;
}

pub enum InitHandshakeResult<Handshake, Err> {
    Ok(Handshake),
    DuplicateHello,
    InvalidAttestation,
    EnclaveError(Err),
}

pub enum HandshakeResult<Err> {
    MissingAttestation,
    DuplicateReply,
    InvalidHandshakeEncryption,
    Ok,
    EnclaveError(Err),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
enum Peer<Att: Clone + Debug + Eq> {
    None,
    Attested(Att),
    HandshakeComplete(Att, RangeInclusiveMap<u64, Att>),
}

impl<Att: Clone + Debug + Eq> Peer<Att> {
    fn unwrap_complete(&self) -> (&Att, &RangeInclusiveMap<u64, Att>) {
        match self {
            Peer::HandshakeComplete(id, map) => (id, map),
            _ => panic!("Invalid peer state"),
        }
    }
}

#[derive(Error, Debug)]
pub enum EnclaveAutomataError<E> {
    #[error("Invalid Peer ID")]
    InvalidPeerId,
    #[error("Not Ready")]
    NotReady,
    #[error("Not enough valid and signed vertices provided to allow signing")]
    NotEnoughValidVertices,
    #[error("Can only sign vertex when having round matching counter")]
    InvalidVertexRound,
    #[error("enclave trusted error: {0:?}")]
    EnclaveTrusted(#[from] E),
}

pub trait EnclaveTrusted {
    type Attestation: Clone + Eq + Debug;
    type Error;
    type Handshake;
    type Signature;

    fn gen_seed_share(&self) -> Result<[u8; 32], Self::Error>;
    fn seal_seed_share(
        &self,
        attestation: &Self::Attestation,
        share: [u8; 32],
    ) -> Result<Self::Handshake, Self::Error>;
    fn unseal_seed_share(
        &self,
        share: &Self::Handshake,
        attestation: &Self::Attestation,
    ) -> Result<[u8; 32], Self::Error>;

    fn sign(&mut self, message_hash: &[u8], count: u64) -> Result<Self::Signature, Self::Error>;

    fn verify(
        attestation: &Self::Attestation,
        counter: u64,
        message_hash: &[u8],
        signature: &Self::Signature,
    ) -> Result<bool, Self::Error>;

    fn get_attestation_certificate(&mut self) -> Result<Self::Attestation, Self::Error>;
}

#[derive(Debug)]
pub struct EnclaveAutomata<Enc: EnclaveTrusted> {
    n: u64,
    quorum: u64,
    peers_added: u64,
    c: u64,
    peers: Vec<Peer<Enc::Attestation>>,
    seed: [u8; 32],
    seed_share: Option<[u8; 32]>,
    rng: Option<ChaCha12Rng>,
    uniform: Uniform<u64>,
    next_toss: Round,
    enclave: Enc,
}

impl<Enc: EnclaveTrusted> EnclaveAutomata<Enc> {
    pub fn new(n: u64, enclave: Enc) -> Self {
        assert!(n >= 3, "Minimum number of peers is three");
        Self {
            n,
            quorum: (n / 2) + 1,
            peers_added: 0,
            c: 0,
            peers: (0..n).map(|_| Peer::None).collect(),
            seed: [0; 32],
            seed_share: None,
            rng: None,
            uniform: Uniform::from(0..n),
            next_toss: 4.into(),
            enclave,
        }
    }

    pub fn init(&mut self) -> Result<(), EnclaveAutomataError<Enc::Error>> {
        assert_eq!(self.seed_share, None);
        self.seed_share = Some(self.enclave.gen_seed_share()?);
        Ok(())
    }

    pub fn get_n(&self) -> u64 {
        self.n
    }

    pub fn init_peer_handshake(
        &mut self,
        peer_id: shared_ids::ReplicaId,
        attestation: Enc::Attestation,
    ) -> InitHandshakeResult<Enc::Handshake, EnclaveAutomataError<Enc::Error>> {
        assert_ne!(self.seed_share, None);
        if peer_id.as_u64() >= self.n {
            return InitHandshakeResult::EnclaveError(EnclaveAutomataError::InvalidPeerId);
        }
        if let Peer::None = self.peers[peer_id.as_u64() as usize] {
            match self
                .enclave
                .seal_seed_share(&attestation, self.seed_share.unwrap())
            {
                Ok(share) => {
                    self.peers[peer_id.as_u64() as usize] = Peer::Attested(attestation);
                    InitHandshakeResult::Ok(share)
                }
                Err(e) => {
                    InitHandshakeResult::EnclaveError(EnclaveAutomataError::EnclaveTrusted(e))
                }
            }
        } else {
            InitHandshakeResult::DuplicateHello
        }
    }

    pub fn export(
        &self,
    ) -> Result<EnclaveAutomataExport<Enc::Attestation>, EnclaveAutomataError<Enc::Error>> {
        if !self.is_ready() {
            return Err(EnclaveAutomataError::NotReady);
        }
        Ok(EnclaveAutomataExport {
            seed: self.seed,
            next_toss: self.next_toss,
            n: self.n,
            peers: self.peers.clone(),
        })
    }

    pub fn recover(
        &mut self,
        export: EnclaveAutomataExport<Enc::Attestation>,
    ) -> Result<(), EnclaveAutomataError<Enc::Error>> {
        assert_eq!(self.n, export.n);
        self.peers = export.peers;
        self.peers_added = self.n;
        self.seed = export.seed;
        self.check_ready();

        while self.next_toss < export.next_toss {
            self.rand();
        }
        assert_eq!(self.next_toss, export.next_toss);
        Ok(())
    }

    pub fn replace_peer(
        &mut self,
        peer_id: ReplicaId,
        attestation: &Enc::Attestation,
        max_round: Round,
    ) -> Result<bool, EnclaveAutomataError<Enc::Error>> {
        if !self.is_ready() {
            return Ok(false);
        }

        let Peer::HandshakeComplete(id, map) = &mut self.peers[peer_id.as_u64() as usize] else {
            panic!("Invalid peer state");
        };
        let old = mem::replace(id, attestation.clone());
        map.insert(
            map.last_range_value().map(|(r, _)| *r.end()).unwrap_or(0)..=max_round.as_u64(),
            old,
        );

        Ok(true)
    }

    pub fn complete_peer_handshake(
        &mut self,
        peer_id: shared_ids::ReplicaId,
        handshake: &Enc::Handshake,
    ) -> HandshakeResult<EnclaveAutomataError<Enc::Error>> {
        if peer_id.as_u64() >= self.n {
            return HandshakeResult::EnclaveError(EnclaveAutomataError::InvalidPeerId);
        }
        match &self.peers[peer_id.as_u64() as usize] {
            Peer::None => HandshakeResult::MissingAttestation,
            Peer::HandshakeComplete(_, _) => HandshakeResult::DuplicateReply,
            Peer::Attested(att) => {
                let handshake = match self.enclave.unseal_seed_share(handshake, att) {
                    Ok(h) => h,
                    Err(e) => {
                        return HandshakeResult::EnclaveError(EnclaveAutomataError::EnclaveTrusted(
                            e,
                        ))
                    }
                };
                for (seed, handshake) in self.seed.iter_mut().zip(handshake.iter()) {
                    *seed ^= handshake;
                }

                self.peers_added += 1;
                self.peers[peer_id.as_u64() as usize] =
                    Peer::HandshakeComplete(att.clone(), RangeInclusiveMap::new());
                self.check_ready();
                HandshakeResult::Ok
            }
        }
    }

    pub fn sign(
        &mut self,
        message_hash: &[u8],
    ) -> Result<(Enc::Signature, u64), EnclaveAutomataError<Enc::Error>> {
        if !self.is_ready() {
            return Err(EnclaveAutomataError::NotReady);
        }
        self.c += 1;
        let sig = self.enclave.sign(message_hash, self.c - 1)?;
        Ok((sig, self.c - 1))
    }

    pub fn verify(
        &self,
        peer_id: ReplicaId,
        counter: u64,
        message: SignableVertex,
        signature: &Enc::Signature,
    ) -> Result<bool, EnclaveAutomataError<Enc::Error>> {
        let message_hash = message.to_hash();
        if peer_id.as_u64() >= self.n {
            return Err(EnclaveAutomataError::InvalidPeerId);
        }
        if !self.is_ready() {
            return Err(EnclaveAutomataError::NotReady);
        }

        let (current_attestation, old_attestations) =
            self.peers[peer_id.as_u64() as usize].unwrap_complete();

        let attestion =
            if let Some(attestion) = old_attestations.get(message.address().round().as_ref()) {
                attestion
            } else {
                current_attestation
            };

        Ok(Enc::verify(
            attestion,
            counter,
            message_hash.as_ref(),
            signature,
        )?)
    }

    pub fn is_ready(&self) -> bool {
        self.rng.is_some()
    }

    pub fn toss(
        &mut self,
        proof: Vec<SignedVertex<Enc::Signature>>,
    ) -> Result<ReplicaId, EnclaveAutomataError<Enc::Error>> {
        let mut valid = 0u64;
        let mut seen = HashSet::new();
        for v in proof.into_iter() {
            if seen.contains(&v.creator()) || v.round() != self.next_toss {
                continue;
            }
            match self.verify(v.creator(), v.counter(), v.clone(), v.signature()) {
                Ok(b) => {
                    if b {
                        seen.insert(v.creator());
                        valid += 1;
                    }
                }
                Err(e) => return Err(e),
            }
        }
        if valid < self.quorum {
            Err(EnclaveAutomataError::NotEnoughValidVertices)
        } else {
            Ok(ReplicaId::from_u64(self.rand()))
        }
    }

    pub fn check_ready(&mut self) {
        assert!(self.rng.is_none());
        if self.peers_added < self.n {
            return;
        }

        self.rng = Some(ChaCha12Rng::from_seed(self.seed));
    }

    pub fn rand(&mut self) -> u64 {
        self.next_toss += 4;
        self.uniform.sample(self.rng.as_mut().unwrap())
    }

    pub fn get_attestation_certificate(
        &mut self,
    ) -> Result<Enc::Attestation, EnclaveAutomataError<Enc::Error>> {
        let cert = self.enclave.get_attestation_certificate()?;
        Ok(cert)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnclaveAutomataExport<Att: Clone + Eq + Debug> {
    seed: [u8; 32],
    next_toss: Round,
    pub n: u64,
    peers: Vec<Peer<Att>>,
}
