use std::collections::HashSet;
use std::fmt::Debug;

use serde::Deserialize;
use serde::Serialize;
use tracing::info;
use tracing::trace;
use usig::Counter;
use usig::Usig;

use crate::output::NotReflectedOutput;
use crate::peer_message::recovery_reply::RecoveryReply;
use crate::peer_message::usig_message::checkpoint::Checkpoint;
use crate::peer_message::usig_message::view_peer_message::ViewPeerMessage;
use crate::peer_message::usig_message::UsigMessage;
use crate::MinBft;
use crate::RequestPayload;

impl<P: RequestPayload, U: Usig> MinBft<P, U>
where
    U::Attestation: Clone + Serialize + for<'a> Deserialize<'a>,
    U::Signature: Clone + Serialize,
    U::Signature: Debug,
{
    /// Process a received message of type [Checkpoint].
    /// The steps are as following:
    ///
    /// 1. Collect the received [Checkpoint] with the collector of Checkpoints,
    ///     namely [CollectorCheckpoints](crate::peer_message_processor::collector::collector_checkpoints::CollectorCheckpoints).
    /// 2. Retrieve a checkpoint certificate of the collected Checkpoints if
    ///     they can be retrieved (see the documentation of collector).
    /// 3. Discard all entries in the message log of the replica that have
    ///     a counter value less than its own [Checkpoint] using the checkpoint
    ///     certificate.
    /// 4. Update the inner state of the replica by updating the last checkpoint
    ///     cert generated.
    ///
    /// # Arguments
    ///
    /// * `checkpoint` - The Checkpoint message to be processed.
    pub(crate) fn process_checkpoint(
        &mut self,
        checkpoint: Checkpoint<U::Signature, U::Attestation>,
        output: &mut NotReflectedOutput<P, U>,
    ) {
        let amount_collected = self
            .collector_checkpoints
            .collect_checkpoint(checkpoint.clone());
        if amount_collected <= self.config.t {
            trace!("Processing Checkpoint (origin: {:?}, counter latest accepted Prepare: {:?}, amount accepted batches: {:?}) resulted in ignoring creation of Certificate: A sufficient amount of Checkpoints has not been collected yet (collected: {:?}, required: {:?}).", checkpoint.origin, checkpoint.counter_latest_prep, checkpoint.total_amount_accepted_batches, amount_collected, self.config.t + 1);
            return;
        }
        if let Some(cert) = self
            .collector_checkpoints
            .retrieve_collected_checkpoints(&checkpoint, &self.config)
        {
            // The Replica can discard all entries in its log with a sequence number less than the counter value of its own Checkpoint.
            trace!(
                "Clearing message log by removing messages with a counter less than {:?}...",
                cert.my_checkpoint.counter()
            );

            let counter_last_prepare = cert.my_checkpoint.data.counter_latest_prep;
            let view_latest_prep = cert.my_checkpoint.data.view_latest_prep;

            if let Some(latest_delted_msg_counter) = self.sent_usig_msgs.iter().find(|u| match u {
                UsigMessage::View(ViewPeerMessage::Prepare(p)) => {
                    p.view == view_latest_prep && p.counter() == counter_last_prepare
                }
                UsigMessage::View(ViewPeerMessage::Commit(p)) => {
                    p.prepare.view == view_latest_prep
                        && p.prepare.counter() == counter_last_prepare
                }
                _ => false,
            }) {
                let counter = latest_delted_msg_counter.counter();
                self.sent_usig_msgs.retain(|x| x.counter() > counter);
            }

            trace!(
                "Successfully cleared message log by removing messages with a counter less than {:?}.",
                cert.my_checkpoint.counter()
            );
            let awaiting_recovery = self.awaiting_recovery_reply.clone();
            self.awaiting_recovery_reply = HashSet::new();

            for replica_id in awaiting_recovery {
                if replica_id == self.config.id {
                    continue;
                }

                let nonce = *self.recovering_replicas.get(&replica_id).unwrap();

                if let Some(cert_nonce) = cert.my_checkpoint.recovering_replicas.get(&replica_id) {
                    if *cert_nonce != nonce {
                        info!("Reached stable checkpoint, but it's unable for recovery of {:?}. Checkpoint formed before recovery started or replica started next recovery. Expected nonce {:?} but got {:?}", replica_id, cert_nonce, nonce);
                        self.awaiting_recovery_reply.insert(replica_id);
                        continue;
                    }
                } else {
                    info!("Reached stable checkpoint, but it's unable for recovery of {:?} with nonce {:?}. Checkpoint formed before recovery started.", replica_id, nonce);
                    self.awaiting_recovery_reply.insert(replica_id);
                    continue;
                }

                let msg = RecoveryReply::<P, U::Signature, U::Attestation> {
                    certificate: cert.clone(),
                    nonce,
                    storage: self.checkpoint_replica_storage.clone(),
                };
                info!("===========================================================");
                info!("Sending recovery reply for replica={:?}", replica_id);
                info!("===========================================================");

                output.send(msg, replica_id);
            }
            self.last_checkpoint_cert = Some(cert);
        }
    }
}
