pub mod blockstore;
pub mod enclave;
mod message;
mod output;

use std::collections::hash_map::Entry;
use std::fmt::Debug;
use std::time::Instant;
use std::{collections::HashMap, time::Duration};

use blockstore::{Block, Blockstore};
use damysus_base::enclave::{BlockHash, Enclave, EnclaveError, Justification, SignedCommitment};
use damysus_base::view::View;
use damysus_base::Parent;
use heaps::pending_client_requests::PendingClientRequests;
use heaps::HeapBuffer;
use id_set::ReplicaSet;
pub use message::PeerMessage;
use message::{Message, MessageDestination};
pub use output::Output;
pub use output::Timeout;
use output::UnreflectedOutput;
use shared_ids::{ClientId, ReplicaId, RequestId};
use tracing::{debug, debug_span, error, info, warn};
use transaction_trait::Transaction;

pub struct Damysus<Tx: Transaction + Clone + Debug, Enc: Enclave> {
    n: u64,
    quorum: usize,
    me: ReplicaId,
    view: View,
    enclave: Enc,
    pending_client_requests: PendingClientRequests<Tx>,
    stored_commitments: HeapBuffer<SignedCommitment<Enc::Signature>>,
    output: UnreflectedOutput<Tx, Enc>,
    commitments: Vec<SignedCommitment<Enc::Signature>>,
    commitment_senders: ReplicaSet,
    peers_added: u64,
    minimum_block_size: usize,
    call_try_create_block: bool,
    block_timeout: Duration,
    view_timeout: Duration,
    initial_view_timeout: Duration,
    executed_transactions: HashMap<ClientId, RequestId>,
    blockstore: Blockstore<Tx, Enc::Signature>,
    open_block_requests: HashMap<View, ReplicaSet>,
    stored_blocks: HeapBuffer<Block<Tx, Enc::Signature>>,
    requested_blocks: HashMap<View, BlockHash>,
    last_round_finished_at: Instant,
}

impl<Tx: Transaction + Clone + Debug, Enc: Enclave> Damysus<Tx, Enc> {
    pub fn new(
        me: ReplicaId,
        n: u64,
        minimum_block_size: u64,
        maximum_block_size: u64,
        block_timeout: Duration,
        initial_view_timeout: Duration,
    ) -> Self {
        assert_ne!(maximum_block_size, 0);
        assert!(minimum_block_size <= maximum_block_size);
        let quorum = (n / 2) + 1;
        Self {
            n,
            quorum: quorum as usize,
            me,
            view: View::default(),
            enclave: Enc::new(me, n, quorum),
            pending_client_requests: PendingClientRequests::new(maximum_block_size as usize),
            output: UnreflectedOutput::new(me),
            stored_commitments: HeapBuffer::default(),
            commitments: Vec::with_capacity(n as usize),
            commitment_senders: ReplicaSet::with_capacity(n),
            peers_added: 0,
            minimum_block_size: minimum_block_size as usize,
            call_try_create_block: false,
            block_timeout,
            view_timeout: initial_view_timeout,
            executed_transactions: HashMap::new(),
            blockstore: Blockstore::default(),
            open_block_requests: HashMap::new(),
            stored_blocks: HeapBuffer::default(),
            requested_blocks: HashMap::default(),
            initial_view_timeout,
            last_round_finished_at: Instant::now(),
        }
    }

    pub fn init_handshake(&mut self) -> Output<Tx, Enc> {
        let _damysus_span = debug_span!(
            "damysus - init handshake",
            id = self.me.as_u64(),
            view = self.view.as_u64()
        )
        .entered();
        let key = self
            .enclave
            .get_public_key()
            .expect("Cannot receive enclave public key");
        self.enclave
            .add_peer(self.me, key.clone())
            .expect("Cannot add own enclave public key to enclave");
        self.peers_added += 1;
        self.broadcast(Message::Handshake(key));
        debug!("Broadcasting public key handshake");
        self.reflect()
    }

