use crate::{
    output::NotReflectedOutput, MinBft, Nonce, RecoveryRequestPayload, RequestPayload,
    WrappedRequestPayload,
};
use hashbar::Hashbar;
use serde::{Deserialize, Serialize};
use shared_ids::ClientId;
use shared_ids::ReplicaId;
use std::fmt::Debug;
use tracing::trace;
use usig::Usig;

impl<P: RequestPayload, U: Usig> MinBft<P, U>
where
    U::Attestation:
        Clone + Serialize + for<'a> Deserialize<'a> + PartialEq + Hashbar + 'static + Sync,
    U::Signature: Clone + Serialize + Hashbar + Debug,
{
    /// Process a message of type RecoveryReq.
    ///
    /// # Arguments
    ///
    /// * `from` - The ID of the replica from which the Hello originates.
    /// * `attestation` - The attestation of the replica.
    /// * `output` - The output struct to be adjusted in case of, e.g., errors
    ///              or responses.
    pub(crate) fn process_recovery_request(
        &mut self,
        from: ReplicaId,
        attestation: U::Attestation,
        nonce: Nonce,
        output: &mut NotReflectedOutput<P, U>,
    ) {
        if from == self.config.id {
            trace!("Received a recovery request from self. Ignoring.");
            return;
        }

        let client_id = ClientId::from_u64(from.as_u64());
        let next_req_id = self.request_processor.next_client_req_id(&client_id);

        let recovery_request = RecoveryRequestPayload {
            request_id: next_req_id,
            replica_id: from,
            attestation,
            nonce,
        };
        self.handle_internal_client_message(
            client_id,
            WrappedRequestPayload::<P, U::Attestation>::Internal(recovery_request),
            output,
        );
    }
}
