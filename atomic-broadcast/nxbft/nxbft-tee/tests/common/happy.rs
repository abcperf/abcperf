use shared_ids::ReplicaId;

use super::checkers::*;
use super::setup::*;
use super::TestTx;

pub fn happy(replicas: u64) {
    let mut rounds = 9;
    while rounds <= 49 {
        let f = replicas - ((replicas / 2) + 1);
        let mut instances = instances(replicas);
        let mut stats = Vec::new();
        let mut delivery = Vec::new();
        for tx in 0u64..rounds {
            let mut outputs = Vec::new();
            for instance in instances.iter_mut() {
                outputs.push(instance.receive_client_request(TestTx(tx)));
            }
            for output in outputs.into_iter().enumerate() {
                let (mut local_stats, mut local_delivery) = deliver_from_output(
                    ReplicaId::from_u64(output.0 as u64),
                    output.1,
                    &mut instances,
                );
                stats.append(&mut local_stats);
                delivery.append(&mut local_delivery);
            }
        }

        if let Some(fails) = check(replicas, stats, delivery) {
            panic!(
                "\nHappy: n: {}, f: {}, rounds: {}\n\n{}",
                replicas, f, rounds, fails
            );
        }
        rounds += 4;
    }
}
