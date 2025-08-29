use std::collections::HashSet;

use rand::{distributions::Bernoulli, prelude::Distribution, Rng, SeedableRng};
use rand_chacha::ChaCha8Rng;
use rayon::prelude::*;
use shared_ids::{map::ReplicaMap, ReplicaId};

use super::{
    checker::check,
    setup::{deliver_from_output, deliver_from_unicasts, instances},
    TestTx,
};

pub fn fuzzing_no_drop() {
    let rounds = 21;
    let repetitions = 15;
    let no_chance = Bernoulli::new(0.0).unwrap();
    let reorder_chances = [0.0, 0.3, 0.6, 0.9];
    let mut seed = 0u64;
    for replicas in 3u64..=30 {
        let f = replicas - ((replicas / 2) + 1);
        for reorder_chance in reorder_chances {
            (0..repetitions).into_par_iter().for_each(move |rep| {
                let mut rng = ChaCha8Rng::seed_from_u64(seed + rep);
                let faulty_nodes = HashSet::new();
                let reorder_bernoulli = Bernoulli::new(reorder_chance).unwrap();
                let mut instances = instances(replicas);
                let mut delivery = Vec::new();
                let mut view = Vec::new();
                let mut held_back = Vec::new();
                let mut max_tx = 0u64;
                for tx in 0u64..rounds {
                    let mut outputs = Vec::new();
                    for instance in instances.iter_mut() {
                        outputs.push(instance.receive_client_request(TestTx(tx)));
                    }
                    for output in outputs.into_iter().enumerate() {
                        let (_, mut local_delivery, mut local_held_back, mut local_view) =
                            deliver_from_output(
                                ReplicaId::from_u64(output.0 as u64),
                                output.1,
                                &mut instances,
                                &reorder_bernoulli,
                                &faulty_nodes,
                                &no_chance,
                                &mut rng,
                            );
                        delivery.append(&mut local_delivery);
                        held_back.append(&mut local_held_back);
                        view.append(&mut local_view);
                    }
                    max_tx = tx;
                }
                loop {
                    if held_back.is_empty() {
                        break;
                    }
                    let (mut local_delivery, local_held_back, mut local_view) =
                        deliver_from_unicasts(
                            held_back,
                            &mut instances,
                            &reorder_bernoulli,
                            &faulty_nodes,
                            &no_chance,
                            &mut rng,
                        );
                    delivery.append(&mut local_delivery);
                    held_back = local_held_back;
                    view.append(&mut local_view);
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

                if let Some(fails) =
                    check(replicas, &faulty_nodes, blocks_by_replica, delivery, max_tx)
                {
                    panic!(
                        "\nFuzzing: n: {}, f: {} ({:?}), reorder: {}, repetition: {}\n\n{}",
                        replicas, f, faulty_nodes, reorder_chance, rep, fails
                    );
                }
            });
            seed += repetitions;
        }
    }
}

pub fn fuzzing_with_drop() {
    let rounds = 9;
    let repetitions = 8;
    let no_chance = Bernoulli::new(0.0).unwrap();
    let drop_chances = [0.0, 0.33, 0.66, 1.0];
    let reorder_chances = [0.0, 0.3, 0.6];
    let timeout_chances = [0.25, 0.5];
    let mut seed = 0u64;
    for replicas in 3u64..=30 {
        let f = replicas - ((replicas / 2) + 1);
        for reorder_chance in reorder_chances {
            for drop_chance in drop_chances {
                for timeout_chance in timeout_chances {
                    (0..repetitions).into_par_iter().for_each(move|rep| {
                    let mut rng = ChaCha8Rng::seed_from_u64(seed + rep);
                    let mut faulty_nodes = HashSet::new();
                    while faulty_nodes.len() < f as usize {
                        faulty_nodes.insert(ReplicaId::from_u64(rng.gen_range(0..replicas)));
                    }
                    let reorder_bernoulli = Bernoulli::new(reorder_chance).unwrap();
                    let drop_bernoulli = Bernoulli::new(drop_chance).unwrap();
                    let timeout_bernoulli = Bernoulli::new(timeout_chance).unwrap();
                    let mut instances = instances(replicas);
                    let mut delivery = Vec::new();
                    let mut view = Vec::new();
                    let mut held_back = Vec::new();
                    let mut max_tx = 0u64;
                    for tx in 0u64..rounds {
                        let mut outputs = Vec::new();
                        for instance in instances.iter_mut() {
                            outputs.push(instance.receive_client_request(TestTx(tx)));
                        }
                        for output in outputs.into_iter().enumerate() {
                            let (_, mut local_delivery, mut local_held_back, mut local_view) =
                                deliver_from_output(
                                    ReplicaId::from_u64(output.0 as u64),
                                    output.1,
                                    &mut instances,
                                    &reorder_bernoulli,
                                    &faulty_nodes,
                                    &drop_bernoulli,
                                    &mut rng,
                                );
                            delivery.append(&mut local_delivery);
                            held_back.append(&mut local_held_back);
                            view.append(&mut local_view);
                        }

                        for i in 0..replicas {
                            if !timeout_bernoulli.sample(&mut rng) {
                                continue;
                            }
                                let (_, mut local_delivery, mut local_held_back, mut local_view) =
                                deliver_from_output(
                                    ReplicaId::from_u64(i),
                                    instances[i as usize].handle_view_timeout(),
                                    &mut instances,
                                    &reorder_bernoulli,
                                    &faulty_nodes,
                                    &drop_bernoulli,
                                    &mut rng,
                                );
                            delivery.append(&mut local_delivery);
                            held_back.append(&mut local_held_back);
                            view.append(&mut local_view);
                        }
                        max_tx = tx;
                    }
                    loop {
                        if held_back.is_empty() {
                            break;
                        }
                        let (mut local_delivery, local_held_back, mut local_view) =
                            deliver_from_unicasts(
                                held_back,
                                &mut instances,
                                &reorder_bernoulli,
                                &faulty_nodes,
                                &drop_bernoulli,
                                &mut rng,
                            );
                        delivery.append(&mut local_delivery);
                        held_back = local_held_back;
                        view.append(&mut local_view);
                    }

                    for _ in 0u64..100 {
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

                        for i in 0..replicas {
                            if !timeout_bernoulli.sample(&mut rng) {
                                continue;
                            }
                                let (_, mut local_delivery, _, mut local_view) =
                                deliver_from_output(
                                    ReplicaId::from_u64(i),
                                    instances[i as usize].handle_view_timeout(),
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

                    if let Some(fails) = check(replicas, &faulty_nodes, blocks_by_replica, delivery, max_tx) {
                        panic!(
                            "\nFuzzing With Drop: n: {}, f: {} ({:?}), drops: {}, reorder: {}, timeout: {}, repetition: {}\n\n{}",
                            replicas, f, faulty_nodes, drop_chance, reorder_chance, timeout_chance, rep, fails
                        );
                    }
                });
                    seed += repetitions;
                }
            }
        }
    }
}
