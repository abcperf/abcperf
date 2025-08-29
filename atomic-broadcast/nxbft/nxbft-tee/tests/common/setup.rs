use std::collections::HashSet;
use std::time::Duration;

use nxbft::output::StateMessage;
use nxbft::NxBft;
use rayon::prelude::*;
use shared_ids::ReplicaId;

use nxbft::output::Output;
use nxbft_tee::NxbftTEE;

use super::TestTx;

pub fn instances(n: u64) -> Vec<NxBft<TestTx, NxbftTEE>> {
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

    let output: Vec<_> = instances
        .par_iter_mut()
        .map(|i| i.init_handshake())
        .collect();
    let mut readys = HashSet::new();
    for output in output.into_iter().enumerate() {
        let (state, _) = deliver_from_output(
            ReplicaId::from_u64(output.0 as u64),
            output.1,
            &mut instances,
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
    output: Output<TestTx, NxbftTEE>,
    instances: &mut [NxBft<TestTx, NxbftTEE>],
) -> (
    Vec<(ReplicaId, StateMessage<NxbftTEE>)>,
    Vec<(ReplicaId, TestTx)>,
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
    for msg in broadcasts.iter() {
        for i in 0..instances.len() {
            let to = ReplicaId::from_u64(i as u64);
            let (mut their_state, mut their_delivery) = deliver_from_output(
                to,
                instances[to.as_u64() as usize].process_peer_message(msg.clone()),
                instances,
            );
            our_state.append(&mut their_state);
            our_delivery.append(&mut their_delivery);
        }
    }
    for (to, msg) in unicasts.into_iter() {
        let (mut their_state, mut their_delivery) = deliver_from_output(
            to,
            instances[to.as_u64() as usize].process_peer_message(msg),
            instances,
        );
        our_state.append(&mut their_state);
        our_delivery.append(&mut their_delivery);
    }
    (our_state, our_delivery)
}
