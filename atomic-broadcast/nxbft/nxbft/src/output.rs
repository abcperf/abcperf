use hashbar::Hashbar;
use shared_ids::ReplicaId;
use std::{
    fmt::Debug,
    time::{Duration, Instant},
};

use crate::{PeerMessage, Round, Wave};
use nxbft_base::enclave::Enclave;
use nxbft_base::DagAddress;

pub type Broadcasts<Tx, Enc> = Vec<
    PeerMessage<
        Tx,
        <Enc as Enclave>::Attestation,
        <Enc as Enclave>::Handshake,
        <Enc as Enclave>::Signature,
    >,
>;

pub type Unicasts<Tx, Enc> = Vec<(
    ReplicaId,
    PeerMessage<
        Tx,
        <Enc as Enclave>::Attestation,
        <Enc as Enclave>::Handshake,
        <Enc as Enclave>::Signature,
    >,
)>;

#[derive(Debug)]
pub struct Output<Tx: Hashbar, Enc: Enclave> {
    pub broadcasts: Broadcasts<Tx, Enc>,
    pub unicasts: Unicasts<Tx, Enc>,
    pub state_messages: Vec<StateMessage<Enc>>,
    pub vertex_timeout: VertexTimeout,
    pub delivery: Vec<Tx>,
    pub backup_enclave: bool,
    pub decision_times: Vec<Instant>,
}

