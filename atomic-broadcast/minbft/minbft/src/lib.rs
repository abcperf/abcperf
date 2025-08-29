//! Provides Byzantine fault-tolerant consensus while reducing the amount of
//! consenting nodes (replicas) required as much as possible.
//!
//! Based on the paper ["Efficient Byzantine Fault-Tolerance" by
//! Veronese et al](https://doi.org/10.1109/TC.2011.221), the crate provides an
//! implementation of a partially asynchronous Byzantine fault-tolerant atomic
//! broadcast (BFT) algorithm.
//! The algorithm requires n = 2t + 1 replicas in total, where t is the number
//! of faulty replicas.
//!
//! The intended way to use the library is to create an instance of the
//! struct [MinBft] for each replica, i.e. n instances.
//!
//! Upon setting up the connections between the replicas, instances of the
//! struct [MinBft] may receive and handle messages from clients,
//! messages from peers (other replicas/instances), or timeouts using the
//! respective function.
//!
//! The replicas must sign their peer messages with a Unique Sequential
//! Identifier Generator (USIG), as described in Section 2 of the paper above.
//! A USIG implementation compatible with this MinBFT implementation can be
//! found [here](https://github.com/abcperf/usig).
//! Note that this implementation does not use Trusted Execution Environments
//! and, thus, should not be used in untrusted environments.
//!
//! Timeouts must be handled explicitly by calling the respective function.
//! See the dedicated function below for further explanation.
//!
//! This implementation was created as part of the [ABCperf project](https://doi.org/10.1145/3626564.3629101).
//! An [integration in ABCperf](https://github.com/abcperf/demo) also exists.

use core::panic;
use std::cmp::Reverse;
use std::collections::{BinaryHeap, HashMap, HashSet};
use std::fmt;
use std::marker::PhantomData;
use std::time::Duration;
use std::{cmp::Ord, fmt::Debug, ops::Add};

use anyhow::Result;
pub use config::BackoffMultiplier;
pub use config::Config;
use derivative::Derivative;
pub use error::Error;
use hashbar::{Hashbar, Hasher};
pub use output::Output;
pub use peer_message::PeerMessage;
use peer_message_processor::collector::collector_checkpoints::CollectorCheckpoints;
use peer_message_processor::collector::collector_commits::CollectorCommits;
use peer_message_processor::collector::collector_forward_msg::CollectorForwards;
use peer_message_processor::collector::collector_req_view_changes::CollectorReqViewChanges;
use peer_message_processor::collector::collector_view_changes::CollectorViewChanges;
use rand::thread_rng;
use rand::Rng;
use request_processor::RequestProcessor;
use sha2::Sha256;
use shared_ids::{ClientId, ReplicaId, RequestId};
use timeout::TimeoutType;
use tracing::{debug, error, error_span, info, trace, warn};
use trait_alias_macro::pub_trait_alias_macro;
use usig::Count;
use usig::{Counter, Usig};

use serde::{Deserialize, Serialize};

use peer_message::{
    usig_message::{checkpoint::CheckpointCertificate, UsigMessage},
    ValidatedPeerMessage,
};
use usig_msg_order_enforcer::UsigMsgOrderEnforcer;

use crate::client_request::RequestBatch;
use crate::output::TimeoutRequest;
use crate::peer_message::usig_message::checkpoint::CheckpointHash;
use crate::request_batcher::{EMABatcher, RequestBatcher, TrivialRequestBatcher, AESC};
use crate::timeout::Timeout;
use crate::{
    client_request::ClientRequest,
    output::NotReflectedOutput,
    peer_message::{
        req_view_change::ReqViewChange,
        usig_message::view_peer_message::prepare::{Prepare, PrepareContent},
    },
};

pub mod config;
mod error;
mod peer_message;
mod peer_message_processor;
mod request_batcher;
mod usig_msg_order_enforcer;

pub mod id;
pub mod output;
pub mod timeout;

mod client_request;
mod request_processor;
#[cfg(test)]
mod tests;

pub type MinHeap<T> = BinaryHeap<Reverse<T>>;

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(bound = "")]
pub struct ReplicaStorage<P> {
    phantom_data: PhantomData<P>,
    last_hash: [u8; 32],
}

impl<P: RequestPayload> ReplicaStorage<P> {
    #[allow(clippy::new_without_default)]
    pub fn new() -> Self {
        Self {
            phantom_data: PhantomData,
            last_hash: [0; 32],
        }
    }
}

impl<P: RequestPayload> ReplicaStorage<P> {
    fn execute_request(&mut self, client_id: ClientId, request_id: RequestId, request: P) {
        let mut hasher = Sha256::new();
        use sha2::Digest;
        Digest::update(&mut hasher, self.last_hash);
        Digest::update(&mut hasher, client_id.as_u64().to_le_bytes());
        Digest::update(&mut hasher, request_id.as_u64().to_le_bytes());
        request.hash(&mut hasher);
        self.last_hash = hasher.finalize().into();
    }
    fn hash(&self) -> CheckpointHash {
        self.last_hash
    }
}

