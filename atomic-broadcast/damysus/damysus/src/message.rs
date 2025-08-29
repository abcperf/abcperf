use damysus_base::{enclave::SignedCommitment, view::View};
use serde::{Deserialize, Serialize};
use shared_ids::ReplicaId;

use crate::blockstore::Block;

pub(crate) enum MessageDestination {
    Broadcast,
    Unicast(ReplicaId),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PeerMessage<Tx, Sig, PubKey> {
    pub from: ReplicaId,
    pub(super) message: Message<Tx, Sig, PubKey>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) enum Message<Tx, Sig, PubKey> {
    NewBlock {
        block: Block<Tx, Sig>,
        prepare: SignedCommitment<Sig>,
    },
    Commitment(SignedCommitment<Sig>),
    Handshake(PubKey),
    BlockRequest(View),
    RequestedBlock(Block<Tx, Sig>),
}