    pub fn process_peer_message(
        &mut self,
        peer_message: PeerMessage<Tx, Enc::Signature, Enc::PublicKey>,
    ) -> Output<Tx, Enc> {
        let _damysus_span = debug_span!(
            "damysus - peer message",
            id = self.me.as_u64(),
            view = self.view.as_u64()
        )
        .entered();
        match peer_message.message {
            Message::NewBlock { block, prepare } => self.process_block(block, prepare),
            Message::Commitment(commitment) => {
                self.process_commitment(peer_message.from, commitment)
            }
            Message::Handshake(key) => self.process_handshake(peer_message.from, key),
            Message::BlockRequest(block_view) => {
                self.process_block_request(peer_message.from, block_view)
            }
            Message::RequestedBlock(block) => self.process_requested_block(block),
        }
        self.reflect()
    }

    pub fn receive_client_request(&mut self, request: Tx) -> Output<Tx, Enc> {
        let _damysus_span = debug_span!(
            "damysus - client request",
            id = self.me.as_u64(),
            view = self.view.as_u64()
        )
        .entered();
        debug!("Received client request");
        self.pending_client_requests.insert(request);
        if self.call_try_create_block {
            if self
                .pending_client_requests
                .full_block_or_at_least(self.minimum_block_size)
                || self.last_round_finished_at.elapsed() > self.block_timeout
            {
                debug!("Received enough client requests to propose block for which I already had a quorum");
                self.create_block();
            } else {
                debug!("I have a quorum but not enough client requests to propose block");
            }
        } else if self.is_view_lead_by_me(self.view) {
            debug!("I do not have a quorum for current round");
        }
        self.reflect()
    }

    pub fn handle_block_timeout(&mut self) -> Output<Tx, Enc> {
        let _damysus_span = debug_span!(
            "damysus - block timeout",
            id = self.me.as_u64(),
            view = self.view.as_u64()
        )
        .entered();
        if !self.call_try_create_block {
            warn!("Block timeout invoked when not having a quorum for current round");
        } else {
            debug!("Creating block due to timeout");
            self.create_block();
        }
        self.reflect()
    }

    pub fn handle_view_timeout(&mut self) -> Output<Tx, Enc> {
        let _damysus_span = debug_span!(
            "damysus - view_timeout",
            id = self.me.as_u64(),
            view = self.view.as_u64()
        )
        .entered();
        if self.is_view_lead_by_me(self.view) {
            debug!("Won't transition to new view, I am its leader!");
            return self.reflect();
        }
        warn!(
            "View transition due to timeout (from view: {}, to view: {}, leader was: {})",
            self.view,
            self.view.get_successor(),
            self.view.get_view_leader(self.n).as_u64()
        );
        self.new_view();
        self.reflect()
    }

    fn process_block(
        &mut self,
        block: Block<Tx, Enc::Signature>,
        prepare: SignedCommitment<Enc::Signature>,
    ) {
        if block.get_view() < self.view {
            match self.blockstore.get(block.get_view()) {
                None => {
                    debug!(
                        "Block for view {} received which timed out",
                        block.get_view()
                    )
                }
                Some(existing_block) if existing_block.hash_value() != block.hash_value() => {
                    error!("Block for view {} received which I already had but with different hash value!", block.get_view());
                    unreachable!()
                }
                _ => {
                    debug!(
                        "Block for view {} received but already known",
                        block.get_view()
                    );
                    return;
                }
            }
        }
        debug!("Block for view {} is being processed", block.get_view());
        let Some(accumulator) = block.get_justification().get_commitment() else {
            warn!("Received block with Genesis justification!");
            return;
        };

        if !accumulator.is_accumulator() {
            warn!("Received block with usig instead of accumulator as justification!");
            return;
        }

        if accumulator.get_view_usable_for() != block.get_view() {
            warn!("Accumulator cannot be used for block!");
            return;
        }

        if !self.enclave.verify_untrusted(
            accumulator.get_creator(),
            accumulator.hash_value(),
            accumulator.get_signature(),
        ) {
            warn!("Received block with invalid accumulator signature!");
            return;
        }

        if !prepare.is_usig() {
            warn!("Accompanying prepare is accumulator instead of usig");
            return;
        }

        if !prepare.is_prepare() {
            warn!("Leader used new view instead of prepare for block proposal");
            return;
        }

        if prepare.get_block_hash() != block.hash_value() {
            warn!("Accompanying prepare does not prepare the block");
            return;
        }

        if prepare.get_view() != block.get_view() {
            warn!("Block view and prepare view do not match!");
            return;
        }

        if !self.enclave.verify_untrusted(
            prepare.get_creator(),
            prepare.hash_value(),
            prepare.get_signature(),
        ) {
            warn!("Prepare is not validly signed");
            return;
        }

        if self.open_block_requests.contains_key(&block.get_view()) {
            for to in self
                .open_block_requests
                .remove(&block.get_view())
                .unwrap()
                .iter()
            {
                debug!(
                    "Answering block request for view {} by {} delayed",
                    block.get_view(),
                    to
                );
                self.unicast(to, Message::RequestedBlock(block.clone()));
            }
        }

        if self.requested_blocks.remove(&block.get_view()).is_some() {
            debug!(
                "Received block for view {}, was requested but came in normally",
                block.get_view()
            );
        }

        if block.get_view() > self.view {
            if self.stored_blocks.knows(&block) {
                debug!("Received future block twice");
                return;
            }
            debug!(
                "Block for view {} stored for future handling",
                block.get_view()
            );
            self.stored_blocks.insert(block);
            return;
        }

        self.add_and_prepare_block(block);
    }

