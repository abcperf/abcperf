//! Models the output that the replicas return when handling client requests,
//! peer messages, or timeouts.

use std::time::{Duration, Instant};

use shared_ids::{ClientId, ReplicaId};
use tracing::{error_span, info, trace};

use usig::{Usig, UsigError};

use crate::timeout::TimeoutAny;
use crate::{Config, Error, WrappedRequestPayload};

use crate::{
    peer_message::{usig_message::UsigMessage, ValidatedPeerMessage},
    timeout::Timeout,
    PeerMessage, RequestPayload,
};

/// The type that can only be constructed in this module.
/// This allows for trait functions that are only callable by this module.
pub(super) struct OutputRestricted(());

/// Contains the PeerMessages to be broadcasted.
type BroadcastList<Att, P, Sig> = Box<[PeerMessage<Att, P, Sig>]>;

/// Contains the PeerMessages to sent directly to a replica.
type DirectedList<Att, P, Sig> = Box<[(ReplicaId, PeerMessage<Att, P, Sig>)]>;

pub enum ViewInfo {
    InView(u64),
    ViewChange { from: u64, to: u64 },
}

/// Collects all the information a replica (of a system of multiple replicas
/// that together form the atomic broadcast) may generate when handling
/// client-requests, peer-messages or timeouts.
///
/// A replica may generate following output:
///
/// 1. Broadcasts to other participants
/// 2. Responses to client-requests
/// 3. Timeouts for messages of different kinds
/// 4. Various errors when handling client-requests, peer-messages or timeouts.
///
/// In addition, it keeps track of whether the participant is ready to receive
/// client requests and who the current primary participant is.
/// It also saves information on the current View and information on the round.
pub struct Output<P, U: Usig> {
    /// The messages to be broadcasted.
    pub broadcasts: BroadcastList<U::Attestation, P, U::Signature>,
    /// The messages to be sent directly to a specified replica.
    pub direct_messages: DirectedList<U::Attestation, P, U::Signature>,
    /// Contains the responses for the Clients (identified by their ClientId).
    #[allow(clippy::type_complexity)]
    pub responses: Box<[(ClientId, WrappedRequestPayload<P, U::Attestation>)]>,
    /// Contains the timeout requests.
    pub timeout_requests: Box<[TimeoutRequest]>,
    /// Contains the errors possibly returned upon the receival and processing of messages.
    pub errors: Box<[Error]>,
    /// True if the participant is ready to receive client requests, otherwise false.
    pub ready_for_client_requests: bool,
    /// The current primary if the participant is in the state InView.
    pub primary: Option<ReplicaId>,
    /// The information on the current View.
    pub view_info: ViewInfo,
    /// The information on the current round.
    pub round: u64,

    pub round_times: Vec<Instant>,
}

/// Collects all the non-reflected output, i.e. without own messages, a replica
/// (of a system of multiple replicas that together form the atomic broadcast)
/// may generate when handling client-requests, peer-messages or timeouts.
pub(super) struct NotReflectedOutput<P, U: Usig> {
    /// The messages to be broadcasted.
    broadcasts: Vec<ValidatedPeerMessage<U::Attestation, P, U::Signature>>,
    /// The messages to be sent directly to a specified replica.
    #[allow(clippy::type_complexity)]
    direct_messages: Vec<(
        ReplicaId,
        ValidatedPeerMessage<U::Attestation, P, U::Signature>,
    )>,
    /// Contains the responses for the Clients (identified by their ClientId).
    responses: Vec<(ClientId, WrappedRequestPayload<P, U::Attestation>)>,
    /// Contains the timeout requests.
    timeout_requests: Vec<TimeoutRequest>,
    /// Contains the errors possibly returned upon the receival and processing of messages.
    errors: Vec<Error>,
    /// True if the participant is ready for client requests, otherwise false.
    ready_for_client_requests: bool,

    round_times: Vec<Instant>,
}

/// Defines the trait of a participant being reflectable,
/// i.e. to be able to receive its own messages.
pub(super) trait Reflectable<P: RequestPayload, U: Usig> {
    /// Processes its own reflected message of type PeerMessage.
    fn process_reflected_peer_message(
        &mut self,
        peer_message: ValidatedPeerMessage<U::Attestation, P, U::Signature>,
        output: &mut NotReflectedOutput<P, U>,
        restricted: OutputRestricted,
    );

