use std::{
    borrow::Cow,
    cmp::Ordering,
    collections::{binary_heap::PeekMut, BinaryHeap, HashMap},
    fmt::Debug,
    iter,
};

use either::Either;
use tracing::{debug, trace};
use usig::{Count, Counter};

use crate::{peer_message::usig_message::UsigMessage, View};

/// Defines a Wrapper for messages of type UsigMessage.
#[derive(Debug, Clone)]
#[repr(transparent)]
struct UsigMessageWrapper<P, Sig, Att>(UsigMessage<P, Sig, Att>);

impl<P, Sig, Att> From<UsigMessageWrapper<P, Sig, Att>> for UsigMessage<P, Sig, Att> {
    /// Convert the given UsigMessageWrapper to a UsigMessage.
    fn from(usig_message_wrapper: UsigMessageWrapper<P, Sig, Att>) -> Self {
        usig_message_wrapper.0
    }
}

impl<P, Sig, Att> From<UsigMessage<P, Sig, Att>> for UsigMessageWrapper<P, Sig, Att> {
    /// Convert the given UsigMessage to a UsigMessageWrapper.
    fn from(usig_message: UsigMessage<P, Sig, Att>) -> Self {
        Self(usig_message)
    }
}

impl<P, Sig: Counter, Att> Counter for UsigMessageWrapper<P, Sig, Att> {
    /// Returns the counter of the UsigMessage.
    fn counter(&self) -> Count {
        self.0.counter()
    }
}

impl<P, Sig: Counter, Att> PartialEq for UsigMessageWrapper<P, Sig, Att> {
    /// Returns true if the counters of the UsigMessageWrappers are equal, otherwise false.
    fn eq(&self, other: &Self) -> bool {
        self.counter().eq(&other.counter())
    }
}

impl<P, Sig: Counter, Att> Eq for UsigMessageWrapper<P, Sig, Att> {}

impl<P, Sig: Counter, Att> PartialOrd for UsigMessageWrapper<P, Sig, Att> {
    /// Partially compares the counters of the UsigMessageWrappers.
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl<P, Sig: Counter, Att> Ord for UsigMessageWrapper<P, Sig, Att> {
    /// Compares the counters of the UsigMessageWrappers.
    fn cmp(&self, other: &Self) -> Ordering {
        self.counter().cmp(&other.counter()).reverse()
    }
}

/// Defines errors regarding the counter of UsigMessages.
enum CountCheckerError {
    /// Already seen UsigMessage based on its counter.
    AlreadySeen,
    /// Not the next expected UsigMessage based on its counter.
    NotNext,
}

/// The checker for counters of messages of type UsigMessage.
#[derive(Debug, Clone)]
struct CountChecker {
    /// The next expected Counter.
    next_count: Count,
}

impl CountChecker {
    /// Creates a new CountChecker.
    fn new() -> Self {
        Self {
            next_count: Count(0),
        }
    }

    /// Increments the next expected count if the given count is the expected one.
    fn next(&mut self, count: Count) -> Result<(), CountCheckerError> {
        match count.cmp(&self.next_count) {
            Ordering::Less => Err(CountCheckerError::AlreadySeen),
            Ordering::Equal => {
                self.next_count += 1;
                Ok(())
            }
            Ordering::Greater => Err(CountCheckerError::NotNext),
        }
    }
}

/// Defines the state of the UsigMessageHandler.
#[derive(Debug, Clone)]
pub(super) struct UsigMsgOrderEnforcer<P, Sig, Att> {
    /// Used for assuring the UsigMessages are handled and processed in the right order.
    count_checker: CountChecker,
    /// Collects already received, but yet to process UsigMessages
    /// (UsigMessages that have a smaller Counter are yet to be received).
    unprocessed: BinaryHeap<UsigMessageWrapper<P, Sig, Att>>, // TODO limit

    pending_new_views: HashMap<View, Vec<UsigMessage<P, Sig, Att>>>,
}

impl<P, Sig: Counter, Att> Default for UsigMsgOrderEnforcer<P, Sig, Att> {
    /// Creates a new default UsigMessageHandlerState.
    fn default() -> Self {
        Self {
            count_checker: CountChecker::new(),
            unprocessed: BinaryHeap::new(),
            pending_new_views: HashMap::new(),
        }
    }
}

impl<P: Clone + Debug, Sig: Counter + Clone + Debug, Att: Clone + Debug>
    UsigMsgOrderEnforcer<P, Sig, Att>
{
    /// Check if the given UsigMessage is the next one expected based on its
    /// counter.
    /// case 1: If the given UsigMessage is the next one expected, an Iterator
    /// is returned over the given UsigMessage and all other received messages
    /// of type UsigMessage that have yet to be processed and have counters that
    /// follow directly.
    /// case 2: If the given UsigMessage is not the one expected and was already
    /// seen, it is ignored.
    /// case 3: If the given UsigMessage is not the one expected and was not yet
    /// seen, it is stored as unprocessed.
    ///
    /// In cases 2 and 3 an empty Iterator is returned.
    pub(super) fn push_to_handle<'a>(
        &'a mut self,
        msg: Cow<'_, impl Into<UsigMessage<P, Sig, Att>> + Clone + Counter>,
    ) -> impl Iterator<Item = UsigMessage<P, Sig, Att>> + 'a {
        let msg = msg.into_owned();

