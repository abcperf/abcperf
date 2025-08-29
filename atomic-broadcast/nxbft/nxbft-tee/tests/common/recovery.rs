use nxbft::NxBft;
use shared_ids::ReplicaId;
use tracing::debug_span;

use super::checkers::*;
use super::setup::*;
use super::TestTx;

pub fn recovery(replicas: u64) {
    let mut rounds = 9;
    while rounds <= 49 {
        let _entered = debug_span!("test_recovery", replicas, rounds).entered();
        let mut recovering_nodes = Vec::new();
        let f = replicas - ((replicas / 2) + 1);
        let mut instances = instances(replicas);
        let mut stats = Vec::new();
        let mut delivery = Vec::new();
        for tx in 0u64..rounds {
            if tx % 6 == 5 {
                let id = (tx / 6) % replicas;
                let recover = &mut instances[id as usize];
                let id = ReplicaId::from_u64(id);
                recovering_nodes.push(id);
                let backup = recover.backup_enclave().unwrap();
                let (new, out) = NxBft::init_recovery(backup).unwrap();
                *recover = new;

                let (mut local_stats, mut local_delivery) =
                    deliver_from_output(id, out, &mut instances);
                stats.append(&mut local_stats);
                delivery.append(&mut local_delivery);
            }

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
                "\nRecovery: n: {}, f: {}, rounds: {}, recovery on: {:?}\n\n{}",
                replicas, f, rounds, recovering_nodes, fails
            );
        }
        rounds += 4;
    }
}