    /// Returns the current primary participant.
    fn current_primary(&self, restricted: OutputRestricted) -> Option<ReplicaId>;

    fn view_info(&self, restricted: OutputRestricted) -> ViewInfo;

    fn round(&self, restricted: OutputRestricted) -> u64;
}

impl<P: RequestPayload, U: Usig> NotReflectedOutput<P, U>
where
    U::Attestation: Clone,
    U::Signature: Clone,
{
    pub(super) fn new(_config: &Config, ready_for_client_requests: bool) -> Self {
        NotReflectedOutput {
            broadcasts: Vec::new(),
            direct_messages: Vec::new(),
            responses: Vec::new(),
            timeout_requests: Vec::new(),
            errors: Vec::new(),
            ready_for_client_requests,
            round_times: Vec::new(),
        }
    }

    /// Broadcast the given message of type PeerMessage.
    ///
    /// # Arguments
    ///
    /// * `message` - The message to be broadcast.
    /// * `message_log` - The log of sent USIG-signed messages.
    pub(super) fn broadcast(
        &mut self,
        message: impl Into<ValidatedPeerMessage<U::Attestation, P, U::Signature>>,
        message_log: &mut Vec<UsigMessage<P, U::Signature, U::Attestation>>,
    ) {
        let message = message.into();

        if let ValidatedPeerMessage::Usig(msg) = &message {
            message_log.push(msg.clone());
        }

        self.broadcasts.push(message);
    }

    pub(super) fn send(
        &mut self,
        message: impl Into<ValidatedPeerMessage<U::Attestation, P, U::Signature>>,
        receiver: ReplicaId,
    ) {
        let message = message.into();

        if let ValidatedPeerMessage::Usig(_msg) = &message {
            unimplemented!("Sending a USIG signed message only to one replica is not supported.")
        }

        self.direct_messages.push((receiver, message));
    }

    /// Collects the given response.
    ///
    /// # Arguments
    ///
    /// * `client_id` - The ID of the client for which a response is sent.
    /// * `output` - The output struct to be adjusted in case of, e.g., errors
    ///              or responses.
    pub(super) fn response(
        &mut self,
        client_id: ClientId,
        output: WrappedRequestPayload<P, U::Attestation>,
    ) {
        trace!(
            "Output response to client request (ID: {:?}, client ID: {:?}).",
            output.id(),
            client_id
        );
        self.responses.push((client_id, output));
    }

    /// Sets the given timeout.
    pub(super) fn timeout_request(&mut self, timeout_request: TimeoutRequest) {
        match &timeout_request {
            TimeoutRequest::Start(timeout) => {
                trace!(
                    "Output request for starting timeout (type: {:?}, duration: {:?}, stop class: {:?}).",
                    timeout.timeout_type, timeout.duration, timeout.stop_class
                );
            }
            TimeoutRequest::Stop(timeout) => {
                trace!(
                    "Output request for stopping timeout (type: {:?}, duration: {:?}, stop class: {:?}).",
                    timeout.timeout_type, timeout.duration, timeout.stop_class
                );
            }
            TimeoutRequest::StopAny(timeout) => {
                trace!(
                    "Output request for stopping timeout (type: {:?}, duration: {:?} ).",
                    timeout.timeout_type,
                    timeout.duration
                );
            }
        }
        self.timeout_requests.push(timeout_request);
    }

    /// Processes the given UsigError by parsing it to
    /// an OutputError and collecting it.
    ///
    /// # Arguments
    ///
    /// * `usig_error` - The USIG error.
    /// * `replica` - The ID of the replica for which the error occurred.
    /// * `msg_type` - The type of the message for which the error occured.
    pub(super) fn process_usig_error(
        &mut self,
        usig_error: UsigError,
        replica: ReplicaId,
        msg_type: &'static str,
    ) {
        let output_error = Error::Usig {
            replica,
            msg_type,
            usig_error,
        };
        self.error(output_error);
    }

    /// Collects the given OutputError.
    ///
    /// # Arguments
    ///
    /// * `output_error` - The error that occured and that should be collected
    ///                    in the output.
    pub(super) fn error(&mut self, output_error: Error) {
        self.errors.push(output_error);
    }

    /// Returns true if the participant is ready to receive client requests,
    /// otherwise false.
    pub(super) fn ready_for_client_requests(&mut self) {
        info!(
            "Replica is ready for client requests as sufficient Hello messages have been received."
        );
        self.ready_for_client_requests = true;
    }

    pub(super) fn log_round_change(&mut self) {
        self.round_times.push(Instant::now());
    }

    /// Receives and processes messages that the reflectable sent to itself.
    pub(super) fn reflect<R: Reflectable<P, U>>(mut self, reflectable: &mut R) -> Output<P, U> {
        let _minbft_span = error_span!("reflecting").entered();

        let mut last_len = 0;

        loop {
            let cur_len = self.broadcasts.len();
            if last_len == cur_len {
                break;
            }
            let messages: Vec<_> = self.broadcasts.iter().skip(last_len).cloned().collect();
            for message in messages {
                trace!(
                    "Processing reflected message (type {:?}) ...",
                    message.msg_type()
                );
                reflectable.process_reflected_peer_message(
                    message.clone(),
                    &mut self,
                    OutputRestricted(()),
                );
                trace!(
                    "Processed reflected message (type: {:?}).",
                    message.msg_type()
                );
            }
            last_len = cur_len;
        }

        let broadcast = self.broadcasts.into_iter().map(|m| m.into()).collect();
        let direct_messages = self
            .direct_messages
            .into_iter()
            .map(|(r, m)| (r, m.into()))
            .collect();

        let primary = reflectable.current_primary(OutputRestricted(()));
        let view_info = reflectable.view_info(OutputRestricted(()));
        let round = reflectable.round(OutputRestricted(()));

        Output {
            broadcasts: broadcast,
            direct_messages,
            responses: self.responses.into_boxed_slice(),
            timeout_requests: self.timeout_requests.into_boxed_slice(),
            errors: self.errors.into_boxed_slice(),
            ready_for_client_requests: self.ready_for_client_requests,
            primary,
            view_info,
            round,
            round_times: self.round_times,
        }
    }
}