    fn process_commitment(
        &mut self,
        from: ReplicaId,
        commitment: SignedCommitment<Enc::Signature>,
    ) {
        debug!("Process commitment");
        if from != commitment.get_creator() {
            warn!(
                "Commitments must not be forwarded. Received commitment by {} from {}",
                commitment.get_creator(),
                from
            );
            return;
        }

        if commitment.get_view_usable_for() < self.view {
            debug!("Dropped outdated commitment by {} for view {} and usable for view {} (my view: {})", commitment.get_creator(), commitment.get_view(), commitment.get_view_usable_for(), self.view);
            return;
        }

        if !self.enclave.verify_untrusted(
            commitment.get_creator(),
            commitment.hash_value(),
            commitment.get_signature(),
        ) {
            warn!(
                "Received invalidly signed commitment by {} for view {}",
                commitment.get_creator(),
                commitment.get_view()
            );
            return;
        }

        if !self.is_view_lead_by_me(commitment.get_view_usable_for()) {
            warn!("Received commitment by {} for view {} and usable for view {} which I am not leader of.", commitment.get_creator(), commitment.get_view(), commitment.get_view_usable_for());
            return;
        }

        if !commitment.is_usig() {
            warn!(
                "Received accumulator instead of usig as commitment by {} for view {}",
                commitment.get_creator(),
                commitment.get_view()
            );
            return;
        }

        if commitment.get_view_usable_for() > self.view {
            if self.stored_commitments.knows(&commitment) {
                warn!(
                    "Received commitment by {} usable in view {} more than once",
                    commitment.get_creator(),
                    commitment.get_view_usable_for()
                );
            } else {
                debug!(
                    "Received commitment by {} usable in view {} ahead of time",
                    commitment.get_creator(),
                    commitment.get_view_usable_for()
                );
                self.stored_commitments.insert(commitment);
                debug!("Stored commitment for future handling");
            }
            return;
        }
        self.apply_commitment(commitment);
    }

    fn process_handshake(&mut self, from: ReplicaId, key: Enc::PublicKey) {
        if let Err(e) = self.enclave.add_peer(from, key) {
            if matches!(e, EnclaveError::DoubleKeyReceived) {
                debug!("Received key from peer {from} twice");
            } else {
                error!("Cannot add key from peer {from}, reason: {:?}", e);
                panic!("Cannot add key from peer {from}, reason: {:?}", e);
            }
        } else {
            self.peers_added += 1;
            debug!("Added peer {from}");

            if self.peers_added == self.n {
                self.new_view();
                self.output.ready();
                info!("Ready to process client requests (new-view to View 1)");
            }
        }
    }

