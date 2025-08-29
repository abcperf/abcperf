use crate::{
    output::NotReflectedOutput,
    peer_message::usig_message::{view_peer_message::ViewPeerMessage, UsigMessage, UsigMessageV},
    ChangeInProgress, MinBft, Nonce, RequestPayload, ViewState,
};
use hashbar::Hashbar;
use serde::{Deserialize, Serialize};
use shared_ids::ReplicaId;
use std::fmt::Debug;
use tracing::{debug, trace, warn};
use usig::Usig;

impl<P: RequestPayload, U: Usig> MinBft<P, U>
where
    U::Attestation:
        Clone + Serialize + for<'a> Deserialize<'a> + PartialEq + Hashbar + 'static + Sync,
    U::Signature: Clone + Serialize + Hashbar + Debug,
{
    /// Process a message of type Forward.
    ///
    /// # Arguments
    ///
    /// * `from` - The ID of the replica from which the Hello originates.
    /// * `output` - The output struct to be adjusted in case of, e.g., errors
    ///              or responses.
    pub(crate) fn process_forward_message(
        &mut self,
        from: ReplicaId,
        nonce: Nonce,
        msg: UsigMessage<P, U::Signature, U::Attestation>,
        pending_new_views: Vec<UsigMessage<P, U::Signature, U::Attestation>>,
        output: &mut NotReflectedOutput<P, U>,
    ) {
        debug!("===========================================");

        if from == self.config.id {
            warn!("Received a forward message sent by self. Ignoring it.");
            return;
        }

        match self.recovering_nonce {
            None => {
                warn!("Received a forwward message but replica has never initiated recovery. Ignoring it.");
                return;
            }
            Some(recovering_nonce) => {
                if nonce != recovering_nonce {
                    warn!(
                        "Received a forward message for a different nonce. got={:?}, expected={:?}",
                        nonce, self.recovering_nonce
                    );
                    return;
                }
            }
        }

        match msg {
            UsigMessageV::View(view_msg) => {
                let origin = match &view_msg {
                    ViewPeerMessage::Commit(commit) => commit.origin,
                    ViewPeerMessage::Prepare(prepare) => prepare.origin,
                };

                if origin == self.config.id {
                    warn!("Received a forward message from of a message from self. Ignoring.");
                    return;
                }

                debug!(
                    "Received a forward message from {:?} {:?}: {:?}",
                    from, nonce, view_msg
                );
                let recovered_counters = self.collector_recovery_forwards.collect(
                    from,
                    view_msg,
                    pending_new_views,
                    &self.config,
                );
                debug!("recovered counters {:?}", recovered_counters);

                for (replica_id, (count, new_views)) in recovered_counters {
                    for usig in new_views {
                        match usig {
                            UsigMessageV::ViewChange(vc) => {
                                self.process_view_change(vc, output);
                            }
                            UsigMessageV::NewView(nv) => {
                                trace!("PROCESSING NEW VIEW FROM FORWARD: {:?}", nv);
                                if let ViewState::InView(in_view) = &self.view_state {
                                    self.view_state =
                                        ViewState::ChangeInProgress(ChangeInProgress {
                                            prev_view: in_view.view,
                                            next_view: nv.next_view,
                                            has_broadcast_view_change: false,
                                        });
                                }
                                self.process_new_view(nv, output);
                            }
                            _ => unreachable!(),
                        }
                    }

                    self.recheck_usig_msg_queue(replica_id, count, output);
                }

                debug!("===========================================");
            }
            _ => warn!("Received invalid forward message type {:?}", msg),
        }
    }
}
