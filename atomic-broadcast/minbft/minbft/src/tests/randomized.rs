use core::panic;
use std::collections::{HashSet, VecDeque};
use std::hash::Hash;
use std::vec;

use super::*;
use crate::ViewState;
use itertools::Itertools;
use rand::prelude::*;
use rand_chacha::ChaCha8Rng;
use test_log::test;
use tracing::{debug, error, info};

#[derive(Debug, Clone)]
#[allow(clippy::large_enum_variant)]
enum Message {
    Client(RequestId, DummyPayload),
    Peer(PeerMessage<DummyAttestation, DummyPayload, usig::noop::Signature>),
}

#[derive(Debug, Eq, PartialEq, Hash, Copy, Clone)]
enum Participant {
    Client(ClientId),
    Replica(ReplicaId),
}

fn receive(network: &mut Network, sender: Participant, receiver: Participant, message: Message) {
    let output;
    match (sender, receiver, message) {
        (
            Participant::Client(client_id),
            Participant::Replica(receiver_id),
            Message::Client(request_id, req),
        ) => {
            let minbft = network.minbfts.get_mut(&receiver_id).unwrap();
            output = minbft.handle_client_message(client_id, request_id, req);
            handle_output(network, receiver_id, output)
        }
        (
            Participant::Replica(sender_id),
            Participant::Replica(receiver_id),
            Message::Peer(msg),
        ) => {
            info!(
                "message from {:?} to {:?}: {:?}",
                sender_id, receiver_id, msg
            );
            let minbft = network.minbfts.get_mut(&receiver_id).unwrap();
            output = minbft.handle_peer_message(sender_id, msg.clone());
            handle_output(network, receiver_id, output)
        }
        (Participant::Replica(_), Participant::Client(_), _) => {}
        (_, _, msg) => panic!(
            "Invalid message from {:?} to {:?}: {:?}",
            sender, receiver, msg
        ),
    }
}

impl From<ClientId> for Participant {
    fn from(client_id: ClientId) -> Self {
        Participant::Client(client_id)
    }
}
impl From<ReplicaId> for Participant {
    fn from(replica_id: ReplicaId) -> Self {
        Participant::Replica(replica_id)
    }
}

impl From<(RequestId, DummyPayload)> for Message {
    fn from((request_id, value): (RequestId, DummyPayload)) -> Self {
        Message::Client(request_id, value)
    }
}

impl From<PeerMessage<DummyAttestation, DummyPayload, usig::noop::Signature>> for Message {
    fn from(value: PeerMessage<DummyAttestation, DummyPayload, usig::noop::Signature>) -> Self {
        Message::Peer(value)
    }
}

struct Network<'a> {
    t: u64,
    current_time: Instant,
    tx_count: u64,
    channels: HashMap<(Participant, Participant), Channel, ConstState>,
    rng: &'a mut dyn RngCore,
    replicas: Vec<ReplicaId>,
    pending_client_requests: HashMap<ClientId, VecDeque<u64>, ConstState>,
    request_collector: HashMap<ClientId, (RequestId, Vec<ReplicaId>), ConstState>,
    minbfts: &'a mut HashMap<ReplicaId, MinBft<DummyPayload, UsigNoOp>, ConstState>,
    timeout_handlers: HashMap<ReplicaId, SimulatedTimeoutHandler, ConstState>,
    crashed: bool,
    error: bool,
}

impl<'a> Network<'a> {
    fn new(
        minbfts: &'a mut HashMap<ReplicaId, MinBft<DummyPayload, UsigNoOp>, ConstState>,
        rng: &'a mut dyn RngCore,
        t: u64,
    ) -> Network<'a> {
        let mut timeout_handlers = HashMap::with_hasher(ConstState);
        for replica_id in minbfts.keys() {
            timeout_handlers.insert(
                *replica_id,
                SimulatedTimeoutHandler(HashMap::new(), *replica_id),
            );
        }
        Network {
            t,
            tx_count: 0,
            current_time: Instant::now(),
            channels: HashMap::with_hasher(ConstState),
            rng,
            replicas: minbfts.keys().cloned().collect(),
            pending_client_requests: HashMap::with_hasher(ConstState),
            request_collector: HashMap::with_hasher(ConstState),
            minbfts,
            timeout_handlers,
            crashed: false,
            error: false,
        }
    }
    fn get_channel<S, R>(&mut self, sender: S, receiver: R) -> &mut Channel
    where
        S: Into<Participant>,
        R: Into<Participant>,
    {
        let sender: Participant = sender.into();
        let receiver: Participant = receiver.into();

        match self.channels.get_mut(&(sender, receiver)) {
            Some(_) => self.channels.get_mut(&(sender, receiver)).unwrap(),
            None => {
                let channel = Channel {
                    sender,
                    receiver,
                    message_queue: VecDeque::new(),
                };
                self.channels.insert((sender, receiver), channel);
                self.channels.get_mut(&(sender, receiver)).unwrap()
            }
        }
    }

    fn broadcast(&mut self, sender: ReplicaId, msg: Message) {
        debug!("minbft{{id={:?}}} broadcasts: {:?}", sender.as_u64(), msg);
        for channel in self.channels.values_mut() {
            if let (Participant::Replica(sender_rep), Participant::Replica(receiver_rep)) =
                (channel.sender, channel.receiver)
            {
                if sender_rep == sender && receiver_rep != sender {
                    channel.queue(msg.clone())
                }
            }
        }
    }
}

