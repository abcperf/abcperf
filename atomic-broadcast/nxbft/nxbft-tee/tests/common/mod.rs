use hashbar::{Hashbar, Hasher};
use serde::Serialize;
use shared_ids::{ClientId, RequestId};
use transaction_trait::Transaction;

pub mod checkers;
pub mod happy;
pub mod recovery;
pub mod setup;

#[derive(Clone, Debug, Serialize, PartialEq, Eq, Hash)]
pub struct TestTx(u64);

impl Hashbar for TestTx {
    fn hash<H: Hasher>(&self, hasher: &mut H) {
        hasher.update(&self.0.to_le_bytes())
    }
}

impl Transaction for TestTx {
    fn request_id(&self) -> RequestId {
        RequestId::from_u64(self.0)
    }

    fn client_id(&self) -> ClientId {
        ClientId::from_u64(self.0)
    }
}