pub_trait_alias_macro!(RequestPayload = Clone + Serialize + for<'a> Deserialize<'a> + Debug + Hashbar + 'static + Send + Sync);

pub type Nonce = u128;

#[derive(Clone, Serialize, Deserialize, Debug, PartialEq, Eq, Hash)]
pub struct RecoveryRequestPayload<Att> {
    request_id: RequestId,
    replica_id: ReplicaId,
    attestation: Att,
    nonce: Nonce,
}

impl<Att: Hashbar> Hashbar for RecoveryRequestPayload<Att> {
    fn hash<H: Hasher>(&self, hasher: &mut H) {
        Hashbar::hash(&self.request_id, hasher);
        Hashbar::hash(&self.replica_id, hasher);
        self.attestation.hash(hasher);
        hasher.update(&self.nonce.to_le_bytes());
    }
}

#[derive(Clone, Serialize, Deserialize, Debug, PartialEq, Eq, Hash)]
pub enum WrappedRequestPayload<P, Att> {
    Client(RequestId, P),
    Internal(RecoveryRequestPayload<Att>),
}

impl<P, Att> WrappedRequestPayload<P, Att> {
    /// Returns the ID of the request.
    pub fn id(&self) -> RequestId {
        match self {
            WrappedRequestPayload::Client(id, _) => *id,
            WrappedRequestPayload::Internal(req) => req.request_id,
        }
    }
}

impl<P: Hashbar, Att: Hashbar> Hashbar for WrappedRequestPayload<P, Att> {
    fn hash<H: Hasher>(&self, hasher: &mut H) {
        match self {
            WrappedRequestPayload::Client(id, payload) => {
                hasher.update(&[0]);
                id.hash(hasher);
                payload.hash(hasher);
            }
            WrappedRequestPayload::Internal(req) => {
                hasher.update(&[1]);
                req.hash(hasher);
            }
        }
    }
}

/// Defines the current view,
/// i.e. the primary replica of a system of multiple replicas
/// that together form an atomic broadcast.
///
/// The view is, therefore, in charge of generating Prepares and
/// batching them when creating a response to a client-request.
#[derive(
    Serialize, Deserialize, Debug, Clone, Copy, Ord, Eq, PartialEq, PartialOrd, Default, Hash,
)]
struct View(u64);

impl Hashbar for View {
    fn hash<H: hashbar::Hasher>(&self, hasher: &mut H) {
        hasher.update(&self.0.to_le_bytes());
    }
}

impl Add<u64> for View {
    type Output = Self;

    /// Defines the addition of a view with an unsigned integer.
    fn add(self, rhs: u64) -> Self::Output {
        Self(self.0 + rhs)
    }
}

impl fmt::Display for View {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "({0})", self.0)
    }
}

/// Defines the state of the view for a replica.
/// The state is to be set to this enum type when the view is non-faulty.
#[derive(Debug)]
struct InView<P, Sig, Att> {
    /// The current non-faulty view.
    view: View,
    /// True if the replica sent a message of type ReqViewChange, otherwise false.
    has_requested_view_change: bool,
    /// Collects messages of type Commit (and the corresponding Prepare).
    collector_commits: CollectorCommits<P, Sig, Att>,
}

/// Defines the state of the view for a replica when changing Views.
#[derive(Debug)]
struct ChangeInProgress {
    /// The previous view which turned out to be faulty.
    prev_view: View,
    /// The next view to be changed to.
    next_view: View,
    /// True if a message of type ViewChange has already been broadcast.
    /// Necessary to ensure exactly one ViewChange (from prev_view to next_view) is broadcast.
    has_broadcast_view_change: bool,
}

/// Defines the possible view states.
/// Either a replica is in the state of a functioning view or
/// in the state of changing views.
#[derive(Debug)]
enum ViewState<P, Sig, Att> {
    /// The current view is functioning expectedly.  
    InView(InView<P, Sig, Att>),
    /// A view-change is being performed.
    ChangeInProgress(ChangeInProgress),
}

impl<P: Clone, Sig: Counter + Clone, Att: Clone> ViewState<P, Sig, Att> {
    /// Creates a ViewState with the default initial values (state is InView).
    fn new() -> Self {
        Self::InView(InView {
            view: View::default(),
            has_requested_view_change: false,
            collector_commits: CollectorCommits::new(),
        })
    }
}

/// Defines the state of a replica.
#[derive(Clone, Debug, Derivative)]
#[derivative(Default(bound = "Sig: Counter"))]
struct ReplicaState<P, Sig, Att> {
    usig_message_order_enforcer: UsigMsgOrderEnforcer<P, Sig, Att>,
}