    fn process_block_request(&mut self, from: ReplicaId, block_view: View) {
        if let Some(block) = self.blockstore.get(block_view) {
            debug!(
                "Answering block request for view {} by {}",
                block_view, from
            );
            self.unicast(from, Message::RequestedBlock(block.clone()));
        } else if self.stored_blocks.knows_key(&block_view) {
            let block = self.stored_blocks.find_by_key(&block_view).unwrap();
            debug!(
                "Answering block request for view {} by {}",
                block_view, from
            );
            self.unicast(from, Message::RequestedBlock(block.clone()));
        } else {
            self.open_block_requests
                .entry(block_view)
                .or_insert_with(|| ReplicaSet::with_capacity(self.n))
                .insert(from);
            debug!(
                "Block request for view {} received from {}, I do not have it yet",
                block_view, from
            );
        }
    }

    fn process_requested_block(&mut self, block: Block<Tx, Enc::Signature>) {
        debug!(
            "Block for view {} received in block request response",
            block.get_view()
        );
        if let Some(hash) = self.requested_blocks.remove(&block.get_view()) {
            if hash != *block.hash_value() {
                warn!("Received a requested block with invalid hash!");
                return;
            }
        } else {
            debug!("Received a requested block I did not ask for (anymore)");
            return;
        }
        if self.blockstore.get(block.get_view()).is_some() {
            error!("Requested and received a block I already have");
            panic!("Requested and received a block I already have");
        }
        self.add_and_prepare_block(block);

        if let Some(stored_block) = self.stored_blocks.peek() {
            if stored_block.get_view() == self.view {
                let stored_block = self.stored_blocks.pop().unwrap();
                self.add_and_prepare_block(stored_block);
            }
        }
    }

    fn apply_commitment(&mut self, commitment: SignedCommitment<Enc::Signature>) {
        debug!("Apply commitment for view {} by {} (new_view/prepare: {}/{}, usig/accumulator: {}/{}, block view: {}, prepared: {})", commitment.get_view_usable_for(), commitment.get_creator(), !commitment.is_prepare(), commitment.is_prepare(), commitment.is_usig(), commitment.is_accumulator(), commitment.get_block_view(), commitment.get_prepared_block_view());
        if self.commitment_senders.contains(commitment.get_creator()) {
            warn!(
                "Received commitment by {} for view {} more than once",
                commitment.get_creator(),
                commitment.get_view()
            );
            return;
        }

        if self.blockstore.get(commitment.get_block_view()).is_some()
            && commitment.get_block_hash()
                != self
                    .blockstore
                    .get(commitment.get_block_view())
                    .unwrap()
                    .hash_value()
        {
            warn!(
                "Received commitment for block {} that diverges in my local store (local: {}, received: {})",
                commitment.get_block_view(), self.blockstore.get(commitment.get_block_view()).unwrap().hash_value(), commitment.get_block_hash()
            );
            return;
        }

        self.commitment_senders.insert(commitment.get_creator());
        self.commitments.push(commitment);

        if self.commitments.len() < self.quorum {
            debug!("Insufficient commitments to propose view {}", self.view);
            return;
        }

        let timeout_already_passed = self.last_round_finished_at.elapsed() > self.block_timeout;

        if timeout_already_passed
            || self
                .pending_client_requests
                .full_block_or_at_least(self.minimum_block_size)
        {
            self.create_block();
        } else {
            self.call_try_create_block = true;
            self.output.set_block_timeout(
                self.block_timeout
                    .checked_sub(self.last_round_finished_at.elapsed())
                    .unwrap_or(Duration::ZERO),
            );
            debug!("Not enough client requests to propose block");
        }
    }

