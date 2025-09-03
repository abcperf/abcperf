use shared_ids::{ReplicaId, RequestId};
use splitee_crypto::{hash::HashValue, math::FieldElement};

pub mod coin;
pub mod dkg;
pub mod encryption;
pub mod signature;

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct CoinToss(RequestId);

impl AsRef<[u8]> for CoinToss {
    fn as_ref(&self) -> &[u8] {
        todo!()
    }
}

impl CoinToss {
    pub fn to_bignum(&self) -> FieldElement {
        todo!()
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub struct Party(ReplicaId);

impl From<HashValue> for Party {
    fn from(hash: HashValue) -> Self {
        todo!()
    }
}

impl Party {
    pub fn from_u16(input: u16) -> Party {
        todo!()
    }

    pub fn to_usize(&self) -> usize {
        self.0.as_u64() as usize
    }

    pub fn to_bignum(&self) -> FieldElement {
        todo!()
    }

    pub fn to_fieldelement(&self) -> FieldElement {
        todo!()
    }
}