impl<Tx: Hashbar, Enc: Enclave> Output<Tx, Enc> {
    pub fn is_empty(&self) -> bool {
        self.broadcasts.is_empty()
            && self.unicasts.is_empty()
            && self.state_messages.is_empty()
            && self.vertex_timeout == VertexTimeout::None
            && self.delivery.is_empty()
            && !self.backup_enclave
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VertexTimeout {
    Set(Duration),
    Cancel,
    None,
}

#[derive(Debug)]
pub enum StateMessage<Enc: Enclave> {
    Error(Error<Enc::EnclaveError>),
    PeerError(PeerError),
    Consensus(Consensus),
    Broadcast(Broadcast),
    Ready,
    Verbose(Verbose),
}

impl<E: Enclave> From<PeerError> for StateMessage<E> {
    fn from(value: PeerError) -> Self {
        Self::PeerError(value)
    }
}

impl<E: Enclave> From<Error<E::EnclaveError>> for StateMessage<E> {
    fn from(value: Error<E::EnclaveError>) -> Self {
        Self::Error(value)
    }
}

impl<E: Enclave> From<Consensus> for StateMessage<E> {
    fn from(value: Consensus) -> Self {
        Self::Consensus(value)
    }
}

impl<E: Enclave> From<Broadcast> for StateMessage<E> {
    fn from(value: Broadcast) -> Self {
        Self::Broadcast(value)
    }
}

impl<E: Enclave> From<Verbose> for StateMessage<E> {
    fn from(value: Verbose) -> Self {
        Self::Verbose(value)
    }
}

#[derive(thiserror::Error, Debug)]
pub enum Error<E> {
    EnclaveError(E),
    NotReady,
}

#[derive(Debug)]
pub struct PeerError {
    pub by: ReplicaId,
    pub error_type: PeerErrorType,
}

impl PeerError {
    pub(crate) fn from_type(by: ReplicaId, error_type: PeerErrorType) -> Self {
        Self { by, error_type }
    }
}

#[derive(Debug)]
pub enum PeerErrorType {
    InvalidSignature,
    InvalidVertexEdgesRound1,
    InvalidVertexEdges,
    UnknownVertexRequest,
    InvalidAttestation,
    DuplicateHelloReply,
    InvalidHandshakeEncryption,
    DuplicateReady,
    DuplicateHello,
    AttestationEquivocation { or: ReplicaId },
    DuplicateHelloEcho,
    InvalidFrom,
    InvalidRelayCreator { given: ReplicaId },
    DuplicateRecoveryProposal,
    DuplicateRecoveryCommit,
}

#[derive(Debug)]
pub enum Consensus {
    UnknownWaveLeader,
    DirectCommitRuleViolated,
    Decided {
        waves: Vec<Wave>,
        leaders: Vec<DagAddress>,
        order: Vec<DagAddress>,
    },
    RoundTransition {
        new_round: Round,
    },
    CatchUp {
        delayed: u64,
    },
}

#[derive(Debug)]
pub enum Broadcast {
    VertexRequest { round: Round, creator: ReplicaId },
}

#[derive(Debug)]
pub enum Verbose {
    ReceivedHello { from: ReplicaId },
    ReceivedHelloReply { from: ReplicaId },
    HelloReplyBeforeHello { from: ReplicaId },
    CachedHelloReply { from: ReplicaId },
    ReceivedHelloEcho { from: ReplicaId, creator: ReplicaId },
    BufferedVertex { vertex: DagAddress },
    NonEquivocationBroadcastReceive { creator: ReplicaId, payload: String },
    NonEquivocationBroadcastDeliver { creator: ReplicaId, payload: String },
    ReceivedDuplicate { from: ReplicaId, vertex: DagAddress },
    VertexAdded { vertex: DagAddress },
    OwnRecoveryIgnore(String),
    PeerRecoveryIgnore { from: ReplicaId, msg: String },
    IgnoreOutdatedRecoveryProposal { by: ReplicaId },
}

pub(super) struct UnreflectedOutput<Tx: Hashbar + Clone, Enc: Enclave> {
    pub(super) reflection_messages: Broadcasts<Tx, Enc>,
    pub(super) broadcasts: Broadcasts<Tx, Enc>,
    pub(super) unicasts: Unicasts<Tx, Enc>,
    pub(super) state_messages: Vec<StateMessage<Enc>>,
    pub(super) delivery: Vec<Tx>,
    pub(super) vertex_timeout: VertexTimeout,
    me: ReplicaId,
    pub(super) backup_enclave: bool,
    pub(super) decision_times: Vec<Instant>,
}

impl<Tx: Hashbar + Clone, Enc: Enclave> UnreflectedOutput<Tx, Enc> {
    pub fn new(me: ReplicaId) -> Self {
        UnreflectedOutput {
            reflection_messages: Vec::new(),
            broadcasts: Vec::new(),
            unicasts: Vec::new(),
            state_messages: Vec::new(),
            delivery: Vec::new(),
            vertex_timeout: VertexTimeout::None,
            me,
            backup_enclave: false,
            decision_times: Vec::new(),
        }
    }

    pub fn broadcast(
        &mut self,
        peer_message: PeerMessage<Tx, Enc::Attestation, Enc::Handshake, Enc::Signature>,
    ) {
        self.broadcasts.push(peer_message.clone());
        self.reflection_messages.push(peer_message);
    }

    pub fn unicast(
        &mut self,
        to: ReplicaId,
        peer_message: PeerMessage<Tx, Enc::Attestation, Enc::Handshake, Enc::Signature>,
    ) {
        if to == self.me {
            self.reflection_messages.push(peer_message);
        } else {
            self.unicasts.push((to, peer_message));
        }
    }

    pub fn state_message(&mut self, state_message: impl Into<StateMessage<Enc>>) {
        self.state_messages.push(state_message.into());
    }

    pub fn append_delivery(&mut self, tx: Tx) {
        self.delivery.push(tx);
    }

    pub fn set_timeout(&mut self, timeout: Duration) {
        self.vertex_timeout = VertexTimeout::Set(timeout);
    }

    pub fn cancel_timeout(&mut self) {
        self.vertex_timeout = VertexTimeout::None
    }

    pub fn set_backup_enclave(&mut self) {
        self.backup_enclave = true
    }

    pub(super) fn log_decision_time(&mut self) {
        self.decision_times.push(Instant::now());
    }
}
