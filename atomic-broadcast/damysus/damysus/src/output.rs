use damysus_base::enclave::Enclave;
use shared_ids::ReplicaId;
use std::time::{Duration, Instant};

use crate::{message::MessageDestination, PeerMessage};

type Unicasts<Tx, Sig, PubKey> = Vec<(ReplicaId, PeerMessage<Tx, Sig, PubKey>)>;

pub struct Output<Tx, Enc: Enclave> {
    pub broadcasts: Vec<PeerMessage<Tx, Enc::Signature, Enc::PublicKey>>,
    pub unicasts: Unicasts<Tx, Enc::Signature, Enc::PublicKey>,
    pub view_timeout: Timeout,
    pub block_timeout: Timeout,
    pub delivery: Vec<Tx>,
    pub ready: bool,
    pub view: u64,
    pub decision_times: Vec<Instant>,
}

impl<Tx, Enc: Enclave> Output<Tx, Enc> {
    pub fn is_empty(&self) -> bool {
        self.broadcasts.is_empty()
            && self.unicasts.is_empty()
            && self.view_timeout == Timeout::None
            && self.block_timeout == Timeout::None
            && self.delivery.is_empty()
            && !self.ready
    }
}

#[derive(PartialEq, Eq, Clone)]
pub enum Timeout {
    Set(Duration),
    None,
    Cancel,
}

pub(super) struct UnreflectedOutput<Tx, Enc: Enclave> {
    pub(super) reflection_messages: Vec<PeerMessage<Tx, Enc::Signature, Enc::PublicKey>>,
    pub(super) broadcasts: Vec<PeerMessage<Tx, Enc::Signature, Enc::PublicKey>>,
    pub(super) unicasts: Unicasts<Tx, Enc::Signature, Enc::PublicKey>,
    pub(super) view_timeout: Timeout,
    pub(super) block_timeout: Timeout,
    pub(super) delivery: Vec<Tx>,
    pub(super) ready: bool,
    pub(super) decision_times: Vec<Instant>,
    me: ReplicaId,
}

impl<Tx, Enc: Enclave> UnreflectedOutput<Tx, Enc> {
    pub(super) fn new(me: ReplicaId) -> Self {
        UnreflectedOutput {
            reflection_messages: Vec::new(),
            broadcasts: Vec::new(),
            unicasts: Vec::new(),
            view_timeout: Timeout::None,
            block_timeout: Timeout::None,
            delivery: Vec::new(),
            ready: false,
            me,
            decision_times: Vec::new(),
        }
    }

    pub(super) fn append_peer_message(
        &mut self,
        to: MessageDestination,
        peer_message: PeerMessage<Tx, Enc::Signature, Enc::PublicKey>,
    ) {
        match to {
            MessageDestination::Unicast(to) if to == self.me => {
                self.reflection_messages.push(peer_message)
            }
            MessageDestination::Unicast(to) => self.unicasts.push((to, peer_message)),
            MessageDestination::Broadcast => self.broadcasts.push(peer_message),
        }
    }

    pub(super) fn append_delivery(&mut self, tx: Tx) {
        self.delivery.push(tx);
    }

    pub(super) fn set_view_timeout(&mut self, timeout: Duration) {
        self.view_timeout = Timeout::Set(timeout);
    }

    pub(super) fn set_block_timeout(&mut self, timeout: Duration) {
        self.block_timeout = Timeout::Set(timeout);
    }

    pub(super) fn cancel_block_timeout(&mut self) {
        self.block_timeout = Timeout::Cancel
    }

    pub(super) fn ready(&mut self) {
        self.ready = true;
    }

    pub(super) fn log_decision_time(&mut self) {
        self.decision_times.push(Instant::now());
    }
}
