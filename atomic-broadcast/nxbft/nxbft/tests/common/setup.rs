use std::collections::HashSet;
use std::time::Duration;

use nxbft::output::StateMessage;
use nxbft::NxBft;
use nxbft_base::enclave::simple::{EnclaveId, EnclaveSignature, SimpleEnclave};
use rand::distributions::Bernoulli;
use rand::SeedableRng;
use rand_chacha::ChaCha8Rng;
use shared_ids::ReplicaId;

use nxbft::output::Output;
use rand::prelude::Distribution;

use nxbft::PeerMessage;

use super::TestTx;

pub fn instances(n: u64) -> Vec<NxBft<TestTx, SimpleEnclave>> {
    let mut instances = Vec::with_capacity(n as usize);
    for i in 0u64..n {
        instances.push(NxBft::new(
            ReplicaId::from_u64(i),
            n,
            Duration::from_secs(1000),
            1,
            1,
        ))
    }
    let mut output = Vec::new();
    for inst in instances.iter_mut() {
        output.push(inst.init_handshake());
    }
    let mut readys = HashSet::new();
    let no_chance = Bernoulli::new(0.0).unwrap();
    let mut rng = ChaCha8Rng::from_entropy();
    for output in output.into_iter().enumerate() {
        let (state, _, _) = deliver_from_output(
            ReplicaId::from_u64(output.0 as u64),
            output.1,
            &mut instances,
            &no_chance,
            &HashSet::new(),
            &no_chance,
            &mut rng,
        );

        for s in state.into_iter() {
            // println!("{:?}", s);
            if matches!(s.1, StateMessage::Ready) {
                readys.insert(s.0);
            }
        }
    }
    for i in 0..n {
        assert!(readys.contains(&ReplicaId::from_u64(i)));
    }
    assert_eq!(readys.len(), n as usize);
    instances
}

#[allow(clippy::type_complexity)]
pub fn deliver_from_output(
    from: ReplicaId,
    output: Output<TestTx, SimpleEnclave>,
    instances: &mut [NxBft<TestTx, SimpleEnclave>],
    hold_back_chance: &Bernoulli,
    faulty_nodes: &HashSet<ReplicaId>,
    omission_chance: &Bernoulli,
    rng: &mut ChaCha8Rng,
) -> (
    Vec<(ReplicaId, StateMessage<SimpleEnclave>)>,
    Vec<(ReplicaId, TestTx)>,
    Vec<(
        ReplicaId,
        PeerMessage<TestTx, EnclaveId, [u8; 32], EnclaveSignature>,
    )>,
) {
    let Output {
        broadcasts,
        unicasts,
        state_messages,
        delivery,
        ..
    } = output;
    let mut our_state = Vec::new();
    for state in state_messages.into_iter() {
        our_state.push((from, state));
    }
    let mut our_delivery = Vec::new();
    for tx in delivery.into_iter() {
        our_delivery.push((from, tx));
    }
    let mut held_back = Vec::new();
    for (to, msg) in unicasts.into_iter() {
        if hold_back_chance.sample(rng) {
            held_back.push((to, msg));
            continue;
        }
        if (faulty_nodes.contains(&msg.from) || faulty_nodes.contains(&to))
            && omission_chance.sample(rng)
        {
            continue;
        }
        let (mut their_state, mut their_delivery, mut their_held_back) = deliver_from_output(
            to,
            instances[to.as_u64() as usize].process_peer_message(msg),
            instances,
            hold_back_chance,
            faulty_nodes,
            omission_chance,
            rng,
        );
        our_state.append(&mut their_state);
        held_back.append(&mut their_held_back);
        our_delivery.append(&mut their_delivery);
    }

    for msg in broadcasts.iter() {
        for i in 0..instances.len() {
            let to = ReplicaId::from_u64(i as u64);
            if hold_back_chance.sample(rng) {
                held_back.push((to, msg.clone()));
                continue;
            }
            if (faulty_nodes.contains(&msg.from) || faulty_nodes.contains(&to))
                && omission_chance.sample(rng)
            {
                continue;
            }
            let (mut their_state, mut their_delivery, mut their_held_back) = deliver_from_output(
                to,
                instances[to.as_u64() as usize].process_peer_message(msg.clone()),
                instances,
                hold_back_chance,
                faulty_nodes,
                omission_chance,
                rng,
            );
            our_state.append(&mut their_state);
            held_back.append(&mut their_held_back);
            our_delivery.append(&mut their_delivery);
        }
    }
    (our_state, our_delivery, held_back)
}

#[allow(clippy::type_complexity)]
pub fn deliver_from_unicasts(
    unicasts: Vec<(
        ReplicaId,
        PeerMessage<TestTx, EnclaveId, [u8; 32], EnclaveSignature>,
    )>,
    instances: &mut [NxBft<TestTx, SimpleEnclave>],
    hold_back_chance: &Bernoulli,
    faulty_nodes: &HashSet<ReplicaId>,
    omission_chance: &Bernoulli,
    rng: &mut ChaCha8Rng,
) -> (
    Vec<(ReplicaId, StateMessage<SimpleEnclave>)>,
    Vec<(ReplicaId, TestTx)>,
    Vec<(
        ReplicaId,
        PeerMessage<TestTx, EnclaveId, [u8; 32], EnclaveSignature>,
    )>,
) {
    let mut our_state = Vec::new();
    let mut held_back = Vec::new();
    let mut our_delivery = Vec::new();
    for (to, msg) in unicasts.into_iter() {
        if hold_back_chance.sample(rng) {
            // println!("Delaying {}", msg);
            held_back.push((to, msg));
            continue;
        }
        if (faulty_nodes.contains(&msg.from) || faulty_nodes.contains(&to))
            && omission_chance.sample(rng)
        {
            // println!("DROP {}", msg);
            continue;
        }
        let (mut their_state, mut their_delivery, mut their_held_back) = deliver_from_output(
            to,
            instances[to.as_u64() as usize].process_peer_message(msg),
            instances,
            hold_back_chance,
            faulty_nodes,
            omission_chance,
            rng,
        );
        our_state.append(&mut their_state);
        our_delivery.append(&mut their_delivery);
        held_back.append(&mut their_held_back);
    }
    (our_state, our_delivery, held_back)
}
