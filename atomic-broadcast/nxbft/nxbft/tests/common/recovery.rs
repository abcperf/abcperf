use std::collections::HashSet;

use nxbft::NxBft;
use rand::distributions::Bernoulli;
use rand::Rng;
use rand::SeedableRng;
use rand_chacha::ChaCha8Rng;
use rayon::prelude::*;
use shared_ids::ReplicaId;
use tracing::debug_span;

use super::checkers::*;
use super::setup::*;
use super::TestTx;

pub fn recovery() {
    let no_chance = Bernoulli::new(0.0).unwrap();
    let mut rounds = 9;
    while rounds <= 49 {
        (3u64..=35).into_par_iter().for_each(move |replicas| {
            let _entered = debug_span!("test_recovery", replicas, rounds).entered();
            let faulty_nodes = HashSet::new();
            let mut recovering_nodes = Vec::new();
            let mut rng = ChaCha8Rng::seed_from_u64(0u64);
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

                    let (mut local_stats, mut local_delivery, _) = deliver_from_output(
                        id,
                        out,
                        &mut instances,
                        &no_chance,
                        &faulty_nodes,
                        &no_chance,
                        &mut rng,
                    );
                    stats.append(&mut local_stats);
                    delivery.append(&mut local_delivery);
                }

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
                    "\nRecovery: n: {}, f: {}, rounds: {}, recovery on: {:?}\n\n{}",
                    replicas, f, rounds, recovering_nodes, fails
                );
            }
        });
        rounds += 4;
    }
}

pub fn recovery_fuzzing() {
    let rounds = 30;
    let repetitions = 15;
    let no_chance = Bernoulli::new(0.0).unwrap();
    let full_chance = Bernoulli::new(1.0).unwrap();
    let reorder_chances = [0.3, 0.6];
    let mut seed = 0u64;
    for replicas in 3u64..=25 {
        let f = 0;
        for reorder_chance in reorder_chances {
            #[allow(clippy::useless_conversion)]
                (0..repetitions).into_iter().for_each(move|rep| {
                    let _entered = debug_span!("test_recovery", replicas, rounds, rep, reorder_chance).entered();
                    let mut rng = ChaCha8Rng::seed_from_u64(seed + rep);
                    let mut faulty_nodes = HashSet::new();
                    while faulty_nodes.len() < f as usize {
                        faulty_nodes.insert(ReplicaId::from_u64(rng.gen_range(0..replicas)));
                    }
                    let reorder_bernoulli = Bernoulli::new(reorder_chance).unwrap();
                    let mut instances = instances(replicas);
                    let mut stats = Vec::new();
                    let mut delivery = Vec::new();
                    let mut held_back = Vec::new();
                    let mut recovering_nodes = Vec::new();
                    for tx in 0u64..rounds {
                        if tx % 6 == 5 {
                            let id = (tx / 6) % replicas;
                            let recover = &mut instances[id as usize];
                            let id = ReplicaId::from_u64(id);
                            recovering_nodes.push(id);
                            let backup = recover.backup_enclave().unwrap();
                            let (new, out) = NxBft::init_recovery(backup).unwrap();
                            *recover = new;

                            let (mut local_stats, mut local_delivery, _) = deliver_from_output(
                                id,
                                out,
                                &mut instances,
                                &no_chance,
                                &faulty_nodes,
                                &no_chance,
                                &mut rng,
                            );
                            stats.append(&mut local_stats);
                            delivery.append(&mut local_delivery);
                        }

                        let mut outputs = Vec::new();
                        for instance in instances.iter_mut() {
                            outputs.push(instance.receive_client_request(TestTx(tx)));
                        }
                        for output in outputs.into_iter().enumerate() {
                            let (mut local_stats, mut local_delivery, mut local_held_back) =
                                deliver_from_output(
                                    ReplicaId::from_u64(output.0 as u64),
                                    output.1,
                                    &mut instances,
                                    &reorder_bernoulli,
                                    &faulty_nodes,
                                    &no_chance,
                                    &mut rng,
                                );
                            stats.append(&mut local_stats);
                            delivery.append(&mut local_delivery);
                            held_back.append(&mut local_held_back);
                        }
                    }
                    loop {
                        if held_back.is_empty() {
                            break;
                        }
                        let (mut local_stats, mut local_delivery, local_held_back) =
                            deliver_from_unicasts(
                                held_back,
                                &mut instances,
                                &reorder_bernoulli,
                                &faulty_nodes,
                                &no_chance,
                                &mut rng,
                            );
                        stats.append(&mut local_stats);
                        delivery.append(&mut local_delivery);
                        held_back = local_held_back;
                    }

                    for tx in rounds..(2 * rounds) {
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
                                &full_chance,
                                &mut rng,
                            );
                            stats.append(&mut local_stats);
                            delivery.append(&mut local_delivery);
                        }
                    }

            //                     for recovered_replica in &recovering_nodes {
            //     let instance = &mut instances[recovered_replica.as_u64() as usize];
            //     let out = instance.process_vertex_timeout();

            //     let (mut local_stats, mut local_delivery, _) = deliver_from_output(
            //         *recovered_replica,
            //         out,
            //         &mut instances,
            //         &no_chance,
            //         &faulty_nodes,
            //         &no_chance,
            //         &mut rng,
            //     );
            //     stats.append(&mut local_stats);
            //     delivery.append(&mut local_delivery);
            // }

                    if let Some(fails) = check(replicas, &faulty_nodes, stats, delivery) {
                        panic!(
                            "\nFuzzing: n: {}, f: {} ({:?}), reorder: {}, repetition: {}, recovery on: {:?}\n\n{}",
                            replicas, f, faulty_nodes, reorder_chance, rep, recovering_nodes, fails
                        );
                    }
                });
            seed += repetitions;
        }
    }
}