/// Handles timeout requests and timeouts.
/// See functions below for a better understanding.
#[derive(Debug, Clone)]
pub(crate) struct SimulatedTimeoutHandler(HashMap<TimeoutType, TimeoutEntry>, ReplicaId);

impl SimulatedTimeoutHandler {
    /// Handles a timeout request.
    /// Sets a timeout if the timeout request itself is a start request and
    /// if there is not already a timeout of the same type set.
    /// Stops a set timeout if the timeout request it self is a stop request and
    /// if the type and the stop class of the timeout in the request is the same as the set timeout.
    fn handle_timeout_request(&mut self, timeout_request: TimeoutRequest, current_time: Instant) {
        info!(
            "new timeout request for {:?}: self.0={:?}, req={:?}",
            self.1, self.0, timeout_request
        );
        if let TimeoutRequest::Start(timeout) = timeout_request {
            if self.0.contains_key(&timeout.timeout_type) {
                return;
            }
            let new_entry = TimeoutEntry {
                timeout_type: timeout.timeout_type,
                timeout_deadline: current_time + timeout.duration,
                stop_class: timeout.stop_class,
            };
            self.0.insert(new_entry.timeout_type, new_entry);
        }
        if let TimeoutRequest::Stop(timeout) = timeout_request {
            if !self.0.contains_key(&timeout.timeout_type) {
                return;
            }
            let current_timeout = self.0.get(&timeout.timeout_type).unwrap();
            if current_timeout.stop_class == timeout.stop_class {
                self.0.remove(&timeout.timeout_type);
            }
        }

        if let TimeoutRequest::StopAny(timeout) = timeout_request {
            if !self.0.contains_key(&timeout.timeout_type) {
                return;
            }
            self.0.remove(&timeout.timeout_type);
        }
    }

    /// Handles a collection of timeout requests.
    fn handle_timeout_requests(
        &mut self,
        timeout_requests: &[TimeoutRequest],
        current_time: Instant,
    ) {
        for timeout_request in timeout_requests.iter() {
            self.handle_timeout_request(timeout_request.to_owned(), current_time);
        }
    }

    fn retrieve_due_timeouts(&mut self, current_time: Instant) -> HashSet<TimeoutType> {
        let mut result = HashSet::new();
        for (timeout_type, timeout_entry) in self.0.iter() {
            if timeout_entry.timeout_deadline <= current_time {
                result.insert(*timeout_type);
            }
        }
        for timeout_type in result.iter() {
            self.0.remove(timeout_type);
        }
        result
    }

    fn has_pending_timeout(&self) -> bool {
        self.0.values().len() != 0
    }
}

#[derive(Debug)]
struct Channel {
    sender: Participant,
    receiver: Participant,
    message_queue: VecDeque<Message>,
}

impl Channel {
    fn queue<T>(&mut self, message: T)
    where
        T: Into<Message>,
    {
        self.message_queue.push_back(message.into());
    }

