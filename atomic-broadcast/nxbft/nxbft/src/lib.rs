pub mod consensus_dag;
pub mod message;
pub mod output;
mod util;

use std::{
    collections::{hash_map::Entry, HashMap, HashSet},
    fmt::Debug,
    hash::Hash,
    ops::Deref,
    time::{Duration, Instant},
};

use chrono::Local;
use consensus_dag::Dag;
use derivative::Derivative;
use hashbar::Hashbar;
use heaps::{pending_client_requests::PendingClientRequests, CursoredHeap, HeapBuffer};
use id_set::{IdSet, ReplicaSet};
pub use message::PeerMessage;
use message::{Message, NonEquivocationPayload, RecoveryCommit, RecoveryProposal, SignedMessage};
pub use nxbft_base::enclave;
pub use nxbft_base::enclave::Enclave;
use nxbft_base::{
    enclave::{HandshakeResult, InitHandshakeResult},
    vertex::Vertex,
};
pub use nxbft_base::{DagAddress, Round};
use output::{
    Broadcast, Consensus, Error, Output, PeerError, PeerErrorType, StateMessage, UnreflectedOutput,
    Verbose,
};
use serde::Serialize;
use shared_ids::{id_type, map::ReplicaMap, ClientId, ReplicaId, RequestId};
use tracing::{debug, debug_span, error, info, warn};
use transaction_trait::Transaction;
use util::round;

use std::fs::OpenOptions;
use std::io::Write;

id_type!(pub Wave);

impl From<Round> for Wave {
    fn from(round: Round) -> Self {
        assert!(round.as_u64() >= 4);
        Self::from_u64(round.as_u64() / 4)
    }
}

type NonEquivocationMessageBuffers<Tx, Sig> = ReplicaMap<HeapBuffer<SignedMessage<Tx, Sig>>>;
type RecoveryCommits<Tx, Enc> = HashMap<
    <Enc as Enclave>::Attestation,
    ReplicaMap<
        Option<RecoveryCommit<Tx, <Enc as Enclave>::Attestation, <Enc as Enclave>::Signature>>,
    >,
>;

#[derive(Debug, Derivative)]
#[derivative(Clone(bound = ""))]
pub struct Backup<Tx: Clone + Hashbar, Enc: Enclave> {
    n: u64,
    quorum: u64,
    me: ReplicaId,
    round: Round,
    decided_wave: Wave,
    dag: Dag<Tx, Enc::Signature>,
    client_last_request: HashMap<ClientId, RequestId>,
    expected_counters: ReplicaMap<u64>,
    highest_rounds: ReplicaMap<Round>,
    vertex_buffer: CursoredHeap<Vertex<Tx, Enc::Signature>>,
    pending_wave_coins: HashMap<Wave, ReplicaId>,
    vertex_timeout: Duration,
    min_vertex_size: usize,
    max_vertex_size: usize,
    enclave: Enc::Export,
    seen_recovery_requests: HashSet<Enc::Attestation>,
    successful_recoveries: HashSet<Enc::Attestation>,
    recovery_proposal_senders: HashMap<Enc::Attestation, ReplicaSet>,
    max_rounds: ReplicaMap<Round>,
}

pub struct NxBft<Tx, Enc>
where
    Tx: Clone + Hashbar,
    Enc: Enclave,
{
    n: u64,
    quorum: u64,
    me: ReplicaId,
    round: Round,
    decided_wave: Wave,
    dag: Dag<Tx, Enc::Signature>,
    hello_reply_count: u64,
    ready: bool,
    call_create_vertex: bool,
    in_recovery: bool,
    hello_set: ReplicaSet,
    ready_set: ReplicaSet,
    hello_map: HashMap<ReplicaId, (Enc::Attestation, ReplicaSet)>,
    pending_client_requests: PendingClientRequests<Tx>,
    client_last_request: HashMap<ClientId, RequestId>,
    expected_counters: ReplicaMap<u64>,
    highest_rounds: ReplicaMap<Round>,
    non_equivocation_message_buffers: NonEquivocationMessageBuffers<Tx, Enc::Signature>,
    vertex_buffer: CursoredHeap<Vertex<Tx, Enc::Signature>>,
    enclave: Enc,
    pending_wave_coins: HashMap<Wave, ReplicaId>,
    cached_hello_replies: HashMap<ReplicaId, Enc::Handshake>,
    pending_vertices: HashSet<DagAddress>,
    vertex_requests: HashMap<DagAddress, ReplicaSet>,
    vertex_timeout: Duration,
    allow_timeout: bool,
    min_vertex_size: usize,
    max_vertex_size: usize,
    restart_round: Round,
    recovery_proposal_senders: HashMap<Enc::Attestation, ReplicaSet>,
    longest_recovery_proposal: HashMap<Enc::Attestation, Vec<SignedMessage<Tx, Enc::Signature>>>,
    currently_recovering: ReplicaSet,
    recovery_commits: RecoveryCommits<Tx, Enc>,
    seen_recovery_requests: HashSet<Enc::Attestation>,
    successful_recoveries: HashSet<Enc::Attestation>,
    output: UnreflectedOutput<Tx, Enc>,
    max_rounds: ReplicaMap<Round>,
    recovery_start: Option<Instant>,
    last_vertex_created_at: Instant,
}

