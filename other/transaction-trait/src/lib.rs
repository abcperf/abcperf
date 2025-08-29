use shared_ids::{ClientId, RequestId};

pub trait Transaction {
    fn client_id(&self) -> ClientId;
    fn request_id(&self) -> RequestId;
}