    fn create_block(&mut self) {
        debug!(
            "Create block for view {} with {} commitments",
            self.view,
            self.commitments.len()
        );
        self.call_try_create_block = false;

        self.commitments
            .sort_unstable_by_key(|sc| sc.get_prepared_block_view());

        let accumulator = self
            .enclave
            .accumulate(&self.commitments)
            .expect("Could not interact with enclave");

        self.pending_client_requests.filter_front(|tx| {
            if let Some(last_req_id) = self.executed_transactions.get(&tx.client_id()) {
                if *last_req_id >= tx.request_id() {
                    return false;
                }
            }
            true
        });

        let txs_temp = self.pending_client_requests.copy_front();

        let parent =
            if let Some(parent_block) = self.blockstore.get(self.view.get_predecessor().unwrap()) {
                debug!("Parent is {:?}", parent_block.hash_value());
                Parent::Some(parent_block.hash_value().clone())
            } else {
                debug!(
                "Creating a block with no parent as we did not receive a block for previous view"
            );
                Parent::None
            };
        let block = Block::new(
            self.me,
            self.view,
            Justification::Quorum(accumulator),
            parent,
            txs_temp,
        );
        debug!("Block for view {} created", block.get_view());
        let prepare = self
            .add_and_prepare_block(block.clone())
            .expect("Cannot prepare block that was created by me!");
        assert!(prepare.is_usig());

        self.broadcast(Message::NewBlock { block, prepare });
    }

    fn add_and_prepare_block(
        &mut self,
        mut block: Block<Tx, Enc::Signature>,
    ) -> Option<SignedCommitment<Enc::Signature>> {
        block.not_executed();
        let block = block;
        let accumulator = block.get_justification().get_commitment().unwrap();
        debug!(
            "Inserting block for view {} justified by block for view {}",
            block.get_view(),
            accumulator.get_block_view()
        );

        if let Parent::Some(hash) = block.get_parent() {
            if self
                .blockstore
                .get(block.get_view().get_predecessor().unwrap())
                .is_none()
            {
                let pre_view = block.get_view().get_predecessor().unwrap();
                if let std::collections::hash_map::Entry::Vacant(e) =
                    self.requested_blocks.entry(pre_view)
                {
                    info!(
                        "Need to request parent block {} from peers",
                        block.get_view().get_predecessor().unwrap()
                    );
                    e.insert(hash.clone());
                    self.broadcast(Message::BlockRequest(
                        block.get_view().get_predecessor().unwrap(),
                    ));
                } else {
                    debug!(
                        "Parent block for view {} is missing, but requested",
                        block.get_view()
                    );
                }
            }
        }

        if self.blockstore.get(accumulator.get_block_view()).is_none() {
            if let std::collections::hash_map::Entry::Vacant(e) =
                self.requested_blocks.entry(accumulator.get_block_view())
            {
                info!(
                    "Need to request justification block {} from peers",
                    accumulator.get_block_view(),
                );
                e.insert(accumulator.get_block_hash().clone());
                self.broadcast(Message::BlockRequest(accumulator.get_block_view()));
            } else {
                debug!(
                    "Block for view {} is missing, but requested",
                    accumulator.get_block_view()
                );
            }
        }

        if self.view != block.get_view() {
            debug!(
                "Block for view {} is for timed-out view, not preparing or executing",
                block.get_view()
            );
            self.blockstore.insert(block);
            return None;
        }

        let justification_block_hash = block
            .get_justification()
            .get_commitment()
            .unwrap()
            .get_block_hash()
            .clone();
        let prepare = match self.enclave.prepare(
            block.get_view(),
            block.hash_value().clone(),
            block.get_justification().clone(),
            block.get_parent().clone(),
            justification_block_hash,
        ) {
            Ok(commitment) => commitment,
            Err(EnclaveError::PlatformError(e)) => {
                error!("Enclaved execution failed: {:?}", e);
                panic!("Enclaved execution failed: {:?}", e);
            }
            Err(e) => {
                error!(
                    "Could not prepare block for view {} created by {} (is leader: {}) (in view {}) :\n\t{:?}",
                    block.get_view(),
                    block.get_creator().as_u64(),
                    block.get_view().get_view_leader(self.n) == block.get_creator(),
                    self.view,
                    e
                );
                panic!("Enclaved execution failed: {:?}", e);
            }
        };
        debug!("Sending prepare for block for view {}", block.get_view());
        self.unicast(
            prepare.get_view_usable_for().get_view_leader(self.n),
            Message::Commitment(prepare.clone()),
        );

        if let Some(justification_block) = self.blockstore.get(accumulator.get_block_view()) {
            debug!(
                "Checking chain of children of block for view {} for possible execution from grand child on",
                block.get_view()
            );
            if let Justification::Quorum(grand_justification) =
                justification_block.get_justification()
            {
                if let Some(grand_justification_block) =
                    self.blockstore.get(grand_justification.get_block_view())
                {
                    if block.get_parent() == justification_block.hash_value()
                        && justification_block.get_parent()
                            == grand_justification_block.hash_value()
                    {
                        debug!(
                            "Trying to execute from block for view {} on, chain {}-{}-{}",
                            grand_justification_block.get_view(),
                            block.get_view(),
                            justification_block.get_view(),
                            grand_justification_block.get_view()
                        );
                        self.try_execute_from(grand_justification_block.get_view());
                    } else {
                        debug!(
                                "Block for view {} is not executable, parent chain {}-{}-{} vs. justification chain {}-{}-{}",
                                block.get_view().get_predecessor().unwrap().get_predecessor().unwrap(),
                                block.get_view(),
                                block.get_view().get_predecessor().unwrap(),
                                block.get_view().get_predecessor().unwrap().get_predecessor().unwrap(),
                                block.get_view(),
                                justification_block.get_view(),
                                grand_justification_block.get_view()
                            );
                        debug!(
                                "Parent: {:?}, Justification: {:?}, Justifcation Parent: {:?}, Grand Justification: {:?}",
                                block.get_parent(),
                                justification_block.hash_value(),
                                justification_block.get_parent(),
                                grand_justification_block.hash_value()
                            );
                    }
                } else {
                    debug!("Chain of children of block for view {} is not complete, grand justifcation block is missing", block.get_view());
                }
            }
        } else {
            debug!("Chain of children of block for view {} is not complete, justifcation block is missing", block.get_view());
        }

        self.blockstore.insert(block);
        self.round_transition(false);
        Some(prepare)
    }

