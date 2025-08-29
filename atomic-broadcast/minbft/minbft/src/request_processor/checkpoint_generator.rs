use std::{collections::HashMap, marker::PhantomData};

use tracing::trace;
use usig::{Counter, Usig};

use crate::{
    peer_message::usig_message::{
        checkpoint::{CheckpointContent, CheckpointHash},
        view_peer_message::prepare::Prepare,
    },
    ClientId, Config, Nonce, ReplicaId, RequestId, RequestPayload,
};

/// The purpose of the struct is to generate Checkpoints.
/// In addition, it keeps track of the hash of the last Checkpoint generated and
/// of the total amount of accepted batches.
#[derive(Debug, Clone)]
pub(crate) struct CheckpointGenerator<P: RequestPayload, U: Usig> {
    phantom_data: PhantomData<(P, U)>,
}

impl<P: RequestPayload, U: Usig> CheckpointGenerator<P, U> {
    /// Create a new CheckpointGenerator.
    pub(crate) fn new() -> Self {
        CheckpointGenerator {
            phantom_data: PhantomData,
        }
    }

    /// Decides if a Checkpoint should be generated.
    ///
    /// # Arguments
    ///
    /// * `prepare` - The [Prepare] that has just been accepted.
    /// * `config` - The configuration of the replica.
    ///
    /// # Return Value
    ///
    /// Returns a Checkpoint if it decides in favor, otherwise [None] is
    /// returned.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn generate_checkpoint(
        &mut self,
        prepare: &Prepare<P, U::Signature, U::Attestation>,
        storage_hash: CheckpointHash,
        last_executed_client_req: HashMap<ClientId, RequestId>,
        usig_attestations: HashMap<ReplicaId, U::Attestation>,
        recovering_replicas: HashMap<ReplicaId, Nonce>,
        total_amount_accepted_batches: u64,
        config: &Config,
    ) -> Option<CheckpointContent<U::Attestation>> {
        trace!(
            "Accepted in total {:?} batches.",
            total_amount_accepted_batches
        );
        if total_amount_accepted_batches % config.checkpoint_period != 0 {
            return None;
        };
        let checkpoint = CheckpointContent {
            origin: config.id,
            state_hash: storage_hash,
            counter_latest_prep: prepare.counter(),
            total_amount_accepted_batches,
            last_executed_client_req,
            usig_attestations,
            recovering_replicas,
            view_latest_prep: prepare.view,
        };
        Some(checkpoint)
    }
}
