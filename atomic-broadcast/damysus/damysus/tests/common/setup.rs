use std::collections::HashSet;
use std::time::Duration;

use damysus::enclave::NoopEnclave;
use damysus::{Damysus, Output, PeerMessage};
use rand::distributions::Bernoulli;
use rand::distributions::Distribution;
use rand::SeedableRng;
use rand_chacha::ChaCha8Rng;
use shared_ids::ReplicaId;

use super::TestTx;

pub fn instances(n: u64) -> Vec<Damysus<TestTx, NoopEnclave>> {
    let mut instances = Vec::with_capacity(n as usize);
    for i in 0u64..n {
        instances.push(Damysus::new(
            ReplicaId::from_u64(i),
            n,
            1,
            5,
            Duration::from_secs(1000),
            Duration::from_secs(1),
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
        let (their_readys, _, _, _) = deliver_from_output(
            ReplicaId::from_u64(output.0 as u64),
            output.1,
            &mut instances,
            &no_chance,
            &HashSet::new(),
            &no_chance,
            &mut rng,
        );
        readys.extend(their_readys);
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
    output: Output<TestTx, NoopEnclave>,
    instances: &mut [Damysus<TestTx, NoopEnclave>],
    hold_back_chance: &Bernoulli,
    faulty_nodes: &HashSet<ReplicaId>,
    omission_chance: &Bernoulli,
    rng: &mut ChaCha8Rng,
) -> (
    HashSet<ReplicaId>,
    Vec<(ReplicaId, TestTx)>,
    Vec<(ReplicaId, PeerMessage<TestTx, [u8; 8], [u8; 8]>)>,
    Vec<(ReplicaId, u64)>,
) {
    let Output {
        broadcasts,
        unicasts,
        ready,
        delivery,
        view,
        ..
    } = output;
    let mut our_readys = HashSet::new();
    if ready {
        our_readys.insert(from);
    }
    let mut our_delivery = Vec::new();
    for tx in delivery.into_iter() {
        our_delivery.push((from, tx));
    }
    let mut our_view = Vec::new();
    our_view.push((from, view));

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
        let (their_readys, mut their_delivery, mut their_held_back, mut their_view) =
            deliver_from_output(
                to,
                instances[to.as_u64() as usize].process_peer_message(msg),
                instances,
                hold_back_chance,
                faulty_nodes,
                omission_chance,
                rng,
            );
        our_readys.extend(their_readys);
        held_back.append(&mut their_held_back);
        our_delivery.append(&mut their_delivery);
        our_view.append(&mut their_view);
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
            let (their_readys, mut their_delivery, mut their_held_back, mut their_view) =
                deliver_from_output(
                    to,
                    instances[to.as_u64() as usize].process_peer_message(msg.clone()),
                    instances,
                    hold_back_chance,
                    faulty_nodes,
                    omission_chance,
                    rng,
                );
            our_readys.extend(their_readys);
            held_back.append(&mut their_held_back);
            our_delivery.append(&mut their_delivery);
            our_view.append(&mut their_view);
        }
    }
    (our_readys, our_delivery, held_back, our_view)
}

#[allow(clippy::type_complexity)]
pub fn deliver_from_unicasts(
    unicasts: Vec<(ReplicaId, PeerMessage<TestTx, [u8; 8], [u8; 8]>)>,
    instances: &mut [Damysus<TestTx, NoopEnclave>],
    hold_back_chance: &Bernoulli,
    faulty_nodes: &HashSet<ReplicaId>,
    omission_chance: &Bernoulli,
    rng: &mut ChaCha8Rng,
) -> (
    Vec<(ReplicaId, TestTx)>,
    Vec<(ReplicaId, PeerMessage<TestTx, [u8; 8], [u8; 8]>)>,
    Vec<(ReplicaId, u64)>,
) {
    let mut held_back = Vec::new();
    let mut our_delivery = Vec::new();
    let mut our_view = Vec::new();
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
        let (_, mut their_delivery, mut their_held_back, mut their_view) = deliver_from_output(
            to,
            instances[to.as_u64() as usize].process_peer_message(msg),
            instances,
            hold_back_chance,
            faulty_nodes,
            omission_chance,
            rng,
        );
        our_delivery.append(&mut their_delivery);
        held_back.append(&mut their_held_back);
        our_view.append(&mut their_view);
    }
    (our_delivery, held_back, our_view)
}
