mod checkpoint_generator;

use std::{
    cmp::Ordering,
    collections::{HashMap, VecDeque},
    fmt::Debug,
    time::{Duration, Instant},
};

use derivative::Derivative;
use serde::{Deserialize, Serialize};
use shared_ids::{ClientId, ReplicaId, RequestId};
use tracing::{debug, trace};
use usig::Usig;

use crate::{
    client_request::ClientRequest,
    output::{NotReflectedOutput, TimeoutRequest},
    peer_message::usig_message::{
        checkpoint::CheckpointContent, view_peer_message::prepare::Prepare,
    },
    timeout::Timeout,
    Config, Nonce, RecoveryRequestPayload, ReplicaStorage, RequestBatch, RequestBatcher,
    RequestPayload, ViewState, WrappedRequestPayload,
};

use self::checkpoint_generator::CheckpointGenerator;

/// Defines the state of a Client.
/// Contains the ID of the last accepted client request and the currently
/// processing client request with its ID.
#[derive(Debug, Serialize, Deserialize, Derivative)]
#[derivative(Default(bound = ""))]
struct ClientState<P, Att> {
    /// The last accepted client request.
    last_accepted_req: Option<RequestId>,
    /// The currently processing, not yet accepted client request with its ID.
    currently_processing_req: Option<(RequestId, ClientRequest<P, Att>)>,
}

impl<P, Att> ClientState<P, Att> {
    /// Update [ClientState] when receiving a new client request.
    ///
    /// # Arguments
    ///
    /// * `request_id` - The ID of the request received.
    /// * `client_req` - The client request received.
    ///
    /// # Return Value
    ///
    /// Returns true if the [RequestId] is greater than the currently processing
    /// request or if there is no currently processing request, otherwise false.
    ///
    ///
    fn update_upon_request_receival(
        &mut self,
        request_id: RequestId,
        client_req: ClientRequest<P, Att>,
    ) -> bool {
        if Some(request_id) <= self.last_accepted_req {
            trace!("Ignored request to update client state with an old client request with ID {:?} from client with ID {:?}: last accepted request of the same client had ID {:?}.", request_id, client_req.client, self.last_accepted_req);
            return false;
        }

        if let Some(processing) = &self.currently_processing_req {
            match request_id.cmp(&processing.0) {
                Ordering::Less => {
                    trace!("Ignored request to update client state with an old client request with ID {:?} from client with ID {:?}: currently processing request of the same client is newer, has ID {:?}.", request_id, client_req.client, processing.0);
                    false
                }

                Ordering::Equal => {
                    // It was seen before.
                    trace!("Ignored request to update client state with client request with ID {:?} from client with ID {:?} which was already previously received and is being processed.", request_id, client_req.client);
                    false
                }
                Ordering::Greater => {
                    trace!("Updated client state with client request with ID {:?} from client with ID {:?}: received request is newer than the currently processing one with ID {:?} of the same client.", request_id, client_req.client, processing.0);
                    self.currently_processing_req = Some((request_id, client_req));
                    true
                }
            }
        } else {
            trace!(
                "Updated client state with client request with ID {:?} from client with ID {:?}: no request was currently being processed of the same client.",
                request_id, client_req.client
            );
            self.currently_processing_req = Some((request_id, client_req));
            true
        }
    }

    /// Update [ClientState] when completing a client request.
    ///
    /// Ensures the ID of the request completed is higher than the ID of the
    /// last accepted request.
    /// Sets the currently processing request to [None] if its ID is lower
    /// or equal to the completed request.
    ///
    /// # Arguments
    ///
    /// * `request_id` - The ID of the request that was completed.
    ///
    /// # Return Value
    ///
    /// Returns false if the client request was already accepted, otherwise
    /// true.
    fn update_upon_request_completion(&mut self, request_id: RequestId) -> bool {
        if self.last_accepted_req >= Some(request_id) {
            trace!("Failed to update client state regarding the completion of client request (ID: {:?}): ID of last accepted request from the same client is greater than or equal to the receiving request's ID.", request_id);
            return false;
        }
        self.last_accepted_req = Some(request_id);
        if let Some(currently_processing) = &self.currently_processing_req {
            if currently_processing.0 <= request_id {
                self.currently_processing_req = None;
            }
        }
        true
    }
}

