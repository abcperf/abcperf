use hashbar::{Hashbar, Hasher};
use heaps::Keyable;
use nxbft_base::vertex::SignableVertex;
use nxbft_base::vertex::Vertex;
use nxbft_base::DagAddress;
use serde::{Deserialize, Serialize};
use shared_ids::ReplicaId;
use std::fmt::Debug;
use std::fmt::Display;
use std::hash::Hash;

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct PeerMessage<Tx: Hashbar, Att: Hashbar, Handshake, Sig: Hashbar + Hash> {
    pub from: ReplicaId,
    pub(super) message: Message<Tx, Att, Handshake, Sig>,
}

impl<Tx: Hashbar, Att: Hashbar, Handshake, Sig: Hashbar + Hash> Display
    for PeerMessage<Tx, Att, Handshake, Sig>
{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let msg_type = match &self.message {
            Message::Hello(_) => "Hello".to_string(),
            Message::HelloReply(_) => "HelloReply".to_string(),
            Message::HelloEcho(creator, _) => {
                format!("HelloEcho({})", creator.as_u64())
            }
            Message::Ready => "Ready".to_string(),
            Message::Signed(msg) => msg.to_string(),
            Message::VertexRequest(address) => format!("VertexRequest({:?})", address),
            Message::ReocoveryRequest(_) => "ReocoveryRequest".to_string(),
            Message::RecoveryCommit(..) => "RecoveryCommit".to_string(),
            Message::RecovertyProposal(_) => "RecoveryProposal".to_string(),
        };
        f.write_fmt(format_args!("{} from {}", msg_type, self.from.as_u64(),))
    }
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub(super) enum Message<Tx: Hashbar, Att: Hashbar, Handshake, Sig: Hashbar + Hash> {
    Hello(Att),
    HelloEcho(ReplicaId, Att),
    HelloReply(Handshake),
    Ready,
    Signed(SignedMessage<Tx, Sig>),
    VertexRequest(DagAddress),
    ReocoveryRequest(Att),
    RecovertyProposal(RecoveryProposal<Tx, Att, Sig>),
    RecoveryCommit(RecoveryCommit<Tx, Att, Sig>),
}

#[derive(Debug, Deserialize, Serialize, Clone, Hash, PartialEq, Eq)]
pub(crate) struct RecoveryCommit<Tx: Hashbar, Att: Hashbar, Sig: Hashbar + Hash> {
    pub(crate) for_id: ReplicaId,
    pub(crate) for_attestation: Att,
    pub(crate) messages: Vec<SignedMessage<Tx, Sig>>,
}

#[derive(Clone, Debug, Deserialize, Serialize, Hash, PartialEq, Eq)]
pub(super) struct SignedMessage<Tx: Hashbar, Sig: Hashbar> {
    pub(super) payload: NonEquivocationPayload<Tx, Sig>,
}

impl<Tx: Hashbar, Sig: Hashbar> Hashbar for SignedMessage<Tx, Sig> {
    fn hash<H: Hasher>(&self, hasher: &mut H) {
        Hashbar::hash(&self.creator(), hasher);
        hasher.update(&self.counter().to_le_bytes());
        self.signature().hash(hasher);
    }
}

impl<Tx: Hashbar, Sig: Hashbar> SignedMessage<Tx, Sig> {
    pub(crate) fn creator(&self) -> ReplicaId {
        self.payload.0.creator()
    }

    pub(crate) fn counter(&self) -> u64 {
        self.payload.0.counter()
    }

    pub(crate) fn signature(&self) -> &Sig {
        self.payload.0.signature()
    }

    pub(crate) fn signable(&self) -> SignableVertex {
        self.payload.0.clone()
    }
}

impl<Tx: Hashbar, Sig: Hashbar> Display for SignedMessage<Tx, Sig> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_fmt(format_args!(
            "NonEquivocationMessage({}): {}",
            self.creator(),
            self.payload
        ))
    }
}

impl<Tx: Hashbar, Sig: Hashbar> Keyable for SignedMessage<Tx, Sig> {
    type Key = u64;
    fn key(&self) -> Self::Key {
        self.counter()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct RecoveryProposal<Tx: Hashbar, Att: Hashbar, Sig: Hashbar> {
    pub(super) for_id: ReplicaId,
    pub(super) for_attestation: Att,
    pub(super) messages: Vec<SignedMessage<Tx, Sig>>,
}

#[derive(Clone, Debug, Deserialize, Serialize, Hash, PartialEq, Eq)]
pub(crate) struct NonEquivocationPayload<Tx: Hashbar, Sig: Hashbar>(pub(crate) Vertex<Tx, Sig>);

impl<Tx: Hashbar, Sig: Hashbar> Display for NonEquivocationPayload<Tx, Sig> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Vertex(r: {}, p: {})",
            self.0.round(),
            self.0.creator().as_u64()
        )
    }
}
