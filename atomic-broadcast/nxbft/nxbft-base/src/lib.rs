use std::fmt::Debug;

use serde::{Deserialize, Serialize};
use shared_ids::{id_type, ReplicaId};

pub mod enclave;
pub mod vertex;

id_type!(pub Round);

#[derive(Hash, PartialEq, Eq, Clone, Copy, Serialize, Deserialize, PartialOrd, Ord)]
pub struct DagAddress {
    round: Round,
    creator: ReplicaId,
}

impl Debug for DagAddress {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("DagAddress")
            .field(&self.round)
            .field(&self.creator)
            .finish()
    }
}

impl DagAddress {
    pub fn new(round: Round, creator: ReplicaId) -> Self {
        Self { round, creator }
    }

    pub fn round(&self) -> Round {
        self.round
    }
    pub fn creator(&self) -> ReplicaId {
        self.creator
    }
}

impl AsRef<Round> for DagAddress {
    fn as_ref(&self) -> &Round {
        &self.round
    }
}

impl AsRef<ReplicaId> for DagAddress {
    fn as_ref(&self) -> &ReplicaId {
        &self.creator
    }
}

impl From<DagAddress> for Round {
    fn from(value: DagAddress) -> Self {
        value.round
    }
}

impl From<DagAddress> for ReplicaId {
    fn from(value: DagAddress) -> Self {
        value.creator
    }
}