/// The purpose of the struct is to process and accept requests.
pub(crate) struct RequestProcessor<P: RequestPayload, U: Usig> {
    /// Collects the ClientState of each ClientId.
    clients_state: HashMap<ClientId, ClientState<P, U::Attestation>>,
    /// Collects the currently processing client requests.
    /// Additionally to the ID of the request and the client request itself,
    /// the time of arrival of the request is kept track of.
    currently_processing_reqs: VecDeque<(RequestId, ClientRequest<P, U::Attestation>, Instant)>,
    /// Used for batching requests.
    pub(crate) request_batcher: Box<dyn RequestBatcher<P, U::Attestation>>,
    /// Used for possibly generating a Checkpoint when sufficient requests have
    /// been accepted.
    pub(crate) checkpoint_generator: CheckpointGenerator<P, U>,

    /// The set containing the peers from which this replica received usig attestations
    pub(crate) recv_attestations: HashMap<ReplicaId, U::Attestation>,
    round: u64,
}

impl<P: RequestPayload, U: Usig> RequestProcessor<P, U>
where
    U::Attestation: Clone + for<'a> Deserialize<'a> + Serialize + PartialEq,
    U::Signature: Clone + Serialize + Debug,
{
    /// Create a new RequestProcessor with the given
    /// batch timeout and the max size for a batch.
    pub(crate) fn new(request_batcher: Box<dyn RequestBatcher<P, U::Attestation>>) -> Self {
        RequestProcessor {
            clients_state: HashMap::new(),
            currently_processing_reqs: VecDeque::with_capacity(500000),
            request_batcher,
            checkpoint_generator: CheckpointGenerator::new(),
            recv_attestations: HashMap::new(),
            round: 0,
        }
    }

    /// Create a new RequestProcessor with the given
    /// batch timeout and the max size for a batch
    /// and initial values
    pub(crate) fn new_recovered(
        request_batcher: Box<dyn RequestBatcher<P, U::Attestation>>,
        clients_last_req: HashMap<ClientId, RequestId>,
        round: u64,
        recv_attestations: HashMap<ReplicaId, U::Attestation>,
    ) -> Self {
        let mut clients_state = HashMap::new();
        for (client_id, last_accepted_req) in clients_last_req.iter() {
            clients_state.insert(
                *client_id,
                ClientState {
                    last_accepted_req: Some(*last_accepted_req),
                    currently_processing_req: None,
                },
            );
        }
        let mut new = Self::new(request_batcher);
        new.clients_state = clients_state;
        new.round = round;
        new.recv_attestations = recv_attestations;
        new
    }

    pub(crate) fn next_client_req_id(&self, client_id: &ClientId) -> RequestId {
        let mut highest_req_id = 0;

        if let Some(client_state) = self.clients_state.get(client_id) {
            if let Some(id) = client_state.last_accepted_req {
                highest_req_id = id.as_u64() + 1;
            }
        }
        for (_, req, _) in &self.currently_processing_reqs {
            if req.client == *client_id && req.id().as_u64() >= highest_req_id {
                highest_req_id = req.id().as_u64() + 1;
            }
        }
        RequestId::from_u64(highest_req_id)
    }

    /// Processes a client request.
    ///
    /// May return a client timeout, a [Prepare], and/or a batch timeout.
    ///
    /// # Arguments
    ///
    /// * `client_request` - The client request to be processed.
    /// * `view_state` - The inner state of the replica.
    /// * `client_timeout_duration` - The duration of the client timeout.
    /// * `config` - The configuration of the replica.
    ///
    /// # Return Value
    ///
    /// A client timeout is returned when there was no client timeout running
    /// upon receiving the given client request.
    ///
    /// A [Prepare] is returned when the maximum batch size of client requests
    /// has been reached.
    ///
    /// A batch timeout is returned when there was no batch timeout running upon
    /// receiving the given client request and the maximum batch size of client
    /// requests has not (yet) been reached.
    ///
    /// A tuple is returned in the aforementioned order with the respective
    /// values.
    #[allow(clippy::type_complexity)]
    pub(crate) fn process_client_req(
        &mut self,
        client_request: ClientRequest<P, U::Attestation>,
        view_state: &ViewState<P, U::Signature, U::Attestation>,
        client_timeout_duration: Duration,
        config: &Config,
    ) -> (
        Option<TimeoutRequest>,
        Option<RequestBatch<P, U::Attestation>>,
        Option<Timeout>,
    ) {
        trace!(
            "Processing client request (ID {:?}, client ID: {:?}) ...",
            client_request.id(),
            client_request.client
        );
        let request_id = client_request.id();

        trace!(
            "Updating state of client (ID: {:?}) ...",
            client_request.client
        );
        if !self
            .clients_state
            .entry(client_request.client)
            .or_default()
            .update_upon_request_receival(request_id, client_request.clone())
        {
            return (None, None, None);
        }

        self.currently_processing_reqs.push_back((
            request_id,
            client_request.clone(),
            Instant::now(),
        ));

        let start_client_timeout = Some(TimeoutRequest::new_start_client_req(
            client_request.client,
            client_timeout_duration,
        ));

        let mut prepare_content = None;
        let mut batch_timeout_request = None;

        match view_state {
            ViewState::InView(in_view) => {
                if config.me_primary(in_view.view) {
                    let (request_batch, timeout_request) =
                        self.request_batcher.batch(client_request.clone());
                    batch_timeout_request = timeout_request;
                    prepare_content = request_batch;
                };
            }
            // Client messages are ignored when the replica is in the state of
            // changing Views.
            ViewState::ChangeInProgress(in_progress) => {
                trace!("Ignored possible (if replica is primary) creation of Prepare as replica is in the process of changing views (from: {:?}, to: {:?}).", in_progress.prev_view, in_progress.next_view);
            }
        }
        trace!(
            "Processed client request (ID: {:?}, client ID: {:?}).",
            client_request.id(),
            client_request.client
        );
        (start_client_timeout, prepare_content, batch_timeout_request)
    }

    /// Returns the currently processing client request with its RequestId
    /// of each Client being tracked.
    pub(super) fn currently_processing_all(
        &self,
    ) -> impl Iterator<Item = (RequestId, &ClientRequest<P, U::Attestation>)> {
        self.clients_state
            .values()
            .filter_map(|req| req.currently_processing_req.as_ref())
            .map(|(id, req)| (*id, req))
    }

    /// Accepts the provided [Prepare].
    ///
    /// # Arguments
    ///
    /// * `config` - The configuration of the replica.
    /// * `prepare` - The accepted [Prepare].
    /// * `timeout_duration` - The current set timeout duration.
    /// * `output` - The output struct to be adjusted in case of, e.g., errors
    ///              or responses.
    ///
    /// # Return Value
    ///
    /// The CheckpointContent if a Checkpoint is generated (see
    /// [CheckpointGenerator]).
    #[allow(clippy::type_complexity, clippy::too_many_arguments)]
    pub(crate) fn accept_prepare(
        &mut self,
        config: &Config,
        prepare: &Prepare<P, U::Signature, U::Attestation>,
        recovering_replicas: &mut HashMap<ReplicaId, Nonce>,
        replica_storage: &mut ReplicaStorage<P>,
        timeout_duration: Duration,
        output: &mut NotReflectedOutput<P, U>,
        common_case: bool,
    ) -> (
        Option<CheckpointContent<U::Attestation>>,
        Vec<(ClientId, RecoveryRequestPayload<U::Attestation>)>,
        Option<RequestBatch<P, U::Attestation>>,
        Option<Timeout>,
    ) {
        debug!("Accepting batch of Prepares...");
        let mut accepted = false;
        let mut recovery_requests = Vec::new();

        let (new_batch, new_timout) = if common_case {
            self.request_batcher.update(&prepare.request_batch)
        } else {
            (None, None)
        };

        for request in prepare.request_batch.clone() {
            let (request_accepted, recovery_request) =
                self.accept_request(request, timeout_duration, replica_storage, output);

            if let Some((client_id, recovery_request)) = recovery_request {
                recovering_replicas.insert(
                    ReplicaId::from_u64(client_id.as_u64()),
                    recovery_request.nonce,
                );
                recovery_requests.push((client_id, recovery_request));
            }
            if request_accepted {
                accepted = true;
            }
        }
        if accepted {
            self.round += 1;
            output.log_round_change();
        }

        trace!("Accepted batch of Prepares in round ={:?}.", self.round);

        let mut last_executed_client_req = HashMap::new();
        for (client_id, client_state) in self.clients_state.iter() {
            if let Some(request_id) = client_state.last_accepted_req {
                last_executed_client_req.insert(*client_id, request_id);
            }
        }

        (
            self.checkpoint_generator.generate_checkpoint(
                prepare,
                replica_storage.hash(),
                last_executed_client_req,
                self.recv_attestations.clone(),
                recovering_replicas.clone(),
                self.round,
                config,
            ),
            recovery_requests,
            new_batch,
            new_timout,
        )
    }

    /// Returns the round, i.e. the amount of accepted [Prepare]s.
    pub(crate) fn round(&self) -> u64 {
        self.round
    }

    /// Accepts the provided request and responds to the respective client.
    /// Sets a new timeout if there are still currently processing requests.
    /// The elapsed time after the initial arrival of the client request is
    /// considered when computing the timeout duration.
    ///
    /// # Arguments
    ///
    /// * `request` - The accepted client request.
    /// * `curr_full_timeout_duration` - The current set timeout duration.
    /// * `output` - The output struct to be adjusted in case of, e.g., errors
    ///              or responses.
    #[allow(clippy::type_complexity)]
    fn accept_request(
        &mut self,
        request: ClientRequest<P, U::Attestation>,
        curr_full_timeout_duration: Duration,
        replica_storage: &mut ReplicaStorage<P>,
        output: &mut NotReflectedOutput<P, U>,
    ) -> (
        bool,
        Option<(ClientId, RecoveryRequestPayload<U::Attestation>)>,
    ) {
        trace!(
            "Accepting client request (ID: {:?}, client ID: {:?}) ...",
            request,
            request.client
        );
        // Update state of the client from which the request is.
        if !self
            .clients_state
            .entry(request.client)
            .or_default()
            .update_upon_request_completion(request.payload.id())
        {
            return (false, None);
        };

        // Send request to stop timeout that may be set for this now accepted client-request.
        let stop_client_request = TimeoutRequest::new_stop_client_req(request.client);
        output.timeout_request(stop_client_request);

        // Update data structure of currently processing requests.
        // Possibly create a new timeout request with the reset duration.
        let mut start_client_timeout = None;
        while let Some(oldest_req) = self.currently_processing_reqs.front() {
            // Check if the oldest request in the data structure was already accepted.
            // Remove it if so.
            let client_state = self.clients_state.get(&oldest_req.1.client);
            if client_state.is_none() {
                // It was already accepted.
                self.currently_processing_reqs.pop_front();
                continue;
            };
            let client_state = client_state.unwrap();
            match Some(oldest_req.0).cmp(&client_state.last_accepted_req) {
                Ordering::Greater => {
                    // The oldest request in the data structure has not yet been accepted.
                    // Send a request to start a timeout for it with the reset duration.
                    start_client_timeout = Some(TimeoutRequest::new_start_client_req(
                        oldest_req.1.client,
                        curr_full_timeout_duration,
                    ));
                    break;
                }
                _ => {
                    // It was already accepted.
                    self.currently_processing_reqs.pop_front();
                    continue;
                }
            }
        }
        trace!(
            "Accepted client request (ID: {:?}, client ID: {:?})",
            request.id(),
            request.client
        );
        let recovery_request = match request.payload.clone() {
            WrappedRequestPayload::Client(request_id, payload) => {
                replica_storage.execute_request(request.client, request_id, payload);
                None
            }
            WrappedRequestPayload::Internal(recovery_request) => {
                Some((request.client, recovery_request))
            }
        };

        output.response(request.client, request.payload);
        if let Some(start_client_request) = start_client_timeout {
            output.timeout_request(start_client_request);
        }
        (true, recovery_request)
    }
}
