use std::{borrow::Cow, fmt::Debug};

use hashbar::Hashbar;
use serde::Deserialize;
use serde::Serialize;
use tracing::debug;
use tracing::trace;
use tracing::warn;
use usig::Count;
use usig::Counter;
use usig::Usig;

use crate::output::NotReflectedOutput;
use crate::peer_message::usig_message::view_peer_message::ViewPeerMessage;
use crate::peer_message::usig_message::{UsigMessage, UsigMessageV};
use crate::peer_message::ValidatedPeerMessage;
use crate::MinBft;
use crate::WrappedRequestPayload;
use crate::{ReplicaId, RequestPayload, ViewState};

mod checkpoint_processor;
mod new_view_processor;
mod view_change_processor;
mod view_peer_processor;

impl<P: RequestPayload, U: Usig> MinBft<P, U>
where
    U::Attestation:
        Clone + Serialize + for<'a> Deserialize<'a> + PartialEq + Hashbar + 'static + Sync,
    U::Signature: Clone + Serialize + Debug + Hashbar,
{
    /// Process a [UsigMessage].
    /// At this stage, it is not guaranteed that previous messages of type
    /// UsigMessage have already been processed.
    /// This is ensured with a proceeding call.
    ///
    /// The steps are as follows:
    ///
    /// 1. If the [UsigMessage] is a nested one, process the inner messages
    ///     first.
    /// 2. If it is not a nested message, use the UsigMsgOrderEnforcer to
    ///     process the messages in order, i.e., from lowest USIG counter to highest
    ///     and without a hole.
    ///
    /// # Arguments
    ///
    /// * `usig_message` - The UsigMessage to be processed.
    /// * `output` - The output struct to be adjusted in case of, e.g., errors
    ///              or responses.
    pub(crate) fn process_usig_message(
        &mut self,
        usig_message: UsigMessage<P, U::Signature, U::Attestation>,
        output: &mut NotReflectedOutput<P, U>,
    ) {
        // case 1: The UsigMessage contains other messages (i.e. a nested UsigMessage is to be processed).

        // case 1.1: The UsigMessage is a Commit.
        if let UsigMessage::View(ViewPeerMessage::Commit(commit)) = &usig_message {
            trace!(
                "Processing inner message Prepare (origin: {:?}, view: {:?}, counter {:?}) by first passing it to the order enforcer ... (outer message is Commit [origin: {:?}, counter: {:?}])",
                commit.prepare.origin,
                commit.prepare.view,
                commit.prepare.counter(),
                commit.origin,
                commit.counter(),
            );
            let usig_prepare = &commit.prepare;
            let from = *usig_prepare.as_ref();
            let replica_state = &mut self.replicas_state[from.as_u64() as usize];
            let messages: Vec<_> = replica_state
                .usig_message_order_enforcer
                .push_to_handle(Cow::Borrowed(usig_prepare))
                .collect();
            for usig_inner_message in messages {
                self.process_usig_message_ordered(usig_inner_message, output);
            }
        }

        // case 1.2: The UsigMessage is a ViewChange.
        if let UsigMessage::ViewChange(view_change) = &usig_message {
            trace!("Processing inner messages of outer message ViewChange (origin: {:?}, next view: {:?}, counter: {:?}) by first passing them to the order enforcer ...", view_change.origin, view_change.next_view, view_change.counter());
            view_change
                .variant
                .message_log
                .iter()
                .for_each(|usig_inner_message_of_log| {
                    let from = *usig_inner_message_of_log.as_ref();
                    let replica_state = &mut self.replicas_state[from.as_u64() as usize];
                    let messages: Vec<_> = match usig_inner_message_of_log {
                        UsigMessageV::View(view_peer_message) => replica_state
                            .usig_message_order_enforcer
                            .push_to_handle(Cow::Borrowed(view_peer_message))
                            .collect(),
                        UsigMessageV::ViewChange(_) => Vec::new(),
                        UsigMessageV::NewView(new_view_message) => replica_state
                            .usig_message_order_enforcer
                            .push_to_handle(Cow::Borrowed(new_view_message))
                            .collect(),
                        UsigMessageV::Checkpoint(checkpoint_message) => replica_state
                            .usig_message_order_enforcer
                            .push_to_handle(Cow::Borrowed(checkpoint_message))
                            .collect(),
                    };
                    for usig_inner_message in messages {
                        self.process_usig_message_ordered(usig_inner_message, output);
                    }
                });
        }

        // case 1.3: The UsigMessage is a NewView.
        if let UsigMessage::NewView(new_view) = &usig_message {
            trace!("Processing inner messages of outer message NewView (origin: {:?}, next view: {:?}, counter: {:?}) by first passing them to the order enforcer ...", new_view.origin, new_view.next_view, new_view.counter());
            // process the other messages of type ViewChange

            for usig_inner_view_change_message in new_view.data.certificate.view_changes.iter() {
                self.process_usig_message(usig_inner_view_change_message.clone().into(), output);
            }
        }

        trace!(
            "Process message (origin: {:?}, type: {:?}, counter: {:?}) by first passing it to the order enforcer ...",
            *usig_message.as_ref(),
            usig_message.msg_type(),
            usig_message.counter()
        );

        // case 2: The UsigMessage does not contain other messages (i.e. a non-nested UsigMessage is to be processed).
        let from = *usig_message.as_ref();
        let replica_state = &mut self.replicas_state[from.as_u64() as usize];
        let messages: Vec<_> = replica_state
            .usig_message_order_enforcer
            .push_to_handle(
                Cow::<'_, UsigMessage<P, U::Signature, U::Attestation>>::Owned(usig_message),
            )
            .collect();

        for usig_inner_message_non_nested in messages {
            self.process_usig_message_ordered(usig_inner_message_non_nested, output);
        }
    }

    /// Process a [UsigMessage] in an ordered way.
    /// The messages passed are assumed to be ordered.
    fn process_usig_message_ordered(
        &mut self,
        usig_message: UsigMessage<P, U::Signature, U::Attestation>,
        output: &mut NotReflectedOutput<P, U>,
    ) {
        let origin = *usig_message.as_ref();
        let msg_type = usig_message.msg_type();
        let counter = usig_message.counter();
        trace!("Processing message (origin: {:?}, type: {:?}, counter: {:?}) completely as it has been selected by the order enforcer as the next one to be processed ...", origin, msg_type, counter);
        match usig_message {
            UsigMessage::View(view) => {
                let usig_message_copy = UsigMessage::View(view.clone());
                match &mut self.view_state {
                    ViewState::InView(in_view) => {
                        if view.view() == in_view.view {
                            match view {
                                ViewPeerMessage::Prepare(prepare) => {
                                    self.process_prepare(prepare, output);
                                }
                                ViewPeerMessage::Commit(commit) => {
                                    self.process_commit(commit, output);
                                }
                            }
                            trace!(
                                "Successfully processed message (origin: {:?}, type: {:?}, counter: {:?}) completely.",
                                origin,
                                msg_type,
                                counter
                            );
                        } else {
                            trace!("Ignoring ViewMessage from another view while in view.");
                        }
                    }
                    ViewState::ChangeInProgress(in_progress) => {
                        warn!("Processing message (origin: {:?}, type: {:?}, counter: {:?}) resulted in ignoring it: Replica is in progress of changing views (from {:?} to {:?}).", origin, msg_type, counter, in_progress.prev_view, in_progress.next_view);
                    }
                };

                let prepare = match &usig_message_copy {
                    UsigMessage::View(ViewPeerMessage::Prepare(prepare)) => prepare,
                    UsigMessage::View(ViewPeerMessage::Commit(commit)) => &commit.prepare,
                    _ => unreachable!(),
                };

                for request in prepare.request_batch.batch.iter() {
                    if let WrappedRequestPayload::Internal(recovery_request) = &request.payload {
                        if recovery_request.replica_id == self.config.id {
                            continue;
                        }
                        debug!("===========================================================");
                        debug!("AAAAbbbA Forwarding");
                        debug! {"{:?}", usig_message_copy}

                        let forward_msg = ValidatedPeerMessage::Forward(
                            recovery_request.nonce,
                            usig_message_copy.clone(),
                            self.replicas_state[origin.as_u64() as usize]
                                .usig_message_order_enforcer
                                .get_pending_new_views(prepare.view),
                        );
                        output.send(forward_msg, recovery_request.replica_id);
                        debug!("to {:?}", recovery_request.replica_id);
                        debug!("===========================================================");
                    }
                }
            }
            UsigMessage::ViewChange(view_change) => self.process_view_change(view_change, output),
            UsigMessage::NewView(new_view) => self.process_new_view(new_view, output),
            UsigMessage::Checkpoint(checkpoint) => self.process_checkpoint(checkpoint, output),
        }
    }

    pub(crate) fn recheck_usig_msg_queue(
        &mut self,
        replica_id: ReplicaId,
        counter: Count,
        output: &mut NotReflectedOutput<P, U>,
    ) {
        let mut to_handle = Vec::new();
        for msg in self
            .replicas_state
            .get_mut(replica_id.as_u64() as usize)
            .unwrap()
            .usig_message_order_enforcer
            .update_after_recovered_counter(counter)
        {
            to_handle.push(msg);
        }

        for msg in to_handle {
            debug!(
                "Handling pending message after recovering counter {:?}",
                msg
            );
            self.process_usig_message_ordered(msg, output);
        }
    }
}