    fn round_transition(&mut self, timeout: bool) {
        debug!(
            "Round transition from {} to {}",
            self.view,
            self.view.get_successor()
        );
        let old_view_timeout = self.view_timeout;
        if timeout {
            self.view_timeout = self.view_timeout.checked_mul(2).unwrap_or(Duration::MAX);
        } else {
            self.view_timeout = std::cmp::max(
                self.initial_view_timeout,
                self.view_timeout - self.initial_view_timeout,
            );
        }

        if self.view_timeout != old_view_timeout {
            info!(
                "View timeout changed from {}s to {}s",
                old_view_timeout.as_secs(),
                self.view_timeout.as_secs()
            );
        }

        self.view.increment();
        self.commitment_senders.clear();
        self.commitments.clear();
        self.last_round_finished_at = Instant::now();
        self.output.cancel_block_timeout();

        self.output.set_view_timeout(self.view_timeout);

        if let Some(block) = self.stored_blocks.peek() {
            if block.get_view() == self.view {
                let block = self.stored_blocks.pop().unwrap();
                self.add_and_prepare_block(block);
            }
        }

        while let Some(commitment) = self.stored_commitments.peek() {
            if commitment.get_view_usable_for() < self.view
                || !self.is_view_lead_by_me(commitment.get_view_usable_for())
            {
                self.stored_commitments.pop();
                continue;
            }
            if commitment.get_view_usable_for() > self.view {
                break;
            }

            let commitment = self.stored_commitments.pop().unwrap();
            self.apply_commitment(commitment);
        }
    }

    fn try_execute_from(&mut self, block_view: View) {
        let mut to_execute = Vec::new();
        let mut block_view = block_view;

        loop {
            let Some(block) = self.blockstore.get(block_view) else {
                debug!(
                    "Block for view {} is missing, execution canceled",
                    block_view
                );
                return;
            };

            if block.is_executed() {
                debug!(
                    "Block for view {} already executed, all blocks to execute identified",
                    block_view
                );
                break;
            }

            if block.is_genesis() {
                debug!("Genesis block reached, all blocks to execute identified");
                break;
            }

            to_execute.push(block_view);

            block_view = block
                .get_justification()
                .get_commitment()
                .unwrap()
                .get_block_view();
        }

        self.output.log_decision_time();

        for block_view in to_execute.into_iter().rev() {
            self.execute_block(block_view);
        }
        debug!("All blocks executed");
    }

