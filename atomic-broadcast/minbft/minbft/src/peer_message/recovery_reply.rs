use serde::{de::DeserializeOwned, Deserialize, Serialize};

use crate::{Nonce, ReplicaStorage, RequestPayload};

use super::usig_message::checkpoint::CheckpointCertificate;

/// Defines a message of type [RecoveryReply].
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(bound(
    serialize = "Sig: Serialize, Att: Serialize",
    deserialize = "Sig: DeserializeOwned, Att: DeserializeOwned"
))]
pub(crate) struct RecoveryReply<P: RequestPayload, Sig, Att> {
    pub(crate) certificate: CheckpointCertificate<Sig, Att>,
    pub(crate) nonce: Nonce,
    pub(crate) storage: ReplicaStorage<P>,
}