impl<Tx, Enc> NxBft<Tx, Enc>
where
    Tx: Clone + Transaction + Debug + Serialize + Hashbar + Eq + Hash,
    Enc: Enclave,
{
    pub fn new(
        me: ReplicaId,
        n: u64,
        vertex_timeout: Duration,
        min_vertex_size: u64,
        max_vertex_size: u64,
    ) -> Self {
        assert!(n >= 3, "Minimum number of peers is three");
        assert!(me.as_u64() < n, "ReplicaId is out of range");

        assert_ne!(max_vertex_size, 0);
        assert!(min_vertex_size <= max_vertex_size);

        let enclave = Enc::new(n);

        NxBft {
            n,
            me,
            round: 1.into(),
            decided_wave: 0.into(),
            dag: Dag::new(n),
            hello_reply_count: 0,
            expected_counters: ReplicaMap::new(n),
            highest_rounds: ReplicaMap::new(n),
            ready: false,
            call_create_vertex: false,
            in_recovery: false,
            enclave,
            quorum: (n / 2) + 1,
            hello_set: ReplicaSet::with_capacity(n),
            hello_map: HashMap::with_capacity(n as usize),
            non_equivocation_message_buffers: ReplicaMap::new(n),
            vertex_buffer: CursoredHeap::default(),
            pending_vertices: HashSet::new(),
            vertex_requests: HashMap::new(),
            pending_client_requests: PendingClientRequests::new(max_vertex_size as usize),
            client_last_request: HashMap::new(),
            pending_wave_coins: HashMap::new(),
            ready_set: ReplicaSet::with_capacity(n),
            cached_hello_replies: HashMap::with_capacity(n as usize),
            vertex_timeout,
            min_vertex_size: min_vertex_size as usize,
            max_vertex_size: max_vertex_size as usize,
            allow_timeout: false,
            restart_round: 0.into(),
            recovery_proposal_senders: HashMap::new(),
            longest_recovery_proposal: HashMap::new(),
            recovery_commits: HashMap::new(),
            seen_recovery_requests: HashSet::new(),
            successful_recoveries: HashSet::new(),
            output: UnreflectedOutput::new(me),
            currently_recovering: ReplicaSet::with_capacity(n),
            max_rounds: ReplicaMap::new(n),
            recovery_start: None,
            last_vertex_created_at: Instant::now(),
        }
    }

    pub fn init_handshake(&mut self) -> Output<Tx, Enc> {
        assert!(!self.ready, "Can only intialize once!");
        if let Err(e) = self.enclave.init() {
            self.output.state_message(Error::EnclaveError(e));
            return self.reflect();
        }
        let attestation = match self.enclave.get_attestation_certificate() {
            Ok(cert) => cert,
            Err(e) => {
                self.output.state_message(Error::EnclaveError(e));
                return self.reflect();
            }
        };
        self.output.broadcast(PeerMessage {
            from: self.me,
            message: Message::Hello(attestation.clone()),
        });
        self.reflect()
    }

    pub fn init_recovery(
        export: Backup<Tx, Enc>,
    ) -> Result<(Self, Output<Tx, Enc>), Enc::EnclaveError> {
        let Backup {
            n,
            quorum,
            me,
            round,
            decided_wave,
            dag,
            client_last_request,
            expected_counters,
            highest_rounds,
            vertex_buffer,
            pending_wave_coins,
            vertex_timeout,
            min_vertex_size,
            max_vertex_size,
            enclave,
            seen_recovery_requests,
            successful_recoveries,
            recovery_proposal_senders,
            max_rounds,
        } = export;
        let mut enc = Enc::new(export.n);
        enc.recover(enclave)?;

        let attestation = enc.get_attestation_certificate()?;

        let mut slf = Self {
            n,
            quorum,
            me,
            round,
            decided_wave,
            dag,
            hello_reply_count: n,
            ready: true,
            call_create_vertex: false,
            in_recovery: true,
            hello_set: IdSet::with_capacity(n),
            ready_set: IdSet::with_capacity(n),
            hello_map: HashMap::new(),
            pending_client_requests: PendingClientRequests::new(max_vertex_size),
            client_last_request,
            expected_counters,
            highest_rounds,
            non_equivocation_message_buffers: ReplicaMap::new(n),
            vertex_buffer,
            enclave: enc,
            pending_wave_coins,
            cached_hello_replies: HashMap::new(),
            pending_vertices: HashSet::new(),
            vertex_requests: HashMap::new(),
            vertex_timeout,
            allow_timeout: false,
            min_vertex_size,
            max_vertex_size,
            restart_round: 0.into(),
            recovery_proposal_senders,
            longest_recovery_proposal: HashMap::new(),
            recovery_commits: HashMap::new(),
            seen_recovery_requests,
            successful_recoveries,
            output: UnreflectedOutput::new(me),
            currently_recovering: ReplicaSet::with_capacity(n),
            max_rounds,
            recovery_start: Some(Instant::now()),
            last_vertex_created_at: Instant::now(),
        };

        info!("Starting recovery");

        slf.output.broadcast(PeerMessage {
            from: me,
            message: Message::ReocoveryRequest(attestation),
        });
        let out = slf.reflect();
        Ok((slf, out))
    }

    pub fn backup_enclave(&mut self) -> Result<Backup<Tx, Enc>, Enc::EnclaveError> {
        let enclave = self.enclave.export().unwrap();

        Ok(Backup {
            n: self.n,
            quorum: self.quorum,
            me: self.me,
            round: self.round,
            decided_wave: self.decided_wave,
            dag: self.dag.clone(),
            client_last_request: self.client_last_request.clone(),
            expected_counters: self.expected_counters.clone(),
            highest_rounds: self.highest_rounds.clone(),
            vertex_buffer: self.vertex_buffer.clone(),
            pending_wave_coins: self.pending_wave_coins.clone(),
            vertex_timeout: self.vertex_timeout,
            min_vertex_size: self.min_vertex_size,
            max_vertex_size: self.max_vertex_size,
            enclave,
            seen_recovery_requests: self.seen_recovery_requests.clone(),
            successful_recoveries: self.successful_recoveries.clone(),
            recovery_proposal_senders: self.recovery_proposal_senders.clone(),
            max_rounds: self.max_rounds.clone(),
        })
    }

    pub fn process_peer_message(
        &mut self,
        peer_message: PeerMessage<Tx, Enc::Attestation, Enc::Handshake, Enc::Signature>,
    ) -> Output<Tx, Enc> {
        let _span = debug_span!("process_peer_message", replica = self.me.as_u64()).entered();

        if peer_message.from.as_u64() >= self.n {
            self.output.state_message(PeerError::from_type(
                peer_message.from,
                PeerErrorType::InvalidFrom,
            ));
            return self.reflect();
        }

        match peer_message.message {
            Message::Hello(attestation) => self.process_hello(peer_message.from, attestation),
            Message::HelloEcho(creator, attestation) => {
                self.process_hello_echo(peer_message.from, creator, attestation)
            }
            Message::HelloReply(handshake) => {
                self.process_hello_reply(peer_message.from, handshake)
            }
            Message::Ready => self.process_ready(peer_message.from),
            Message::Signed(msg) => self.process_signed_message(peer_message.from, msg),
            Message::VertexRequest(address) => {
                self.process_vertex_request(peer_message.from, address)
            }
            Message::ReocoveryRequest(attestation) => {
                self.process_recovery_request(peer_message.from, attestation)
            }
            Message::RecoveryCommit(commit) => {
                self.process_recovery_commit(peer_message.from, commit)
            }
            Message::RecovertyProposal(recovery_proposal) => {
                self.process_recovery_proposal(peer_message.from, recovery_proposal)
            }
        };
        self.reflect()
    }

    fn process_recovery_request(&mut self, from: ReplicaId, attestation: Enc::Attestation) {
        if self.successful_recoveries.contains(&attestation) {
            debug!("ignore recovery request from {from} since the corresponding recovery requests was already successful");
            return;
        }

        if self.seen_recovery_requests.contains(&attestation) {
            debug!("ignore recovery request from {from} since we already received it");
            self.output.state_message(Verbose::PeerRecoveryIgnore {
                from,
                msg: "RecoveryRequest".to_string(),
            });
            return;
        }
        assert!(self.seen_recovery_requests.insert(attestation.clone()));

        match self.enclave.verify_attestation(&attestation) {
            Ok(b) => {
                if !b {
                    debug!("ignore recovery request from {from} since the attestation is invalid");
                    self.output.state_message(PeerError {
                        by: from,
                        error_type: PeerErrorType::InvalidAttestation,
                    });
                    return;
                }
            }
            Err(e) => {
                error!("ignore recovery request from {from} since the attestation could not be verified");
                self.output.state_message(Error::EnclaveError(e));
                return;
            }
        }

        if self.in_recovery {
            self.output.broadcast(PeerMessage {
                from: self.me,
                message: Message::ReocoveryRequest(
                    self.enclave.get_attestation_certificate().unwrap(),
                ),
            });
        }

        debug!("process recovery request from {from}");
        assert!(self
            .recovery_commits
            .insert(attestation.clone(), ReplicaMap::new(self.n))
            .is_none());
        assert!(self
            .longest_recovery_proposal
            .insert(attestation.clone(), vec![])
            .is_none());
        self.currently_recovering.insert(from);
        assert!(self
            .recovery_proposal_senders
            .insert(attestation.clone(), ReplicaSet::with_capacity(self.n))
            .is_none());

        let mut cursor = self.vertex_buffer.cursor();
        while let Some(v) = cursor.advance() {
            if v.creator() == from {
                cursor.remove();
            }
        }
        cursor.finalize();
        self.non_equivocation_message_buffers[from].clear();

        let mut round_prime = self.round;
        let mut top = 0;
        let mut bot = 0;
        while round_prime > 0.into() {
            let Some(v) = self.dag.get(DagAddress::new(round_prime, from)) else {
                if top == 0 {
                    round_prime -= 1;
                    continue;
                } else {
                    break;
                }
            };
            if top == 0 {
                match self.enclave.verify(
                    v.creator(),
                    v.counter(),
                    v.deref().deref().clone(),
                    v.signature(),
                ) {
                    Ok(b) => {
                        if !b {
                            break;
                        }
                    }
                    Err(e) => {
                        self.output.state_message(Error::EnclaveError(e));
                        return;
                    }
                }
                top = round_prime.into();
            }
            if top != 0 {
                bot = round_prime.into();
                if v.counter() == 0
                    || (round_prime < top.into()
                        && v.counter()
                            > self
                                .dag
                                .get(DagAddress::new(round_prime + 1, from))
                                .unwrap()
                                .counter())
                {
                    break;
                }
            }
            round_prime -= 1;
        }
        let messages = if top == 0 {
            debug!("empty recovery proposal row for {from}");
            Vec::new()
        } else {
            debug!("construct recovery proposal row {bot}..={top} for {from}");
            let mut messages = Vec::new();
            for round_prime in bot..=top {
                let v = self
                    .dag
                    .get(DagAddress::new(round_prime.into(), from))
                    .unwrap();
                messages.push(SignedMessage {
                    payload: NonEquivocationPayload(v.clone()),
                });
            }
            messages
        };

        let proposal = RecoveryProposal {
            for_id: from,
            for_attestation: attestation,
            messages,
        };

        debug!("broadcast recovery proposal");

        self.output.broadcast(PeerMessage {
            from: self.me,
            message: Message::RecovertyProposal(proposal),
        });
    }

    fn process_recovery_proposal(
        &mut self,
        from: ReplicaId,
        recovery_proposal: RecoveryProposal<Tx, Enc::Attestation, Enc::Signature>,
    ) {
        self.process_recovery_request(
            recovery_proposal.for_id,
            recovery_proposal.for_attestation.clone(),
        );

        if self
            .successful_recoveries
            .contains(&recovery_proposal.for_attestation)
        {
            debug!("ignore recovery proposal from {from} since the corresponding recovery requests was already successful");
            return;
        }

        match self
            .enclave
            .verify_attestation(&recovery_proposal.for_attestation)
        {
            Ok(b) => {
                if !b {
                    debug!("ignore recovery proposal for {from} since the attestation is invalid");
                    return;
                }
            }
            Err(e) => {
                error!("attestation of recovery proposal row for {from} could not be verified");
                self.output.state_message(Error::EnclaveError(e));
                return;
            }
        }

        let senders = self
            .recovery_proposal_senders
            .get_mut(&recovery_proposal.for_attestation)
            .unwrap();

        if senders.contains(from) {
            debug!("ignore recovery proposal from {from} since we already received a recovery proposal from them");
            return;
        }
        assert!(senders.insert(from));

        debug!(
            "verify recovery proposal row for {} by {}",
            recovery_proposal.for_id, from
        );

        let mut vs = Vec::with_capacity(recovery_proposal.messages.len());
        for m in recovery_proposal.messages.iter() {
            match self
                .enclave
                .verify(m.creator(), m.counter(), m.signable(), m.signature())
            {
                Ok(b) => {
                    if !b {
                        debug!(
                                "ignore recovery proposal row for {from} since a vertex signature is invalid"
                            );
                        return;
                    }
                }
                Err(e) => {
                    self.output.state_message(Error::EnclaveError(e));
                    return;
                }
            }
            let NonEquivocationPayload(v) = &m.payload;
            vs.push(v);
        }

        for k in 0..vs.len() {
            if (k > 0 && vs[k].round() != vs[k - 1].round() + 1)
                || (vs[k].round() == 1.into() && vs[k].num_edges() > 0)
                || (vs[k].round() > 1.into() && vs[k].num_edges() < self.quorum)
            {
                debug!("ignore recovery proposal row for {from} since the vertices are invalid");
                return;
            }
        }
        if let Some(msg) = self
            .longest_recovery_proposal
            .get_mut(&recovery_proposal.for_attestation)
        {
            if vs.len() > msg.len() {
                debug!(
                    "update longest recovery proposal for {from} with {} vertices",
                    vs.len()
                );
                *msg = recovery_proposal.messages;
            } else {
                debug!("ignore recovery proposal for {from} since it is not longer than the current one");
            }
        } else {
            warn!("ignore recovery proposal row for {from} since we did not receive a recovery request from them");
        }

        let senders_length = senders.len();
        if senders_length != self.n {
            assert!(senders_length < self.n);
            debug!("received {senders_length} recovery proposals");
            return;
        }

        let messages = self
            .longest_recovery_proposal
            .remove(&recovery_proposal.for_attestation)
            .unwrap();

        debug!("broadcast recovery commit");
        self.output.broadcast(PeerMessage {
            from: self.me,
            message: Message::RecoveryCommit(RecoveryCommit {
                for_id: recovery_proposal.for_id,
                for_attestation: recovery_proposal.for_attestation,
                messages,
            }),
        });
    }

    fn process_recovery_commit(
        &mut self,
        from: ReplicaId,
        recovery_commit: RecoveryCommit<Tx, Enc::Attestation, Enc::Signature>,
    ) {
        self.process_recovery_request(
            recovery_commit.for_id,
            recovery_commit.for_attestation.clone(),
        );

        if self
            .successful_recoveries
            .contains(&recovery_commit.for_attestation)
        {
            debug!("ignore recovery commi from {from} since the corresponding recovery requests was already successful");
            return;
        }

        match self
            .enclave
            .verify_attestation(&recovery_commit.for_attestation)
        {
            Ok(b) => {
                if !b {
                    debug!("ignore recovery commit for {from} since the attestation is invalid");
                    return;
                }
            }
            Err(e) => {
                error!("attestation of recovery commit for {from} could not be verified");
                self.output.state_message(Error::EnclaveError(e));
                return;
            }
        }

        let recovery_commits_for_att = self
            .recovery_commits
            .get_mut(&recovery_commit.for_attestation)
            .unwrap();

        if recovery_commits_for_att[from].is_some() {
            debug!("ignore recovery commit from {from} since we already received a recovery proposal from them");
            return;
        }
        assert!(recovery_commits_for_att[from]
            .replace(recovery_commit.clone())
            .is_none());

        if (recovery_commits_for_att.iter_some().count() as u64) < self.quorum {
            debug!("not enough recovery commits received yet");
            return;
        }

        debug!("enough recovery commits received");

        let mut proposal_count = HashMap::new();
        for (_, commit) in recovery_commits_for_att.drain_lazy_some() {
            let count: &mut u64 = proposal_count.entry(commit).or_default();
            *count += 1;
        }

        let mut proposal_count = proposal_count.drain();

        let commit = loop {
            let Some((commit, count)) = proposal_count.next() else {
                debug!("no recovery proposal has quorum");
                return;
            };
            if count >= self.quorum {
                debug!("recovery proposal has quorum");
                break commit;
            }
        };

        assert_eq!(commit.for_id, recovery_commit.for_id);
        assert_eq!(commit.for_attestation, recovery_commit.for_attestation);

        let mut cursor = self.vertex_buffer.cursor();
        while let Some(v) = cursor.advance() {
            if v.creator() == commit.for_id {
                cursor.remove();
            }
        }
        cursor.finalize();
        self.non_equivocation_message_buffers[commit.for_id].clear();

        let mut r = Round::default();
        debug!(
            "recovering replica {} with {} vertices",
            commit.for_id,
            commit.messages.len()
        );
        for m in &commit.messages {
            let NonEquivocationPayload(v) = &m.payload;
            assert_eq!(v.address().creator(), commit.for_id);
            if !self.dag.knows(v.address()) {
                self.vertex_buffer.insert(v.clone());
                self.output.state_message(Verbose::BufferedVertex {
                    vertex: v.address(),
                });
            }
            if self.max_rounds[v.creator()] < v.round() {
                self.max_rounds[v.creator()] = v.round();
            };
            r = v.round() + 1;
        }
        self.expected_counters[commit.for_id] = 0;
        if let Err(e) = self.enclave.replace_peer(
            commit.for_id,
            &commit.for_attestation,
            self.max_rounds[commit.for_id],
        ) {
            self.output.state_message(Error::EnclaveError(e));
            return;
        }
        if commit.for_id == self.me {
            self.restart_round = r;
            assert!(
                self.round <= self.restart_round,
                "{} <  {}",
                self.round,
                self.restart_round
            );

            self.in_recovery = false;
            self.call_create_vertex = true;
            self.output.set_timeout(self.vertex_timeout);
            debug!(
                "recovered self to round {}, restarting at {}",
                self.round, self.restart_round
            );
            let elapsed = self.recovery_start.unwrap().elapsed();
            info!("Recovered in {}s", elapsed.as_secs_f64());
            fn append_or_create_file(file_path: &str, content: &str) -> std::io::Result<()> {
                if cfg!(test) {
                    return Ok(());
                }
                let mut file = OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(file_path)?;
                writeln!(file, "{}", content)?;
                Ok(())
            }
            let datetime = Local::now();

            if let Err(e) = append_or_create_file(
                "/abcperf/recovery_times.csv",
                &format!(
                    "{},{},{},{}",
                    datetime.to_rfc3339(),
                    self.n,
                    self.me.as_u64(),
                    elapsed.as_secs_f64()
                ),
            ) {
                error!("Failed to write recovery time to file: {:?}", e);
            }
        }
        self.output.set_backup_enclave();

        self.seen_recovery_requests.remove(&commit.for_attestation);
        self.longest_recovery_proposal
            .remove(&commit.for_attestation);
        self.successful_recoveries.insert(commit.for_attestation);
        self.currently_recovering.remove(commit.for_id);

        debug!("checking vertex buffer for insertions");

        self.traverse_vertex_buffer();

        debug!("recovery successful");
    }

    pub fn receive_client_request(&mut self, transaction: Tx) -> Output<Tx, Enc> {
        let _span = debug_span!("receive_client_request", replica = self.me.as_u64()).entered();

        self.pending_client_requests.insert(transaction);

        debug!("received client request");

        if self.ready && !self.in_recovery && self.call_create_vertex {
            _ = self.create_vertex(false);
        }
        self.reflect()
    }

    /// Forces a new vertex to be sent. This vertex may be empty.
    /// The next timeout will only be started whenever the replica was able to perform a round transition.
    /// Timeouts are requested by [Output].
    pub fn process_vertex_timeout(&mut self) -> Output<Tx, Enc> {
        let _span = debug_span!("process_vertex_timeout", replica = self.me.as_u64()).entered();
        if self.ready && !self.in_recovery && self.allow_timeout {
            _ = self.create_vertex(true);
        }
        self.reflect()
    }

    fn process_hello(&mut self, from: ReplicaId, attestation: Enc::Attestation) {
        if self.hello_set.contains(from) {
            self.output
                .state_message(PeerError::from_type(from, PeerErrorType::DuplicateHello));
            return;
        }

        self.output.broadcast(PeerMessage {
            from: self.me,
            message: Message::HelloEcho(from, attestation.clone()),
        });

        self.hello_set.insert(from);
        self.hello_map
            .entry(from)
            .or_insert_with(|| (attestation.clone(), ReplicaSet::with_capacity(self.n)));
        self.output.state_message(Verbose::ReceivedHello { from })
    }

    fn process_hello_echo(
        &mut self,
        from: ReplicaId,
        creator: ReplicaId,
        attestation: Enc::Attestation,
    ) {
        if creator.as_u64() >= self.n {
            self.output.state_message(PeerError::from_type(
                from,
                PeerErrorType::InvalidRelayCreator { given: creator },
            ));
            return;
        }

        let (stored_attestation, echos) = self
            .hello_map
            .entry(creator)
            .or_insert_with(|| (attestation.clone(), ReplicaSet::with_capacity(self.n)));

        if *stored_attestation != attestation {
            self.output.state_message(PeerError::from_type(
                from,
                PeerErrorType::AttestationEquivocation { or: creator },
            ));
            return;
        }

        if echos.contains(from) {
            self.output.state_message(PeerError::from_type(
                from,
                PeerErrorType::DuplicateHelloEcho,
            ));
            return;
        }

        echos.insert(from);
        if echos.len() < self.n {
            return;
        }

        let handshake = match self.enclave.init_peer_handshake(creator, attestation) {
            InitHandshakeResult::Ok(handshake) => handshake,
            InitHandshakeResult::DuplicateHello => {
                self.output
                    .state_message(PeerError::from_type(from, PeerErrorType::DuplicateHello));
                return;
            }
            InitHandshakeResult::InvalidAttestation => {
                self.output.state_message(PeerError::from_type(
                    from,
                    PeerErrorType::InvalidAttestation,
                ));
                return;
            }
            InitHandshakeResult::EnclaveError(e) => {
                self.output.state_message(Error::EnclaveError(e));
                return;
            }
        };

        self.output
            .state_message(Verbose::ReceivedHelloEcho { from, creator });

        self.output.unicast(
            creator,
            PeerMessage {
                from: self.me,
                message: Message::HelloReply(handshake),
            },
        );
        if let Some(handshake) = self.cached_hello_replies.remove(&creator) {
            self.output
                .state_message(Verbose::CachedHelloReply { from: creator });
            self.process_hello_reply(creator, handshake);
        }
    }

    fn process_hello_reply(&mut self, from: ReplicaId, handshake: Enc::Handshake) {
        match self.enclave.complete_peer_handshake(from, &handshake) {
            HandshakeResult::DuplicateReply => self.output.state_message(PeerError::from_type(
                from,
                PeerErrorType::DuplicateHelloReply,
            )),
            HandshakeResult::InvalidHandshakeEncryption => self.output.state_message(
                PeerError::from_type(from, PeerErrorType::InvalidHandshakeEncryption),
            ),
            HandshakeResult::MissingAttestation => {
                self.cached_hello_replies.insert(from, handshake);
                self.output
                    .state_message(Verbose::HelloReplyBeforeHello { from })
            }
            HandshakeResult::Ok => {
                self.output
                    .state_message(Verbose::ReceivedHelloReply { from });
                self.hello_reply_count += 1;
                if self.hello_reply_count == self.n {
                    self.output.broadcast(PeerMessage {
                        from: self.me,
                        message: Message::Ready,
                    });
                }
            }
            HandshakeResult::EnclaveError(e) => self.output.state_message(Error::EnclaveError(e)),
        }
    }

    fn process_ready(&mut self, from: ReplicaId) {
        if self.ready_set.contains(from) {
            self.output
                .state_message(PeerError::from_type(from, PeerErrorType::DuplicateReady));
            return;
        }
        self.ready_set.insert(from);

        if self.ready_set.len() < self.n {
            return;
        }
        self.ready = true;
        self.call_create_vertex = true;
        self.allow_timeout = true;
        self.output.set_timeout(self.vertex_timeout);
        self.output.set_backup_enclave();
        self.output.state_message(StateMessage::Ready)
    }

    fn process_signed_message(&mut self, from: ReplicaId, msg: SignedMessage<Tx, Enc::Signature>) {
        if !self.ready {
            self.output.state_message(Error::NotReady);
            return;
        }
        if self.currently_recovering.contains(msg.creator()) {
            self.output.state_message(Verbose::PeerRecoveryIgnore {
                from: msg.creator(),
                msg: msg.to_string(),
            });
            return;
        }

        let creator = msg.creator();

        if creator.as_u64() >= self.n {
            debug!("ignore signed message from {from} since the creator is invalid");
            self.output.state_message(PeerError::from_type(
                from,
                PeerErrorType::InvalidRelayCreator { given: creator },
            ));
            return;
        }

        if msg.counter() < self.expected_counters[creator] {
            debug!("ignore signed message from {from} since it has ({}) a lower counter than expected ({})", msg.counter(), self.expected_counters[creator]);
            return;
        }

        if self.non_equivocation_message_buffers[creator].knows(&msg) {
            debug!("ignore signed message from {from} since we already received it");
            return;
        }

        match self
            .enclave
            .verify(creator, msg.counter(), msg.signable(), msg.signature())
        {
            Ok(b) => {
                if !b {
                    debug!("ignore signed message from {from} since the signature is invalid");
                    return;
                }
            }
            Err(e) => {
                debug!(
                    "ignore signed message from {from} since the signature could not be verified"
                );
                self.output.state_message(Error::EnclaveError(e));
                return;
            }
        }

        self.output
            .state_message(Verbose::NonEquivocationBroadcastReceive {
                payload: msg.payload.to_string(),
                creator: msg.creator(),
            });

        let NonEquivocationPayload(vertex) = &msg.payload;
        if self.highest_rounds[vertex.creator()] < vertex.round() {
            self.highest_rounds[vertex.creator()] = vertex.round();
        }

        self.non_equivocation_message_buffers[creator].insert(msg);

        if self.in_recovery {
            debug!("delay processing of signed message from {from} since we are recovering");
            return;
        }

        while let Some(m) = self.non_equivocation_message_buffers[creator].peek() {
            if m.counter() != self.expected_counters[creator] {
                let NonEquivocationPayload(v) = &m.payload;
                let address = DagAddress::new(v.round() - 1, m.creator());
                if self.pending_vertices.contains(&address) {
                    break;
                }
                debug!("requesting vertex with address {address:?}");
                self.output.broadcast(PeerMessage {
                    from: self.me,
                    message: Message::VertexRequest(address),
                });
                self.output.state_message(Broadcast::VertexRequest {
                    creator: address.creator(),
                    round: address.round(),
                });
                self.pending_vertices.insert(address);

                break;
            }

            let msg = self.non_equivocation_message_buffers[creator]
                .pop()
                .unwrap();

            self.output
                .state_message(Verbose::NonEquivocationBroadcastDeliver {
                    creator: msg.creator(),
                    payload: msg.payload.to_string(),
                });

            self.process_vertex(msg.payload.0);

            self.expected_counters[creator] += 1;
            debug!(
                "expected counter for {creator} is now {}",
                self.expected_counters[creator]
            );
        }
    }

    fn process_vertex(&mut self, vertex: Vertex<Tx, Enc::Signature>) {
        if self.vertex_buffer.knows(&vertex) {
            return;
        }

        if self.dag.knows(vertex.address()) {
            return;
        }

        if vertex.round().as_u64() == 1 && vertex.num_edges() > 0 {
            self.output.state_message(PeerError::from_type(
                vertex.creator(),
                PeerErrorType::InvalidVertexEdgesRound1,
            ));
            return;
        }
        if vertex.round().as_u64() > 1 && !vertex.edges_valid(self.quorum) {
            self.output.state_message(PeerError::from_type(
                vertex.creator(),
                PeerErrorType::InvalidVertexEdges,
            ));
            return;
        }

        debug!("received vertex with address {:?}", vertex.address());

        if self.max_rounds[vertex.creator()] < vertex.round() {
            self.max_rounds[vertex.creator()] = vertex.round();
        };

        self.pending_vertices.remove(&vertex.address());

        self.output.state_message(Verbose::BufferedVertex {
            vertex: vertex.address(),
        });

        self.vertex_buffer.insert(vertex);

        self.traverse_vertex_buffer()
    }

    fn traverse_vertex_buffer(&mut self) {
        let mut inserts = Vec::new();
        let mut cursor = self.vertex_buffer.cursor();
        let mut missings = HashSet::new();
        while let Some(v) = cursor.advance() {
            if v.round() > self.round {
                debug!(
                    "vertex with address {:?} is ahead of time {}, {}",
                    v.address(),
                    self.round,
                    self.restart_round
                );
                break;
            }
            let mut add = true;
            for creator in v.edges().iter() {
                let edge = DagAddress::new(v.round() - 1, creator);
                if self.dag.knows(edge) {
                    continue;
                }
                missings.insert(edge);
                add = false;
                if self.pending_vertices.contains(&edge) {
                    continue;
                }
                debug!("requesting vertex with address {edge:?}");
                self.output.broadcast(PeerMessage {
                    from: self.me,
                    message: Message::VertexRequest(edge),
                });
                self.output.state_message(Broadcast::VertexRequest {
                    creator: edge.creator(),
                    round: edge.round(),
                });
                self.pending_vertices.insert(edge);
            }

            if !add {
                continue;
            }

            self.output.state_message(Verbose::VertexAdded {
                vertex: v.address(),
            });
            inserts.push(v.address());
            debug!(
                "insert vertex with address {:?} and size {} into dag with parents {:?}",
                v.address(),
                v.transactions().len(),
                v.edges()
            );
            self.dag.insert(cursor.remove());
        }
        cursor.finalize();
        for address in inserts.into_iter() {
            for to in self
                .vertex_requests
                .remove(&address)
                .get_or_insert(ReplicaSet::with_capacity(self.n))
                .iter()
            {
                self.output.unicast(
                    to,
                    PeerMessage {
                        from: self.me,
                        message: Message::Signed(SignedMessage {
                            payload: NonEquivocationPayload(self.dag[address].clone()),
                        }),
                    },
                );
            }
        }
        if !(self.dag.knows(DagAddress::new(self.round, self.me))
            && self.dag.get_round_valid_count(self.round) >= self.quorum)
        {
            return;
        }
        if self.round.as_u64() % 4 == 0 {
            self.wave_ready();
        }
        // round transition
        self.round += 1;
        self.dag.new_round();
        self.output.state_message(Consensus::RoundTransition {
            new_round: self.round,
        });
        let _ = self.create_vertex(false);
    }

    fn process_vertex_request(&mut self, from: ReplicaId, address: DagAddress) {
        if from == self.me {
            return;
        }
        if self.dag.knows(address) {
            self.output.unicast(
                from,
                PeerMessage {
                    from: self.me,
                    message: Message::Signed(SignedMessage {
                        payload: NonEquivocationPayload(self.dag[address].clone()),
                    }),
                },
            );
        } else {
            self.vertex_requests
                .entry(address)
                .or_insert_with(|| ReplicaSet::with_capacity(self.n))
                .insert(from);
        }
    }

    #[allow(clippy::type_complexity)]
    fn create_vertex(
        &mut self,
        allowed_to_be_empty_or_ahead_of_time: bool,
    ) -> Option<Vertex<Tx, Enc::Signature>> {
        if self.round < self.restart_round {
            if !self.call_create_vertex {
                self.output.set_timeout(self.vertex_timeout);
                self.call_create_vertex = true;
                self.allow_timeout = true;
            }
            return None;
        }

        let would_break = !self
            .pending_client_requests
            .full_block_or_at_least(self.min_vertex_size);

        let timeout_already_passed = self.last_vertex_created_at.elapsed() > self.vertex_timeout;

        let execute =
            !would_break || timeout_already_passed || allowed_to_be_empty_or_ahead_of_time;

        if execute {
            let txs = self.pending_client_requests.pop_front();

            let mut edges = ReplicaSet::with_capacity(self.n);
            if self.round.as_u64() > 1 {
                for edge in self.dag.get_round(self.round - 1) {
                    edges.insert(edge.creator());
                }
            }

            let vertex = match Vertex::new(self.round, self.me, edges, txs, |vertex| {
                let signable = vertex.clone();

                let (signature, counter) = self.enclave.sign(signable)?;

                Ok((signature, counter))
            }) {
                Ok(vertex) => vertex,
                Err(e) => {
                    self.output.state_message(Error::EnclaveError(e));
                    return None;
                }
            };

            let payload = NonEquivocationPayload(vertex);

            let new_message = SignedMessage { payload };

            self.output.broadcast(PeerMessage {
                from: self.me,
                message: Message::Signed(new_message.clone()),
            });

            self.last_vertex_created_at = Instant::now();
            self.call_create_vertex = false;
            self.output.cancel_timeout();
            self.allow_timeout = false;

            #[allow(irrefutable_let_patterns)]
            if let NonEquivocationPayload(new_vertex) = new_message.payload {
                Some(new_vertex)
            } else {
                unreachable!()
            }
        } else {
            self.call_create_vertex = true;
            self.output.set_timeout(
                self.vertex_timeout
                    .checked_sub(self.last_vertex_created_at.elapsed())
                    .unwrap_or(Duration::ZERO),
            );
            self.allow_timeout = true;
            None
        }
    }

    fn wave_ready(&mut self) {
        assert!(self.round.as_u64() >= 4);
        // try to derive consensus and deliver vertices
        let wave = Wave::from(self.round);
        let mut proof = Vec::with_capacity(self.n as usize);
        for va in self.dag.get_round(self.round) {
            proof.push(self.dag[va].deref().clone());
        }
        let wave_leader = match self.enclave.toss(proof.clone()) {
            Ok(leader) => leader,
            Err(e) => {
                error!("could not toss wave {}: {:?},\nproof: {:?}", wave, e, proof);
                self.output.state_message(Error::EnclaveError(e));
                return;
            }
        };
        self.pending_wave_coins.insert(wave, wave_leader);

        if !self.dag.knows(DagAddress::new(round(wave, 1), wave_leader)) {
            self.output.state_message(Consensus::UnknownWaveLeader);
            return;
        }

        let mut reaching_leader = 0u64;
        for va in self.dag.get_round(round(wave, 4)) {
            if self.dag.reaches(va, wave_leader) {
                reaching_leader += 1;
            }
        }

        if reaching_leader < self.quorum {
            self.output
                .state_message(Consensus::DirectCommitRuleViolated);
            return;
        }

        self.output.log_decision_time();

        let mut v = DagAddress::new(round(wave, 1), wave_leader);
        let mut leaders_stack = Vec::new();
        leaders_stack.push(v);
        let mut decided_waves = Vec::new();
        decided_waves.push(wave);
        let mut wave_prime = wave - 1;
        while wave_prime > self.decided_wave {
            let v_prime = DagAddress::new(
                round(wave_prime, 1),
                *self.pending_wave_coins.get(&wave_prime).unwrap(),
            );
            if self.dag.knows(v_prime) && self.dag.has_path(v, v_prime) {
                v = v_prime;
                leaders_stack.push(v);
                decided_waves.push(wave_prime);
            }
            wave_prime -= 1;
        }

        let mut order = Vec::new();
        let mut leaders = Vec::new();
        while let Some(leader) = leaders_stack.pop() {
            leaders.push(leader);
            for va in self.dag.deliver_from(leader, 0.into()) {
                order.push(va);
                for tx in self.dag[va].transactions() {
                    let entry = self.client_last_request.entry(tx.client_id());
                    if let Entry::Occupied(entry) = &entry {
                        if *entry.get() >= tx.request_id() {
                            continue;
                        }
                    }
                    entry.insert_entry(tx.request_id());
                    self.output.append_delivery(tx.clone());
                }
            }
        }

        decided_waves.reverse();
        self.output.state_message(Consensus::Decided {
            waves: decided_waves,
            leaders,
            order,
        });

        self.decided_wave = wave;
        self.pending_wave_coins.clear();
    }

    fn reflect(&mut self) -> Output<Tx, Enc> {
        let UnreflectedOutput {
            reflection_messages,
            mut unicasts,
            mut state_messages,
            mut delivery,
            mut vertex_timeout,
            mut broadcasts,
            mut backup_enclave,
            mut decision_times,
            ..
        } = std::mem::replace(&mut self.output, UnreflectedOutput::new(self.me));
        for to_reflect in reflection_messages.into_iter() {
            let Output {
                state_messages: mut reflection_state,
                delivery: mut reflection_delivery,
                vertex_timeout: reflection_timeout,
                broadcasts: mut reflection_broadcasts,
                unicasts: mut reflection_unicasts,
                backup_enclave: reflection_backup_enclave,
                decision_times: mut new_decision_times,
            } = self.process_peer_message(to_reflect);
            unicasts.append(&mut reflection_unicasts);
            state_messages.append(&mut reflection_state);
            delivery.append(&mut reflection_delivery);
            vertex_timeout = reflection_timeout;
            broadcasts.append(&mut reflection_broadcasts);
            decision_times.append(&mut new_decision_times);
            backup_enclave |= reflection_backup_enclave;
        }
        Output {
            broadcasts,
            unicasts,
            state_messages,
            delivery,
            vertex_timeout,
            backup_enclave,
            decision_times,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::NxBft;
    use hashbar::{Hashbar, Hasher};
    use nxbft_base::enclave::simple::SimpleEnclave;
    use serde::Serialize;
    use shared_ids::{ClientId, ReplicaId, RequestId};
    use transaction_trait::Transaction;

    #[derive(Clone, Debug, Serialize, Hash, PartialEq, Eq)]
    struct SampleTx {}

    impl Hashbar for SampleTx {
        fn hash<H: Hasher>(&self, _hasher: &mut H) {}
    }

    impl Transaction for SampleTx {
        fn client_id(&self) -> ClientId {
            ClientId::from_u64(0)
        }
        fn request_id(&self) -> RequestId {
            RequestId::from_u64(0)
        }
    }

    #[test]
    #[should_panic]
    fn invalid_peer_count() {
        let _ = NxBft::<SampleTx, SimpleEnclave>::new(
            ReplicaId::from_u64(0),
            2,
            Duration::from_micros(1000),
            1,
            1,
        );
    }

    #[test]
    fn reflection() {
        let mut instance = NxBft::<SampleTx, SimpleEnclave>::new(
            ReplicaId::from_u64(0),
            3,
            Duration::from_micros(1000),
            1,
            1,
        );
        let result_1 = instance.init_handshake();
        assert_eq!(result_1.delivery.len(), 0);
        assert_eq!(result_1.broadcasts.len(), 2);
        assert_eq!(instance.hello_set.len(), 1);
        assert_eq!(instance.hello_map.len(), 1);
        let (_, echos) = &instance.hello_map[&ReplicaId::from_u64(0)];
        assert_eq!(echos.len(), 1);
        assert_eq!(instance.hello_reply_count, 0);
        assert_eq!(instance.quorum, 2);
        assert_eq!(instance.ready_set.len(), 0);
        assert!(!instance.ready);
    }
}