    fn receive(&mut self) -> Option<Message> {
        self.message_queue.pop_front()
    }
    fn has_message(&self) -> bool {
        !self.message_queue.is_empty()
    }
}
/*
//#[test]
fn test_until_fail_par() {
    (0..).par_bridge().for_each(|i| {
        println!("IIIIIIIIIIIIIIIIIIIIIIIIIIIIIIIIIIIII   i={:?} IIIIIIIIIIIIIIIIIIIIIIIIIIIIIIIIIIIII", i);
        if std::panic::catch_unwind(|| randomized(ChaCha8Rng::seed_from_u64(i))).is_err() {
            eprintln!(
                "IIIIIIIIIIIIIIIIIIIIIIIIIIIIIIIIIIIII ERROR ON ={:?} ERROR ON =IIIIIIIIIIIIIIIIIIIIIIIIIII",
                i
            );
            std::process::exit(1);
        }
        println!("IIIIIIIIIIIIIIIIIIIIIIIIIIIIIIIIIIIII  done i={:?} IIIIIIIIIIIIIIIIIIIIIIIIIIIIIIIIIIIII", i);
    })
}*/

#[test]
fn test_until_fail() {
    (0..32).for_each(|i| {
        println!("i={:?}", i);
        randomized(ChaCha8Rng::seed_from_u64(i));
        println!("done i={:?}", i);
    })
}

/*#[test]
//[ignore]
fn only_this() {
    let i = 239;
    randomized(ChaCha8Rng::seed_from_u64(i));
}*/

