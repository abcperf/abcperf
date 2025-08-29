use crate::{
    output::NotReflectedOutput, peer_message::usig_message::checkpoint::CheckpointCertificate,
    MinBft, Nonce, ReplicaStorage, RequestPayload,
};
use hashbar::Hashbar;
use serde::{Deserialize, Serialize};
use shared_ids::ReplicaId;
use std::{collections::HashSet, fmt::Debug};
use tracing::{info, warn};
use usig::Usig;

impl<P: RequestPayload, U: Usig> MinBft<P, U>
where
    U::Attestation:
        Clone + Serialize + for<'a> Deserialize<'a> + PartialEq + Hashbar + 'static + Sync,
    U::Signature: Clone + Serialize + Debug + Hashbar,
{
    /// Process a message of type RecoveryReq.
    ///
    /// # Arguments
    ///
    /// * `from` - The ID of the replica from which the Hello originates.
    /// * `attestation` - The attestation of the replica.
    /// * `output` - The output struct to be adjusted in case of, e.g., errors
    ///              or responses.
    pub(crate) fn process_recovery_reply(
        &mut self,
        from: ReplicaId,
        output: &mut NotReflectedOutput<P, U>,
        nonce: Nonce,
        storage: ReplicaStorage<P>,
        certificate: CheckpointCertificate<U::Signature, U::Attestation>,
    ) {
        if self.validate_checkpoint_certificate(self.config.t, &certificate, nonce) {
            let storage_hash = storage.hash();
            if certificate.my_checkpoint.state_hash == storage_hash {
                info!(
                    "Handling recovery reply {:?} from {:?} with hash {:?}",
                    nonce, from, storage_hash
                );

                self.handle_recovery_reply(nonce, certificate, storage, output);
            } else {
                warn!(
                    "Received invalid replica storage replica {:?} expected {:?} but received {:?}",
                    from, certificate.my_checkpoint.state_hash, storage_hash
                );
            }
        } else {
            warn!(
                "Received invalid checkpoint certificate from replica {:?}",
                from
            );
        }
    }

    fn validate_checkpoint_certificate(
        &self,
        t: u64,
        certificate: &CheckpointCertificate<U::Signature, U::Attestation>,
        nonce: Nonce,
    ) -> bool {
        // checkpoints are from different replicas
        //TODO we also need to check some kind of signature to determine that the checkpoints are from different, valid replicas

        let mut replicas = HashSet::new();
        for checkpoint in &certificate.other_checkpoints {
            replicas.insert(checkpoint.origin);
        }
        if replicas.len() < t.try_into().unwrap() {
            return false;
        }

        // checkpoints match
        let (
            cmp_hash,
            cmp_counter,
            cmp_view,
            cmp_batches,
            cmp_client_reqs,
            _cmp_recovering,
            cmp_usigs,
        ) = {
            let cert = &certificate.my_checkpoint;
            (
                cert.state_hash,
                cert.counter_latest_prep,
                cert.view_latest_prep,
                cert.total_amount_accepted_batches,
                cert.last_executed_client_req.clone(),
                cert.recovering_replicas.clone(),
                cert.usig_attestations.clone(),
            )
        };
        for checkpoint in &certificate.other_checkpoints {
            if checkpoint.view_latest_prep != cmp_view {
                return false;
            }
            if checkpoint.state_hash != cmp_hash {
                return false;
            }
            if checkpoint.counter_latest_prep != cmp_counter {
                return false;
            }
            if checkpoint.total_amount_accepted_batches != cmp_batches {
                return false;
            }
            if checkpoint.last_executed_client_req != cmp_client_reqs {
                return false;
            }
            if checkpoint.usig_attestations != cmp_usigs {
                return false;
            }

            match checkpoint.recovering_replicas.get(&self.config.id) {
                Some(cp_nonce) => {
                    if nonce != *cp_nonce {
                        warn!("Received checkpoint that contains different recovery nonce. got={:?}, expected={:?}", cp_nonce, nonce);
                        return false;
                    }
                }
                None => {
                    warn!("Received checkpoint that does not contain current recovery nonce");
                }
            }
        }
        true
    }
}
