use std::collections::HashSet;

use rand::distributions::Bernoulli;
use rand::SeedableRng;
use rand_chacha::ChaCha8Rng;
use rayon::prelude::*;
use shared_ids::ReplicaId;

use super::checkers::*;
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
            let mut stats = Vec::new();
            let mut delivery = Vec::new();
            for tx in 0u64..rounds {
                let mut outputs = Vec::new();
                for instance in instances.iter_mut() {
                    outputs.push(instance.receive_client_request(TestTx(tx)));
                }
                for output in outputs.into_iter().enumerate() {
                    let (mut local_stats, mut local_delivery, _) = deliver_from_output(
                        ReplicaId::from_u64(output.0 as u64),
                        output.1,
                        &mut instances,
                        &no_chance,
                        &faulty_nodes,
                        &no_chance,
                        &mut rng,
                    );
                    stats.append(&mut local_stats);
                    delivery.append(&mut local_delivery);
                }
            }

            if let Some(fails) = check(replicas, &faulty_nodes, stats, delivery) {
                panic!(
                    "\nHappy: n: {}, f: {}, rounds: {}\n\n{}",
                    replicas, f, rounds, fails
                );
            }
        });
        rounds += 4;
    }
}