fn randomized(mut rng: ChaCha8Rng) {
    let n = 3;
    let checkpoint_period = 5;
    let t = n / 2;

    let amount_of_requests = 200;
    let amount_of_clients = 1;
    let (mut minbfts, _): SetupSet = setup_set(n, t, checkpoint_period);
    let mut network = Network::new(&mut minbfts, &mut rng, t);

    for client_id in n..(amount_of_clients + n) {
        let mut requests = VecDeque::new();
        for req_id in 1..amount_of_requests {
            requests.push_front(req_id);
        }
        network
            .pending_client_requests
            .insert(ClientId::from_u64(client_id), requests);
        for rep_id in network.replicas.clone() {
            network
                .get_channel(
                    Participant::Client(ClientId::from_u64(client_id)),
                    Participant::Replica(rep_id),
                )
                .queue((0.into(), DummyPayload));
        }
    }

    for rep_id_sender in 0..n {
        for rep_id_receiver in 0..n {
            if rep_id_sender != rep_id_receiver {
                network.get_channel(
                    ReplicaId::from_u64(rep_id_sender),
                    ReplicaId::from_u64(rep_id_receiver),
                );
            }
        }
    }
    let mut i = 0;
    let vc1: u64 = network.rng.gen_range(0..300);
    let vc2: u64 = vc1 + network.rng.gen_range(0..300);
    let vc3: u64 = vc2 + network.rng.gen_range(0..300);
    let vc4: u64 = vc3 + network.rng.gen_range(0..300);

    'out: loop {
        if i > amount_of_requests * amount_of_clients * n * n * 100000 {
            println!("Too many interactions");
            println!("channels={:?}", network.channels);
            break;
            // panic!("Too many iterations");
        }
        i += 1;

        if i == vc1 {
            let replica_id = ReplicaId::from_u64(2);
            let ((recovering_replica, _output), _) =
                minimal_setup(n, t, replica_id, checkpoint_period, true);
            network.minbfts.remove(&replica_id);
            network.minbfts.insert(replica_id, recovering_replica);
            //handle_output(&mut network, replica_id, output);
        }

        if i == vc2 {
            let replica_id = ReplicaId::from_u64(2);
            let ((recovering_replica, output), _) =
                minimal_setup(n, t, replica_id, checkpoint_period, true);
            network.minbfts.remove(&replica_id);
            network.minbfts.insert(replica_id, recovering_replica);
            handle_output(&mut network, replica_id, output);
        }

        /*if i == vc3 {
            let replica_id = ReplicaId::from_u64(2);
            let ((recovering_replica, output), _) =
                minimal_setup(n, t, replica_id, checkpoint_period, true);
            network.minbfts.remove(&replica_id);
            network.minbfts.insert(replica_id, recovering_replica);
            //handle_output(&mut network, replica_id, output);
        }

        if i == vc4 {
            let replica_id = ReplicaId::from_u64(2);
            let ((recovering_replica, output), _) =
                minimal_setup(n, t, replica_id, checkpoint_period, true);
            network.minbfts.remove(&replica_id);
            network.minbfts.insert(replica_id, recovering_replica);
            handle_output(&mut network, replica_id, output);
        }*/
        /*if i == 30 {
            let replica_id = ReplicaId::from_u64(1);
            let ((recovering_replica, output), _) =
                minimal_setup(n, t, replica_id, checkpoint_period, true);
            network.minbfts.remove(&replica_id);
            network.minbfts.insert(replica_id, recovering_replica);
            handle_output(&mut network, replica_id, output);
        }*/

        if i == 200 {
            //network.current_time +=  Duration::from_secs(network.rng.gen_range(0..10));
            network.current_time += Duration::from_secs(10);
        }
        /*if i == 60 {
            //network.current_time +=  Duration::from_secs(network.rng.gen_range(0..10));
            network.current_time += Duration::from_secs(10);
        }*/
        if network.error {
            if network.crashed {
                break;
            } else {
                return;
            }
        }
        if !reiceive_random_channel(&mut network) {
            for replica_id in network.replicas.clone() {
                let th = network.timeout_handlers.get(&replica_id).unwrap();

                if th.has_pending_timeout() {
                    for entry in th.0.values() {
                        if entry.timeout_deadline < network.current_time {
                            panic! {"{:?} {:?}", network.current_time, th};
                        }
                    }

                    network.current_time += Duration::from_secs(100);
                    error!("th={:?}", th);
                    error!("time={:?}", network.current_time);
                    continue 'out;
                }
            }
            break;
        }
    }

    let mut replicas: Vec<ReplicaId> = Vec::new();

    for minbft in network.minbfts.values() {
        replicas.push(minbft.config.id);
    }
    replicas.sort();

    for replica_id in replicas.iter() {
        let minbft = network.minbfts.get(replica_id).unwrap();
        error!("{:?}: {:?}", replica_id.as_u64(), minbft.replicas_state);
    }

    for replica_id in replicas.iter() {
        let _ = network.minbfts.get(replica_id).unwrap();

        error!("{:?}", replica_id.as_u64());
    }

    for replica_id in replicas.iter() {
        let minbft = network.minbfts.get_mut(replica_id).unwrap();
        error!(
            "{:?} storage: {:?}",
            replica_id.as_u64(),
            minbft.replica_storage
        );
    }

    for replica_id in replicas.iter() {
        let minbft = network.minbfts.get_mut(replica_id).unwrap();
        error!(
            "{:?} view-state: {:?}",
            replica_id.as_u64(),
            minbft.view_state
        );
    }

    for replica_id in replicas.iter() {
        let minbft = network.minbfts.get(replica_id).unwrap();
        match &minbft.last_checkpoint_cert {
            Some(cert) => {
                error!(
                    "{:?}: {:?} {:?} {:?}",
                    replica_id.as_u64(),
                    cert.my_checkpoint.data.counter_latest_prep,
                    &cert.my_checkpoint.data.state_hash[0..5],
                    cert.my_checkpoint.total_amount_accepted_batches
                );
            }
            None => {
                error!("{:?}: None", replica_id.as_u64());
            }
        }
    }
    println!("recovery times: ->{:?} {:?}->{:?} {:?}", vc1, vc2, vc3, vc4);
    error!("current-time={:?}", network.current_time);
    for replica_id in replicas.iter() {
        let timeout_handler = network.timeout_handlers.get_mut(replica_id).unwrap();
        error!("timeout_handler={:?}", timeout_handler);
    }

    let mut correct = true;

    for replica_id in replicas.iter() {
        error!("requests {:?}:", replica_id.as_u64());

        for client_id in n..amount_of_clients + n {
            let channel = network.get_channel(*replica_id, ClientId::from_u64(client_id));
            let mut requests = Vec::new();
            for m in channel.message_queue.iter() {
                match m {
                    Message::Client(request_id, _) => requests.push(request_id.as_u64()),
                    _ => panic!("Not a client message"),
                }
            }
            for i in 0..amount_of_requests {
                if requests.contains(&i) {
                    print!("{a:>3}", a = i);
                } else {
                    print!("   ");
                    correct = false;
                }
            }
            println!();
        }
    }
    if network.crashed {
        error!("CRASHED BECAUSE OF ERROR");
    }

    if !correct {
        //panic!("Did not execute all client requests");
    }
    for minbft in minbfts.values() {
        if let Some(cp) = minbft.last_checkpoint_cert.as_ref() {
            println!("{:?}", cp.my_checkpoint);
        }
    }

    for (m1, m2) in minbfts.values().tuple_windows::<(_, _)>() {
        match (&m1.last_checkpoint_cert, &m2.last_checkpoint_cert) {
            (None, None) => (),
            (Some(c1), Some(c2)) => {
                assert!(c1.my_checkpoint.same_checkpoint(&c2.my_checkpoint));
                assert!(
                    c1.my_checkpoint.total_amount_accepted_batches
                        > amount_of_requests * amount_of_clients - checkpoint_period
                );
            }
            _ => panic!("replicas have different checkpoints"),
        }

        match (&m1.view_state, &m2.view_state) {
            (ViewState::InView(v1), ViewState::InView(v2)) => assert_eq!(v1.view, v2.view),
            (ViewState::ChangeInProgress(vc1), ViewState::ChangeInProgress(vc2)) => {
                assert_eq!(vc1.next_view, vc2.next_view);
                assert_eq!(vc1.next_view, vc2.next_view);
            }
            _ => panic!("replicas are in different view states"),
        }
    }
}

