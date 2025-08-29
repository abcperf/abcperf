use std::{fmt::Debug, sync::Arc};

use hashbar::{Hashbar, Hasher};
use serde::{Deserialize, Serialize};
use serde_with::serde_as;

#[derive(Deserialize, Serialize, PartialEq, Eq, Clone, Hash)]
#[serde(transparent)]
#[serde_as]
pub struct Payload {
    #[serde_as(as = "serde_with::Bytes")]
    payload: Arc<[u8]>,
}

impl Debug for Payload {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Payload")
            .field("size", &self.payload.len())
            .finish()
    }
}

impl Payload {
    pub(super) fn new(size: usize) -> Self {
        Self {
            payload: vec![0; size].into(),
        }
    }
}

impl Hashbar for Payload {
    fn hash<H: Hasher>(&self, hasher: &mut H) {
        hasher.update(&self.payload);
    }
}
