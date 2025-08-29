use std::ops::{Deref, DerefMut};

use anyhow::Result;
use hashbar::Hashbar;
use serde::{Deserialize, Serialize};
use shared_ids::ClientId;
use tracing::trace;

use crate::{RequestPayload, WrappedRequestPayload};

/// Defines a ClientRequest.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) struct ClientRequest<P, Att> {
    pub(crate) client: ClientId,
    pub(crate) payload: WrappedRequestPayload<P, Att>,
}

impl<P: Hashbar, Att: Hashbar> Hashbar for ClientRequest<P, Att> {
    fn hash<H: hashbar::Hasher>(&self, hasher: &mut H) {
        self.client.hash(hasher);
        self.payload.hash(hasher);
    }
}

impl<P, Att> Deref for ClientRequest<P, Att> {
    type Target = WrappedRequestPayload<P, Att>;

    /// Returns a reference to the payload of the ClientRequest.
    fn deref(&self) -> &Self::Target {
        &self.payload
    }
}

impl<P, Att> DerefMut for ClientRequest<P, Att> {
    /// Returns a mutable reference to the payload of the ClientRequest.
    fn deref_mut(&mut self) -> &mut <Self as Deref>::Target {
        &mut self.payload
    }
}

/// Defines a RequestBatch.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, Hash)]
#[repr(transparent)]
#[serde(transparent)]
pub(crate) struct RequestBatch<P, Att> {
    /// The batch of ClientRequests.
    pub(crate) batch: Box<[ClientRequest<P, Att>]>,
}

impl<P: Hashbar, Att: Hashbar> Hashbar for RequestBatch<P, Att> {
    fn hash<H: hashbar::Hasher>(&self, hasher: &mut H) {
        for request in self.batch.iter() {
            request.hash(hasher);
        }
    }
}

impl<P: RequestPayload, Att> RequestBatch<P, Att> {
    /// Creates a new RequestBatch with the given batch of ClientRequests.
    pub(crate) fn new(batch: Box<[ClientRequest<P, Att>]>) -> Self {
        Self { batch }
    }

    /// Validates the RequestBatch.
    pub(crate) fn validate(&self) -> Result<()> {
        trace!("Validating batch of requests ...");
        for request in self.batch.iter() {
            trace!(
                "Validating client request (ID: {:?}, client ID: {:?}) contained in batch ...",
                request.id(),
                request.client
            );
            trace!("Successfully validated client request (ID: {:?}, client ID: {:?}) contained in batch.", request.id(), request.client);
        }
        trace!("Successfully validated batch of requests.");
        Ok(())
    }

    pub(crate) fn first(&self) -> Option<&ClientRequest<P, Att>> {
        self.batch.first()
    }

    pub(crate) fn len(&self) -> usize {
        self.batch.len()
    }
}

impl<P, Att> IntoIterator for RequestBatch<P, Att> {
    type Item = ClientRequest<P, Att>;

    type IntoIter = std::vec::IntoIter<Self::Item>;

    /// Converts the RequestBatch into an Iterator.
    fn into_iter(self) -> Self::IntoIter {
        let batch = Vec::from(self.batch); // remove once https://github.com/rust-lang/rust/issues/59878 is fixed
        batch.into_iter()
    }
}