fn reiceive_random_channel(network: &mut Network) -> bool {
    for replica_id in network.replicas.clone().iter() {
        let timeout_types = network
            .timeout_handlers
            .get_mut(replica_id)
            .unwrap()
            .retrieve_due_timeouts(network.current_time);
        for timeout_type in timeout_types.iter() {
            let output = network
                .minbfts
                .get_mut(replica_id)
                .unwrap()
                .handle_timeout(*timeout_type);
            handle_output(network, *replica_id, output);
        }
    }

    let non_empty_channels: Vec<(Participant, Participant)> = network
        .channels
        .keys()
        .filter(|(sender, receiver)| {
            network
                .channels
                .get(&(*sender, *receiver))
                .unwrap()
                .has_message()
                && !matches!((sender, receiver), (_, Participant::Client(_)))
        })
        .map(|(sender, receiver)| (*sender, *receiver))
        .collect();

    if non_empty_channels.is_empty() {
        return false;
    }

    let i: usize = network.rng.gen_range(0..non_empty_channels.len());

    let (sender, receiver, msg) = {
        let channel = match network.channels.get_mut(&non_empty_channels[i]) {
            Some(channel) => channel,
            None => return false,
        };
        (channel.sender, channel.receiver, channel.receive().unwrap())
    };
    receive(network, sender, receiver, msg);
    network.tx_count += 1;
    true
}

fn handle_output(
    network: &mut Network,
    replica_id: ReplicaId,
    output: Output<DummyPayload, UsigNoOp>,
) {
    let Output {
        broadcasts,
        direct_messages,
        responses,
        timeout_requests,
        errors,
        ready_for_client_requests: _,
        primary: _,
        view_info: _,
        round: _,
        round_times: _,
    } = output;

    for error in Vec::from(errors) {
        error!("replica_id={:?} {:?}", replica_id, error);

        if let Error::StateTransferRequired { .. } = error {
            network.error = true;
        } else {
            // network.crashed = true;
            //network.error = true;
        }
    }
    if network.crashed {
        return;
    }

    for (client_id, resp) in responses.iter() {
        network
            .get_channel(
                Participant::Replica(replica_id),
                Participant::Client(*client_id),
            )
            .queue((resp.id(), DummyPayload));
        if let Some((request_id, replies)) = network.request_collector.get_mut(client_id) {
            if resp.id() != *request_id {
                continue;
            }
            if replies.contains(&replica_id) {
                panic!("Received duplicate response {:?}", resp);
            } else if replies.len() + 1 > network.t as usize {
                network.request_collector.insert(
                    *client_id,
                    (RequestId::from_u64(resp.id().as_u64() + 1), Vec::new()),
                );

                if let Some(requests) = network.pending_client_requests.get_mut(client_id) {
                    if let Some(req_id) = requests.pop_back() {
                        for rep_id in network.replicas.clone() {
                            network
                                .get_channel(
                                    Participant::Client(*client_id),
                                    Participant::Replica(rep_id),
                                )
                                .queue((req_id.into(), DummyPayload));
                        }
                    }
                };
            } else {
                replies.push(replica_id);
            }
        } else {
            network
                .request_collector
                .insert(*client_id, (resp.id(), vec![replica_id]));
        }
    }

    for msg in broadcasts.iter() {
        network.broadcast(replica_id, Message::Peer(msg.clone()));
    }
    for (receiver, msg) in direct_messages.iter() {
        assert_ne!(*receiver, replica_id);
        network
            .get_channel(replica_id, *receiver)
            .queue(msg.clone());
    }
    network
        .timeout_handlers
        .get_mut(&replica_id)
        .unwrap()
        .handle_timeout_requests(&timeout_requests, network.current_time);
}