/// Defines a replica of a system of multiple replicas
/// that together form an atomic broadcast.
///
/// This is the main component of the crate.
/// A replica may receive client-requests, messages from other replicas
/// (peer messages) or timeouts.
/// It may send peer messages, too.
///
/// # Examples
///
/// 1. The replicas have to first initiate the communication between
///     each other by broadcasting the initial messages in the returned
///     [Output] struct when creating messages.
/// 2. The return values of the public functions ([Output]) are to be
///     handled equally.
///
/// ```no_run
/// use anyhow::Result;
/// use std::time::Duration;
/// use serde::{Deserialize, Serialize};
///
/// use shared_ids::{ReplicaId, ClientId, RequestId};
/// use usig::{Usig, noop::UsigNoOp};
///
/// use minbft::{MinBft, Config, Output, RequestPayload, PeerMessage, timeout::{TimeoutType}};
///
/// // The payload of a client request must be implemented by the user.
/// #[derive(Deserialize, Serialize, Clone, Debug, Eq, PartialEq)]
/// struct SamplePayload {}
/// impl hashbar::Hashbar for SamplePayload {
///     fn hash<H: hashbar::Hasher>(&self, hasher: &mut H) {}
/// }
///
/// // The output should be handled by the user.
/// fn handle_output<U: Usig>(output: Output<SamplePayload, U>) {
///     let Output { broadcasts, responses, timeout_requests, .. } = output;
///     for broadcast in broadcasts.iter() {
///         // broadcast message to all peers, i.e., send messages contained
///         // in `broadcasts` of struct Output over the network to other
///         // replicas.
///         todo!();
///     }
///     for response in responses.iter() {
///         todo!();
///     }
///     for timeout_request in timeout_requests.iter() {
///         todo!();
///     }
/// }
///
/// let (mut minbft, output) = MinBft::<SamplePayload, _>::new(
///         UsigNoOp::default(),
///         Config {
///             ..todo!() // see the struct [Config].
///         },
///         false,
///     )
///     .unwrap();
///
/// // handle output to setup connection with peers
/// handle_output(output);
///
/// // all peers are now ready for client requests as they have
/// // successfully communicated with each replica for the first time.
///
/// let some_client_message: SamplePayload = todo!();
/// let output = minbft.handle_client_message(ClientId::from_u64(0), RequestId::from_u64(0), some_client_message);
/// handle_output(output);
///
/// let some_peer_message: PeerMessage<_, SamplePayload, _> = todo!();
/// let output = minbft.handle_peer_message(ReplicaId::from_u64(0), some_peer_message);
/// handle_output(output);
///
/// let some_timeout: (TimeoutType) = todo!();
/// let output = minbft.handle_timeout(some_timeout);
/// handle_output(output);
/// ```
pub struct MinBft<P: RequestPayload, U: Usig>
where
    U::Attestation: Clone + for<'a> Deserialize<'a>,
    U::Signature: Clone + Serialize,
    U::Signature: Debug,
{
    /// Used for USIG-signing messages of type UsigMessage.
    usig: U,

    /// Contains the configuration parameters for the algorithm.
    config: Config,

    /// Used for processing client-requests.
    request_processor: RequestProcessor<P, U>,

    /// Contains the UsigMessages that the replica created itself and broadcast.
    sent_usig_msgs: Vec<UsigMessage<P, U::Signature, U::Attestation>>,

    /// Contains the state used to track each replica.
    replicas_state: Vec<ReplicaState<P, U::Signature, U::Attestation>>,

    /// Either the state of the current view or the view-change state.
    view_state: ViewState<P, U::Signature, U::Attestation>,

    /// The counter of the last accepted Prepare.
    /// When the view changes, the struct field is set to the counter of the last UsigMessage sent by the new primary.
    /// This can be either the counter of the NewView or the counter of the last generated Checkpoint.
    counter_last_accepted_prep: Option<Count>,

    /// The collector of messages of type ReqViewChange.
    /// Collects ReqViewChanges filtered by their previous and next view.
    collector_rvc: CollectorReqViewChanges,

    /// The collector of messages of type ViewChange.
    /// Collects ViewChanges filtered by their next view.
    collector_vc: CollectorViewChanges<P, U::Signature, U::Attestation>,

    collector_recovery_forwards: CollectorForwards<P, U::Signature, U::Attestation>,

    /// Contains Checkpoints that together form a valid certificate.
    /// At the beginning, the struct field is set to None.
    /// Checkpoints certificates are generated periodically
    /// in order to clear the collection of sent UsigMessages (see struct field sent_usig_msgs).
    last_checkpoint_cert: Option<CheckpointCertificate<U::Signature, U::Attestation>>,

    /// Contains currently received Checkpoints.
    /// Creates a certificate when sufficient valid Checkpoints have been collected.
    /// See the type of the struct itself for more intel.
    collector_checkpoints: CollectorCheckpoints<U::Signature, U::Attestation>,

    /// Allows to increase the duration of timeouts exponentially.
    current_timeout_duration: Duration,

    replica_storage: ReplicaStorage<P>,
    checkpoint_replica_storage: ReplicaStorage<P>,

    pub recovering: bool,
    pub recovering_nonce: Option<Nonce>,
    #[allow(clippy::type_complexity)]
    recovery_unverified_usig_messages:
        Vec<(ReplicaId, PeerMessage<U::Attestation, P, U::Signature>)>,
    recovering_replicas: HashMap<ReplicaId, Nonce>,
    awaiting_recovery_reply: HashSet<ReplicaId>,
}

