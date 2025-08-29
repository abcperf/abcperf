use crate::enclave::BlockHash;
use hashbar::{Hashbar, Hasher};
use serde::{Deserialize, Serialize};

pub mod enclave;
pub mod view;

#[derive(PartialEq, Eq, Serialize, Deserialize, Clone, Debug)]
pub enum Parent {
    None,
    Some(BlockHash),
}

impl Hashbar for Parent {
    fn hash<H: Hasher>(&self, hasher: &mut H) {
        match self {
            Self::None => hasher.update(&[0]),
            Self::Some(value) => {
                hasher.update(&[1]);
                value.hash(hasher);
            }
        }
    }
}

impl PartialEq<BlockHash> for Parent {
    fn eq(&self, other: &BlockHash) -> bool {
        match self {
            Self::None => false,
            Self::Some(value) => value.eq(other),
        }
    }
}
