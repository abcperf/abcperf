use std::collections::HashSet;

use rand::distributions::Bernoulli;
use rand::SeedableRng;
use rand_chacha::ChaCha8Rng;
use rayon::prelude::*;
use shared_ids::map::ReplicaMap;
use shared_ids::ReplicaId;

use super::checker::check;
use super::setup::*;
use super::TestTx;

pub fn happy() {
    let no_chance = Bernoulli::new(0.0).unwrap();
    let mut rounds = 9;
    while rounds <= 49 {
        (3u64..=35).into_par_iter().for_each(move |replicas| {
            let faulty_nodes = HashSet::new();
            let mut rng = ChaCha8Rng::seed_from_u64(0u64);
            let f = replicas - ((replicas / 2) + 1);
            let mut instances = instances(replicas);
            let mut delivery = Vec::new();
            let mut view = Vec::new();
            let mut max_tx = 0u64;
            for tx in 0u64..rounds {
                let mut outputs = Vec::new();
                for instance in instances.iter_mut() {
                    outputs.push(instance.receive_client_request(TestTx(tx)));
                }
                for output in outputs.into_iter().enumerate() {
                    let (_, mut local_delivery, _, mut local_view) = deliver_from_output(
                        ReplicaId::from_u64(output.0 as u64),
                        output.1,
                        &mut instances,
                        &no_chance,
                        &faulty_nodes,
                        &no_chance,
                        &mut rng,
                    );
                    delivery.append(&mut local_delivery);
                    view.append(&mut local_view);
                }
                max_tx = tx;
            }

            for _ in 0u64..25 {
                let mut outputs = Vec::new();
                for instance in instances.iter_mut() {
                    outputs.push(instance.handle_block_timeout());
                }
                for output in outputs.into_iter().enumerate() {
                    let (_, mut local_delivery, _, mut local_view) = deliver_from_output(
                        ReplicaId::from_u64(output.0 as u64),
                        output.1,
                        &mut instances,
                        &no_chance,
                        &faulty_nodes,
                        &no_chance,
                        &mut rng,
                    );
                    delivery.append(&mut local_delivery);
                    view.append(&mut local_view);
                }
            }

            let mut blocks_by_replica = ReplicaMap::new(replicas);

            for (i, instance) in instances.iter().enumerate() {
                blocks_by_replica[ReplicaId::from(i as u64)] = instance.get_executed_blocks();
            }

            if let Some(fails) = check(replicas, &faulty_nodes, blocks_by_replica, delivery, max_tx)
            {
                panic!(
                    "\nHappy: n: {}, f: {}, rounds: {}\n\n{}",
                    replicas, f, rounds, fails
                );
            }
        });
        rounds += 4;
    }
}