    fn execute_block(&mut self, block_view: View) {
        debug!("Block for view {} executed", block_view);

        let block = self.blockstore.get_mut(block_view).unwrap();
        block.execute();
        for tx in block.get_transactions() {
            let entry = self.executed_transactions.entry(tx.client_id());
            if let Entry::Occupied(entry) = &entry {
                if *entry.get() >= tx.request_id() {
                    continue;
                }
            }
            entry.insert_entry(tx.request_id());
            self.output.append_delivery(tx.clone());
        }
    }

    fn new_view(&mut self) {
        let new_view = self
            .enclave
            .re_sign_last_prepared()
            .expect("Cannot re-sign last prepared block");
        self.unicast(
            new_view.get_view_usable_for().get_view_leader(self.n),
            Message::Commitment(new_view),
        );
        self.round_transition(true);
    }

    fn is_view_lead_by_me(&self, view: View) -> bool {
        view.get_view_leader(self.n) == self.me
    }

    fn broadcast(&mut self, message: Message<Tx, Enc::Signature, Enc::PublicKey>) {
        self.output.append_peer_message(
            MessageDestination::Broadcast,
            PeerMessage {
                from: self.me,
                message,
            },
        )
    }

    fn unicast(&mut self, to: ReplicaId, message: Message<Tx, Enc::Signature, Enc::PublicKey>) {
        self.output.append_peer_message(
            MessageDestination::Unicast(to),
            PeerMessage {
                from: self.me,
                message,
            },
        );
    }

    fn reflect(&mut self) -> Output<Tx, Enc> {
        let _minbft_span = debug_span!("reflection").entered();
        let UnreflectedOutput {
            reflection_messages,
            mut block_timeout,
            mut broadcasts,
            mut delivery,
            mut unicasts,
            mut view_timeout,
            mut ready,
            mut decision_times,
            ..
        } = std::mem::replace(&mut self.output, UnreflectedOutput::new(self.me));
        let mut view = self.view.as_u64();
        for to_reflect in reflection_messages.into_iter() {
            let Output {
                broadcasts: mut reflection_broadcasts,
                unicasts: mut reflection_unicasts,
                delivery: mut reflection_delivery,
                view_timeout: reflection_view_timeout,
                block_timeout: reflection_block_timeout,
                ready: reflection_ready,
                view: reflection_view,
                decision_times: mut new_decision_times,
            } = self.process_peer_message(to_reflect);
            broadcasts.append(&mut reflection_broadcasts);
            unicasts.append(&mut reflection_unicasts);
            delivery.append(&mut reflection_delivery);
            block_timeout = reflection_block_timeout;
            view_timeout = reflection_view_timeout;
            ready |= reflection_ready;
            view = std::cmp::max(view, reflection_view);
            decision_times.append(&mut new_decision_times);
        }
        Output {
            block_timeout,
            broadcasts,
            delivery,
            unicasts,
            view_timeout,
            ready,
            view,
            decision_times,
        }
    }

    #[cfg(debug_assertions)]
    pub fn get_executed_blocks(&self) -> Vec<Option<u64>> {
        // let mut view = self.view;
        // let mut just_chain = Vec::new();
        // loop {
        //     if view.as_u64() == 0 {
        //         just_chain.push(view);
        //         break;
        //     }
        //     if let Some(block) = self.blockstore.get(view) {
        //         just_chain.push(view);
        //         view = block
        //             .get_justification()
        //             .get_commitment()
        //             .unwrap()
        //             .get_block_view();
        //     } else {
        //         view = view.get_predecessor().unwrap();
        //     }
        // }
        // just_chain
        //     .iter()
        //     .rev()
        //     .for_each(|v| print!("{} <- ", v.as_u64()));
        // println!();

        self.blockstore
            .get_block_iterator()
            .map(|block| {
                if let Some(block) = block {
                    if block.is_executed() {
                        Some(block.get_view().as_u64())
                    } else {
                        None
                    }
                } else {
                    None
                }
            })
            .collect()
    }
}