        if let UsigMessage::NewView(new_view) = msg.clone().into() {
            let view = new_view.next_view;
            self.pending_new_views
                .entry(view)
                .or_default()
                .push(UsigMessage::NewView(new_view));
        }
        if let UsigMessage::ViewChange(view_change) = msg.clone().into() {
            let view = view_change.next_view;
            self.pending_new_views
                .entry(view)
                .or_default()
                .push(UsigMessage::ViewChange(view_change));
        }

        match self.count_checker.next(msg.counter()) {
            Ok(()) => {
                // we have the next message, so yield it and any messages that follow directly
                Either::Left(iter::once(msg.into()).chain(self.yield_to_be_processed()))
            }
            Err(e) => {
                match e {
                    CountCheckerError::AlreadySeen => {
                        // the given UsigMessage is an old one, so it is ignored
                        trace!("Ignoring already seen UsigMessage");
                    }
                    CountCheckerError::NotNext => {
                        // the given UsigMessage is not the next expected message, so it is put in the unprocessed heap
                        // (we might add a message mulitple times to the heap, but those get filtered out in yield_processed)
                        trace!("Storing UsigMessage for later processing");
                        self.unprocessed.push(msg.into().into())
                    }
                }
                Either::Right(iter::empty())
            }
        }
    }

    /// Returns to-be-processed UsigMessages.
    fn yield_to_be_processed(&mut self) -> impl Iterator<Item = UsigMessage<P, Sig, Att>> + '_ {
        iter::from_fn(|| {
            trace!("unprocessed: {:?}", self.unprocessed);
            while let Some(head) = self.unprocessed.peek_mut() {
                trace!("Checking UsigMessage for processing {:?}", head);
                match self.count_checker.next(head.counter()) {
                    Ok(()) => {
                        // we found the next message, so return it
                        trace!("Yielding UsigMessage for processing");
                        return Some(PeekMut::pop(head).into());
                    }
                    Err(CountCheckerError::AlreadySeen) => {
                        // the message is a duplicate, i.e. it was already seen
                        // pop, but ignore it
                        trace!("Ignoring already seen UsigMessage");
                        PeekMut::pop(head);
                        continue;
                    }
                    Err(CountCheckerError::NotNext) => {
                        trace!(
                            "Found a hole in the UsigMessage sequence {:?}",
                            head.counter()
                        );
                        // a hole was found, i.e. there are still missing UsigMessages
                        // therefore, end the iterator
                        return None;
                    }
                }
            }
            // no (more) to-be-processed messages, so end the iterator
            None
        })
    }

    pub(crate) fn get_pending_new_views(
        &mut self,
        current_view: View,
    ) -> Vec<UsigMessage<P, Sig, Att>> {
        let mut res = Vec::new();

        for (view, messages) in self.pending_new_views.iter() {
            if *view > current_view {
                res.append(&mut messages.clone());
            }
        }
        debug!(
            "pending new views {:?} {:?}",
            current_view, self.pending_new_views
        );
        res
    }

    /// Update the last seen counter after a unique [crate::Prepare] is accepted
    /// when processing a valid NewView.
    pub(crate) fn update_in_new_view(&mut self, counter_accepted_prep: Count) {
        while self.count_checker.next_count.0 <= counter_accepted_prep.0 {
            self.count_checker.next_count += 1;
            self.unprocessed.pop();
            trace!("Popped UsigMessage from unprocessed");
            trace!(
                "Updated next expected counter to {:?}",
                self.count_checker.next_count
            );
        }
    }
    pub(crate) fn update_after_finished_new_view(&mut self, new_view: View) {
        self.pending_new_views.retain(|view, _| *view > new_view);
    }

    pub(crate) fn update_after_recovered_counter(
        &mut self,
        counter_accepted_prep: Count,
    ) -> impl Iterator<Item = UsigMessage<P, Sig, Att>> + '_ {
        trace!(
            "Updating UsigMessageHandlerState after recovered counter. Before: {:?}",
            self.unprocessed.clone()
        );

        if self.count_checker.next_count.0 < counter_accepted_prep.0 {
            self.count_checker.next_count = counter_accepted_prep;
            trace!(
                "Updated next expected counter to {:?}",
                self.count_checker.next_count
            );
        }

        while let Some(head) = self.unprocessed.peek_mut() {
            if head.counter() < counter_accepted_prep {
                PeekMut::pop(head);
            } else {
                break;
            }
        }
        trace!(
            "Updating UsigMessageHandlerState after recovered counter. After: {:?}",
            self.unprocessed.clone()
        );

        self.yield_to_be_processed()
    }
}