impl<P: RequestPayload, U: Usig> MinBft<P, U>
where
    U::Attestation:
        Clone + Serialize + for<'a> Deserialize<'a> + PartialEq + Hashbar + 'static + Sync,
    U::Signature: Clone + Serialize + Hashbar,
{
    /// Creates a new replica of a system of multiple replicas
    /// that together form an atomic broadcast.
    ///
    /// # Arguments
    ///
    /// * `usig` - The USIG signature of the [MinBft].
    /// * `config` - The configuration of the [MinBft].
    ///
    /// # Return Value
    ///
    /// A tuple consisting of the [MinBft] instance and its [Output].
    pub fn new(usig: U, config: Config, recovering: bool) -> Result<(Self, Output<P, U>)> {
        let _minbft_span = error_span!("minbft", id = config.id.as_u64()).entered();

        config.validate();
        let replica_storage = ReplicaStorage::new();
        let checkpoint_replica_storage = ReplicaStorage::new();

        let request_batcher = config.batcher_configuration.init();

        let mut minbft = Self {
            replicas_state: config
                .all_replicas()
                .map(|_| ReplicaState::default())
                .collect(),
            last_checkpoint_cert: None,
            sent_usig_msgs: Vec::new(),
            usig,
            request_processor: RequestProcessor::<P, U>::new(request_batcher),
            view_state: ViewState::new(),
            counter_last_accepted_prep: None,
            collector_rvc: CollectorReqViewChanges::new(),
            collector_vc: CollectorViewChanges::new(),
            collector_recovery_forwards: CollectorForwards::new(&config),
            collector_checkpoints: CollectorCheckpoints::new(),
            current_timeout_duration: config.initial_timeout_duration,
            config,
            replica_storage,
            checkpoint_replica_storage,
            recovering,
            recovering_nonce: None,
            recovery_unverified_usig_messages: Vec::new(),
            awaiting_recovery_reply: HashSet::new(),
            recovering_replicas: HashMap::new(),
        };
        let output = if recovering {
            minbft.create_recovery_request()?
        } else {
            minbft.attest()?
        };
        let output = output.reflect(&mut minbft);
        Ok((minbft, output))
    }

    /// Returns the ID of the current primary, i.e., the one replica who creates
    /// Prepares for client requests, or [None] if the replica is in the
    /// inner state of changing views.
    pub fn primary(&self) -> Option<ReplicaId> {
        match &self.view_state {
            ViewState::InView(v) => Some(self.config.primary(v.view)),
            ViewState::ChangeInProgress(_) => None,
        }
    }

    /// Handles a message from a client.
    ///
    /// If the replica is in the state of changing views, the client-message is
    /// ignored.
    /// Otherwise, the client-message is not ignored, but undergoes several
    /// checks:
    ///
    /// 1. The request is verified regarding its validity and its age.
    ///    Should the request be too old, it is ignored.
    ///    Otherwise, and in case the replica is the current primary,
    ///    a message of type Prepare is broadcast to all replicas.
    /// 2. A timeout is set for the client-request.
    /// 3. In case batching is on, a timeout for the batch is set, too.
    ///
    /// # Arguments
    ///
    /// * `client_id` - The ID of the client from which the request originates.
    /// * `request` - The client request to be handled.
    ///
    /// # Return Value
    ///
    /// The adjusted [Output] containing relevant information regarding the
    /// handling of the client request, e.g., the response or errors.
    ///
    /// # Example
    ///
    /// ```no_run
    /// use anyhow::Result;
    /// use serde::{Serialize, Deserialize};
    ///
    /// use minbft::{MinBft, Config, RequestPayload};
    /// use usig::noop::UsigNoOp;
    /// use shared_ids::{RequestId, ClientId};
    ///
    /// #[derive(Deserialize, Serialize, Clone, Debug, Eq, PartialEq)]
    /// struct SamplePayload {}
    /// impl hashbar::Hashbar for SamplePayload {
    ///     fn hash<H: hashbar::Hasher>(&self, hasher: &mut H) {}
    /// }
    ///
    /// let (mut minbft, output) = MinBft::<SamplePayload, _>::new(
    ///        UsigNoOp::default(),
    ///        Config {
    ///            ..todo!() // [MinBft]
    ///        },
    ///        false,
    ///    )
    ///    .unwrap();
    /// // handle output, see [MinBft]
    ///
    /// let some_client_message: SamplePayload = todo!();
    /// let output = minbft.handle_client_message(ClientId::from_u64(0), RequestId::from_u64(0), some_client_message);
    /// //TODO assert_eq!(output.responses[0], (ClientId::from_u64(0), some_client_message));
    /// // handle output, see [MinBft]
    /// ```
    pub fn handle_client_message(
        &mut self,
        client_id: ClientId,
        request_id: RequestId,
        request: P,
    ) -> Output<P, U> {
        self.handle_wrapped_client_message(
            client_id,
            WrappedRequestPayload::Client(request_id, request),
        )
    }
    pub fn handle_wrapped_client_message(
        &mut self,
        client_id: ClientId,
        request: WrappedRequestPayload<P, U::Attestation>,
    ) -> Output<P, U> {
        let mut output = NotReflectedOutput::new(&self.config, self.ready_for_client_requests());
        self.handle_internal_client_message(client_id, request, &mut output);
        output.reflect(self)
    }

    fn handle_internal_client_message(
        &mut self,
        client_id: ClientId,
        request: WrappedRequestPayload<P, U::Attestation>,
        output: &mut NotReflectedOutput<P, U>,
    ) {
        let _minbft_span = error_span!("minbft", id = self.config.id.as_u64()).entered();

        let req_id = request.id();

        trace!(
            "Handling client request (ID: {:?}, client ID: {:?}) ...",
            req_id,
            client_id
        );

        // Create output in order to return information regarding the handling of the client-message.

        // The payload of a client-request is forced to have a function that verifies itself.
        // It must be valid, otherwise it is not handled further.
        // Errors are stored in the output variable.
        trace!(
            "Verifying client request (ID {:?}, client ID: {:?}) ...",
            req_id,
            client_id
        );
        trace!(
            "Successfully verified client request (ID: {:?}, client ID: {:?}).",
            req_id,
            client_id
        );

        let client_request = ClientRequest {
            client: client_id,
            payload: request,
        };

        let (start_client_timeout, prepare_content, batch_timeout_request) =
            self.request_processor.process_client_req(
                client_request,
                &self.view_state,
                self.current_timeout_duration,
                &self.config,
            );

        if let Some(client_timeout) = start_client_timeout {
            output.timeout_request(client_timeout);
        }

        if matches!(self.view_state, ViewState::InView(_)) {
            self.process_batching_output(prepare_content, batch_timeout_request, output);
        }
    }

    /// Handles a message of type PeerMessage.
    ///
    /// A message of type PeerMessage is a message from another replica.
    ///
    /// # Arguments
    ///
    /// * `from` - The ID of the replica from which the message originates.
    /// * `message` - The message of a peer (another replica) to be handled.
    ///
    /// The replica handles the message differently, depending on its concrete type.
    /// If the message is valid, it may trigger cascading events, i.e. the
    /// replica itself may broadcast a message in response to receiving this one,
    /// all depending on its inner state and the message's type. \
    /// Messages that are USIG-signed are guaranteed to be handled in correct
    /// order, i.e. messages with a lower count received from a specific replica
    /// are handled before messages with a higher count received from the same
    /// replica.
    ///
    /// # Return Value
    ///
    /// The adjusted [Output] containing relevant information regarding the
    /// handling of the peer message, e.g., response messages or errors.
    ///
    /// # Example
    ///
    /// ```no_run
    /// use serde::{Serialize, Deserialize};
    ///
    /// use minbft::{MinBft, Config, PeerMessage, RequestPayload};
    /// use usig::noop::UsigNoOp;
    /// use shared_ids::{RequestId, ClientId};
    /// use anyhow::Result;
    ///
    /// #[derive(Deserialize, Serialize, Clone, Debug, Eq, PartialEq)]
    /// struct SamplePayload {}
    /// impl hashbar::Hashbar for SamplePayload {
    ///     fn hash<H: hashbar::Hasher>(&self, hasher: &mut H) {}
    /// }
    ///
    /// let (mut minbft_0, output) = MinBft::<SamplePayload, _>::new(
    ///        UsigNoOp::default(),
    ///        Config {
    ///            ..todo!() // [MinBft]
    ///        },
    ///        false,
    ///    )
    ///    .unwrap();
    /// // handle output, see [MinBft]
    ///
    /// let (mut minbft_1, output) = MinBft::<SamplePayload, _>::new(
    ///        UsigNoOp::default(),
    ///        Config {
    ///            ..todo!() // [MinBft]
    ///        },
    ///        false,
    ///    )
    ///    .unwrap();
    /// // handle output, see [MinBft]
    ///
    /// let some_peer_message: PeerMessage<_, SamplePayload, _> = todo!();
    /// // message is sent over network (it is serialized and deserialized)
    /// let output = minbft_0.handle_peer_message(todo!(), some_peer_message);
    /// // handle output, see [MinBft]
    /// ```
    pub fn handle_peer_message(
        &mut self,
        from: ReplicaId,
        message: PeerMessage<U::Attestation, P, U::Signature>,
    ) -> Output<P, U> {
        let _minbft_span = error_span!("minbft", id = self.config.id.as_u64()).entered();

        let msg_type = message.msg_type();

        debug!(
            "Handling message (origin: {:?}, type: {:?}) ...",
            from, msg_type,
        );

        assert_ne!(from, self.config.me());
        assert!(from.as_u64() < self.config.n.get());
        let mut output = NotReflectedOutput::new(&self.config, self.ready_for_client_requests());

        let message_copy = message.clone();
        let message = match message.validate(from, &self.config, &mut self.usig) {
            Ok(message) => message,
            Err(output_inner_error) => {
                if self.recovering {
                    debug!(
                        "Replica is currently recovering. Storing message for later processing."
                    );
                    self.recovery_unverified_usig_messages
                        .push((from, message_copy));
                } else {
                    output.error(output_inner_error.into());
                }
                return output.reflect(self);
            }
        };
        self.process_peer_message(from, message, &mut output);
        trace!(
            "Successfully handled message (origin: {:?}, from {:?}).",
            from,
            msg_type
        );

        output.reflect(self)
    }

    /// Handles a timeout according to their type.
    ///
    /// This function assumes no old timeouts are passed as parameters.
    ///
    /// Replicas may send timeout requests via [Output].
    /// Consequently, the timeout requests ([output::TimeoutRequest]) must be handled explicitly.
    /// This means, whenever a request to start a timeout is sent, it must be checked
    /// if there is already a timeout of the same type running, the request should be ignored.
    /// Whenever a request to stop a timeout is sent, a set timeout should only be stopped
    /// if the stop class and the type are the same.
    ///
    /// Set timeouts must be handled explicitly.
    ///
    /// They are handled differently depending on their type.
    ///
    /// A timeout for a batch is only handled if the primary is non-faulty
    /// from the standpoint of the replica.
    /// Is this the case, then a message of type Prepare is created
    /// and broadcast for the next batch of client-requests.
    ///
    /// A timeout for a client-request is only handled if the primary is non-faulty
    /// from the standpoint of the replica.
    /// Is this the case, then a view-change is requested.
    ///
    /// A timeout for a view-change is only handled if the replica is currently
    /// in the state of changing views.
    /// Is this the case, then a view-change is requested.
    ///
    /// # Arguments
    ///
    /// * `timeout_type` - The type of the timeout to be handled.
    ///
    /// # Return Value
    ///
    /// The adjusted [Output] containing relevant information regarding the
    /// handling of the peer message, e.g., response messages or errors.
    ///
    /// # Example
    ///
    /// ```no_run
    /// use serde::{Serialize, Deserialize};
    /// use anyhow::Result;
    ///
    /// use minbft::{MinBft, Config, RequestPayload, output::TimeoutRequest::{Start, Stop, StopAny}};
    /// use shared_ids::{ClientId, RequestId};
    /// use usig::noop::UsigNoOp;
    ///
    /// #[derive(Deserialize, Serialize, Clone, Debug, Eq, PartialEq)]
    /// struct SamplePayload {}
    /// impl hashbar::Hashbar for SamplePayload {
    ///     fn hash<H: hashbar::Hasher>(&self, hasher: &mut H) {}
    /// }
    ///
    ///
    /// let (mut minbft, output) = MinBft::<SamplePayload, _>::new(
    ///        UsigNoOp::default(),
    ///        Config {
    ///            ..todo!() // [MinBft]
    ///        },
    ///        false,
    ///    )
    ///    .unwrap();
    /// // handle output, see [MinBft]
    ///
    /// let some_client_message: SamplePayload = todo!();
    /// let output = minbft.handle_client_message(ClientId::from_u64(0), RequestId::from_u64(0), some_client_message);
    ///
    /// for timeout_request in output.timeout_requests.iter() {
    ///     match timeout_request {
    ///         Start(timeout) => {
    ///             // check if there is no timeout of the same type already set
    ///             // sleep for the duration of the timeout
    ///             minbft.handle_timeout(timeout.timeout_type);
    ///         }
    ///         Stop(timeout) => {
    ///             // if there is already a timeout set of the same type and
    ///             // stop class, stop it
    ///         }
    ///         StopAny(timeout) => {
    ///             // if there is already a timeout set of the same type and
    ///             // stop class, stop it
    ///         }
    ///     }
    /// }
    /// ```
    pub fn handle_timeout(&mut self, timeout_type: TimeoutType) -> Output<P, U> {
        let _minbft_span = error_span!("minbft", id = self.config.id.as_u64()).entered();
        debug!("Handling timeout (type: {:?}) ...", timeout_type);
        let mut output = NotReflectedOutput::new(&self.config, self.ready_for_client_requests());

        match timeout_type {
            TimeoutType::Batch => match &self.view_state {
                ViewState::InView(_in_view) => {
                    let (maybe_batch, stop_timeout_request) =
                        self.request_processor.request_batcher.timeout();

                    self.process_batching_output(maybe_batch, stop_timeout_request, &mut output);
                }
                ViewState::ChangeInProgress(in_progress) => {
                    warn!("Handling timeout resulted in skipping creation of Prepare for timed out batch: Replica is in progress of changing views (from: {:?}, to: {:?}).", in_progress.prev_view, in_progress.next_view);
                }
            },
            TimeoutType::Client => match &mut self.view_state {
                ViewState::InView(in_view) => {
                    warn!("Client request timed out.");
                    if !in_view.has_requested_view_change {
                        in_view.has_requested_view_change = true;
                        self.current_timeout_duration *= self.config.backoff_multiplier as u32;
                        let msg = ReqViewChange {
                            prev_view: in_view.view,
                            next_view: in_view.view + 1,
                        };
                        info!(
                            "Broadcast ReqViewChange (previous view: {:?}, next view: {:?}).",
                            msg.prev_view, msg.next_view
                        );
                        output.broadcast(msg, &mut self.sent_usig_msgs)
                    } else {
                        trace!("Already broadcast ReqViewChange (previous view: {:?}, next view: {:?}).", in_view.view, in_view.view + 1);
                    }
                    trace!("Successfully handled timeout (type: {:?}).", timeout_type);
                }
                ViewState::ChangeInProgress(in_progress) => {
                    warn!("Handling timeout resulted in skipping creation of ReqViewChange: Replica is in progress of changing views (from: {:?}, to: {:?}).", in_progress.prev_view, in_progress.next_view);
                }
            },
            TimeoutType::ViewChange => match &mut self.view_state {
                ViewState::InView(in_view) => {
                    warn!("Handling timeout resulted in skipping creation of ReqViewChange: Replica is in view ({:?}).", in_view.view);
                }
                ViewState::ChangeInProgress(in_progress) => {
                    warn!("View-Change timed out.");
                    self.current_timeout_duration *= self.config.backoff_multiplier as u32;
                    in_progress.has_broadcast_view_change = false;

                    let msg = ReqViewChange {
                        prev_view: in_progress.prev_view,
                        next_view: in_progress.next_view + 1,
                    };
                    info!(
                        "Broadcast ReqViewChange (previous view: {:?}, next view: {:?}).",
                        msg.prev_view, msg.next_view
                    );
                    output.broadcast(msg, &mut self.sent_usig_msgs);
                    trace!("Successfully handled timeout (type: {:?}).", timeout_type);
                }
            },
        }
        output.reflect(self)
    }

    fn process_batching_output(
        &mut self,
        maybe_batch: Option<RequestBatch<P, U::Attestation>>,
        timeout: Option<Timeout>,
        output: &mut NotReflectedOutput<P, U>,
    ) {
        //TODO: Clear batching state of primary change
        if let Some(timeout) = timeout {
            output.timeout_request(TimeoutRequest::new_stop_batch_req());
            output.timeout_request(TimeoutRequest::Start(timeout));
        }
        let origin = self.config.me();
        if let Some(batch) = maybe_batch {
            let in_view = match &self.view_state {
                ViewState::InView(in_view) => in_view,
                ViewState::ChangeInProgress(_) => {
                    unreachable!();
                }
            };
            if self.config.me_primary(in_view.view) {
                trace!("Creating Prepare for timed out batch ...");
                match Prepare::sign(
                    PrepareContent {
                        view: in_view.view,
                        origin,
                        request_batch: batch,
                    },
                    &mut self.usig,
                ) {
                    Ok(prepare) => {
                        trace!("Successfully created Prepare for timed-out batch.");
                        trace!(
                            "Broadcast Prepare (view: {:?}, counter: {:?}) for timed-out batch.",
                            prepare.view,
                            prepare.counter()
                        );
                        output.broadcast(prepare, &mut self.sent_usig_msgs);
                    }
                    Err(usig_error) => {
                        error!("Failed to create Prepare for batch: {:?}", usig_error);
                        output.process_usig_error(usig_error, origin, "Prepare");
                    }
                };
            } else {
                trace!("Ignoring timed out batch as replica is no longer the primary (current View: {})", in_view.view);
            }
        }
    }

    fn handle_recovery_request(
        &mut self,
        client_id: ClientId,
        request: RecoveryRequestPayload<U::Attestation>,
    ) {
        let RecoveryRequestPayload {
            request_id: _,
            replica_id,
            attestation,
            nonce,
        } = &request;

        if client_id.as_u64() != replica_id.as_u64() {
            warn!("Ignoring recovery request from client because client_id does not match request sender {:?} {:?}", client_id, request);
            return;
        }
        if *replica_id == self.config.id {
            warn!("Ignoring recovery request self.");
            return;
        }

        if !self
            .request_processor
            .recv_attestations
            .contains_key(replica_id)
        {
            warn!(
                "Ignoring recovery request from unknown replica {:?} {:?}",
                client_id, request
            );
            return;
        }

        info!("===========================================================");
        info!(
            "Handling recovery request from replica={:?} {:?} {:?}",
            replica_id, request, nonce
        );

        if !self.usig.add_remote_party(*replica_id, attestation.clone()) {
            warn!("Ignoring recovery request because it's attestation was rejected by the USIG service {:?} {:?}", client_id, request);
        }
        self.request_processor
            .recv_attestations
            .insert(*replica_id, attestation.clone());

        info!("===========================================================");

        let replica_state: &mut ReplicaState<P, U::Signature, U::Attestation> = self
            .replicas_state
            .get_mut(replica_id.as_u64() as usize)
            .unwrap();
        replica_state.usig_message_order_enforcer = UsigMsgOrderEnforcer::default();

        self.recovering_replicas.insert(*replica_id, *nonce);
        self.awaiting_recovery_reply.insert(*replica_id);
    }

    fn handle_recovery_reply(
        &mut self,
        nonce: Nonce,
        certificate: CheckpointCertificate<U::Signature, U::Attestation>,
        storage: ReplicaStorage<P>,
        output: &mut NotReflectedOutput<P, U>,
    ) {
        info!("===========================================================");
        info!("Handling recovery reply {:?}", nonce);
        info!("===========================================================");

        let cert = &certificate.my_checkpoint;

        if !self.recovering {
            info!("Ignoring recovery reply because replica is not recovering");
            return;
        }

        if self.recovering_nonce.unwrap() != nonce {
            warn!(
                "Ignoring recovery reply because nonce does not match. expected={:?}, got={:?}",
                self.recovering_nonce.unwrap(),
                nonce
            );
            return;
        }

        for (replica_id, attestation) in &cert.usig_attestations {
            if !self.usig.add_remote_party(*replica_id, attestation.clone()) {
                panic!("received incorrect usig attestation in recovery reply");
                //TODO
            }
        }

        self.counter_last_accepted_prep = Some(cert.counter_latest_prep);
        self.view_state = ViewState::InView(InView {
            view: cert.view_latest_prep,
            has_requested_view_change: false,
            collector_commits: CollectorCommits::new(),
        });
        let request_batcher = self.config.batcher_configuration.init();

        //self.replicas_state = self.config.all_replicas().map(|_| ReplicaState::default()).collect();
        self.request_processor = RequestProcessor::<P, U>::new_recovered(
            request_batcher,
            cert.last_executed_client_req.clone(),
            cert.total_amount_accepted_batches,
            cert.usig_attestations.clone(),
        );

        self.replica_storage = storage;
        self.checkpoint_replica_storage = self.replica_storage.clone();
        self.recovering_replicas = cert.recovering_replicas.clone();
        self.last_checkpoint_cert = Some(certificate);
        self.recovering = false;

        for (replica_id, peer_message) in self.recovery_unverified_usig_messages.clone() {
            debug!(
                "Retry processing peer message after recovery from replica={:?} {:?}",
                replica_id, peer_message
            );

            let message = match peer_message.validate(replica_id, &self.config, &mut self.usig) {
                Ok(message) => message,
                Err(output_inner_error) => {
                    output.error(output_inner_error.into());
                    continue;
                }
            };
            self.process_peer_message(replica_id, message, output);
        }

        info!("Accepted Recovery reply :D")
    }

    fn ready_for_client_requests(&self) -> bool {
        self.request_processor.recv_attestations.len() as u64 == self.config.n.get()
            && !self.recovering
    }

    /// Performs an attestation.
    fn attest(&mut self) -> Result<NotReflectedOutput<P, U>> {
        let attestation = self.usig.attest()?;
        let message = ValidatedPeerMessage::Hello(attestation);
        let mut output = NotReflectedOutput::new(&self.config, self.ready_for_client_requests());
        output.broadcast(message, &mut self.sent_usig_msgs);
        Ok(output)
    }

    /// Creates a recovery request.
    fn create_recovery_request(&mut self) -> Result<NotReflectedOutput<P, U>> {
        let attestation = self.usig.attest()?;

        let mut rng = thread_rng();
        let nonce: Nonce = rng.gen();
        self.recovering_nonce = Some(nonce);
        let message = ValidatedPeerMessage::RecoveryRequest(attestation, nonce);
        assert!(!self.ready_for_client_requests());
        let mut output = NotReflectedOutput::new(&self.config, self.ready_for_client_requests());
        info!(
            "Broadcasted recovery request {:?} replica={:?} {:?}",
            nonce, self.config.id, message
        );
        output.broadcast(message, &mut self.sent_usig_msgs);
        Ok(output)
    }
}