/// A [TimeoutRequest] may be either a request to start a [Timeout] or to stop
/// it.
///
/// [crate::MinBft] outputs [TimeoutRequest]s when handling client requests or
/// peer messages as it is a partially asynchronous algorithm.
///
/// The [TimeoutRequest]s must be handled externally.
/// For further explanation, see [crate::MinBft].
#[derive(Debug, Clone)]
pub enum TimeoutRequest {
    Start(Timeout),
    Stop(Timeout),
    StopAny(TimeoutAny),
}

impl TimeoutRequest {
    /// Creates a new [TimeoutRequest::Start] for a [Timeout] of type Client with the given [ClientId] and duration.
    pub(crate) fn new_start_client_req(client_id: ClientId, duration: Duration) -> Self {
        Self::Start(Timeout::client(client_id, duration))
    }

    /// Creates a new [TimeoutRequest::Start] for a [Timeout] of type ViewChange with the given duration.
    pub(crate) fn new_start_view_change(duration: Duration) -> Self {
        Self::Start(Timeout::view_change(duration))
    }

    /// Creates a new [TimeoutRequest::Stop] for a [Timeout] of type Batch.
    pub(crate) fn new_stop_batch_req() -> Self {
        Self::Stop(Timeout::batch(Duration::from_secs(0)))
    }

    /// Creates a new [TimeoutRequest::Stop] for a [Timeout] of type Client with the given [ClientId].
    pub(crate) fn new_stop_client_req(client_id: ClientId) -> Self {
        Self::Stop(Timeout::client(client_id, Duration::from_secs(0)))
    }

    /// Creates a new [TimeoutRequest::Stop] for a [Timeout] of type ViewChange.
    pub(crate) fn new_stop_view_change() -> Self {
        Self::Stop(Timeout::view_change(Duration::from_secs(0)))
    }

    /// Creates a new [TimeoutRequest::Stop] for a [Timeout] of type Client with the given [ClientId].
    pub(crate) fn new_stop_any_client_req() -> Self {
        Self::StopAny(TimeoutAny::client(Duration::from_secs(0)))
    }
}
