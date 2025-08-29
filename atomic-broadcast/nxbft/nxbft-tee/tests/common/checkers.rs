use std::collections::HashMap;
use std::collections::HashSet;

use nxbft::output::Consensus;
use nxbft::output::StateMessage;
use nxbft::output::Verbose;
use nxbft::Wave;
use nxbft_tee::NxbftTEE;
use shared_ids::ReplicaId;

use super::TestTx;

fn split_filter_stats(
    n: u64,
    stats: Vec<(ReplicaId, StateMessage<NxbftTEE>)>,
) -> Vec<Vec<StateMessage<NxbftTEE>>> {
    let mut stats_by_replica: Vec<Vec<StateMessage<NxbftTEE>>> =
        (0..n).map(|_| Vec::new()).collect();

    for msg in stats.into_iter() {
        stats_by_replica[msg.0.as_u64() as usize].push(msg.1);
    }
    stats_by_replica
}

fn check_errors(n: u64, stats: &[Vec<StateMessage<NxbftTEE>>]) -> Option<String> {
    let mut result_string = String::new();
    for i in 0u64..n {
        for state_msg in stats[i as usize].iter() {
            if let StateMessage::Error(err) = state_msg {
                result_string = format!("{}\n\tchecking: {}, err: {:?}", result_string, i, err);
            }
        }
    }
    if result_string.is_empty() {
        None
    } else {
        Some(format!("Error !!{}", result_string))
    }
}

fn check_peer_errors(n: u64, stats: &[Vec<StateMessage<NxbftTEE>>]) -> Option<String> {
    let mut result_string = String::new();
    for i in 0u64..n {
        for state_msg in stats[i as usize].iter() {
            if let StateMessage::PeerError(err) = state_msg {
                result_string = format!("{}\n\tchecking: {}, err: {:?}", result_string, i, err);
            }
        }
    }
    if result_string.is_empty() {
        None
    } else {
        Some(format!("Peer Error !!{}", result_string))
    }
}

fn check_non_equivocation_broadcast(
    n: u64,
    stats: &[Vec<StateMessage<NxbftTEE>>],
) -> Option<String> {
    let mut vertices_total = HashSet::new();
    let mut vertices_by_replica = Vec::new();
    for i in 0u64..n {
        let mut my_vertices = HashSet::new();
        for payload in stats[i as usize].iter().filter_map(|e| {
            if let StateMessage::Verbose(Verbose::NonEquivocationBroadcastDeliver {
                payload, ..
            }) = e
            {
                Some(payload)
            } else {
                None
            }
        }) {
            vertices_total.insert(payload.clone());
            my_vertices.insert(payload.clone());
        }
        vertices_by_replica.push(my_vertices);
    }

    let mut result_string = String::new();
    for i in 0u64..n {
        if vertices_by_replica[i as usize].len() != vertices_total.len() {
            let mut missing = HashSet::new();
            for vertex in vertices_total.iter() {
                if !vertices_by_replica[i as usize].contains(vertex) {
                    missing.insert(vertex.clone());
                }
            }
            let mut missing: Vec<String> = missing.into_iter().collect();
            missing.sort();
            result_string = format!(
                "{}\n\tchecking: {}, missing: {:?}",
                result_string, i, missing
            );
        }
    }
    if result_string.is_empty() {
        None
    } else {
        let mut vertices_total: Vec<String> = vertices_total.into_iter().collect();
        vertices_total.sort();
        Some(format!("{}\n\ttotal: {:?}", result_string, vertices_total))
    }
}

fn check_buffer(n: u64, stats: &[Vec<StateMessage<NxbftTEE>>]) -> Option<String> {
    let mut vertices_total = HashSet::new();
    let mut vertices_by_replica = Vec::new();
    for i in 0u64..n {
        let mut my_vertices = HashSet::new();
        for vertex in stats[i as usize].iter().filter_map(|e| {
            if let StateMessage::Verbose(Verbose::BufferedVertex { vertex }) = e {
                Some(vertex)
            } else {
                None
            }
        }) {
            vertices_total.insert(*vertex);
            my_vertices.insert(*vertex);
        }
        vertices_by_replica.push(my_vertices);
    }

    let mut result_string = String::new();
    for i in 0u64..n {
        if vertices_by_replica[i as usize].len() != vertices_total.len() {
            let mut missing = HashSet::new();
            for vertex in vertices_total.iter() {
                if !vertices_by_replica[i as usize].contains(vertex) {
                    missing.insert(*vertex);
                }
            }
            let mut missing: Vec<_> = missing.into_iter().collect();
            missing.sort();
            result_string = format!(
                "{}\n\tchecking: {}, missing: {:?}",
                result_string, i, missing
            );
        }
    }
    if result_string.is_empty() {
        None
    } else {
        let mut vertices_total: Vec<_> = vertices_total.into_iter().collect();
        vertices_total.sort();
        Some(format!("{}\n\ttotal: {:?}", result_string, vertices_total))
    }
}

fn check_graph(n: u64, stats: &[Vec<StateMessage<NxbftTEE>>]) -> Option<String> {
    let mut vertices_total = HashSet::new();
    let mut vertices_by_replica = Vec::new();
    for i in 0u64..n {
        let mut my_vertices = HashSet::new();
        for vertex in stats[i as usize].iter().filter_map(|e| {
            if let StateMessage::Verbose(Verbose::VertexAdded { vertex }) = e {
                Some(vertex)
            } else {
                None
            }
        }) {
            vertices_total.insert(*vertex);
            my_vertices.insert(*vertex);
        }
        vertices_by_replica.push(my_vertices);
    }

    let mut result_string = String::new();
    for i in 0u64..n {
        if vertices_by_replica[i as usize].len() != vertices_total.len() {
            let mut missing = HashSet::new();
            for vertex in vertices_total.iter() {
                if !vertices_by_replica[i as usize].contains(vertex) {
                    missing.insert(*vertex);
                }
            }
            let mut missing: Vec<_> = missing.into_iter().collect();
            missing.sort();

            result_string = format!(
                "{}\n\tchecking: {}, missing: {:?}",
                result_string, i, missing
            );
        }
    }
    if result_string.is_empty() {
        None
    } else {
        let mut vertices_total: Vec<_> = vertices_total.into_iter().collect();
        vertices_total.sort();
        Some(format!("{}\n\ttotal: {:?}", result_string, vertices_total))
    }
}

fn check_round_transition(n: u64, stats: &[Vec<StateMessage<NxbftTEE>>]) -> Option<String> {
    let mut rounds_total = HashSet::new();
    let mut rounds_by_replica = Vec::new();
    for i in 0u64..n {
        let mut my_rounds = HashSet::new();
        for round in stats[i as usize].iter().filter_map(|e| {
            if let StateMessage::Consensus(Consensus::RoundTransition { new_round }) = e {
                Some(new_round)
            } else {
                None
            }
        }) {
            rounds_total.insert(*round);
            my_rounds.insert(*round);
        }
        rounds_by_replica.push(my_rounds);
    }

    let mut result_string = String::new();
    for i in 0u64..n {
        if rounds_by_replica[i as usize].len() != rounds_total.len() {
            let mut missing = HashSet::new();
            for round in rounds_total.iter() {
                if !rounds_by_replica[i as usize].contains(round) {
                    missing.insert(*round);
                }
            }
            let mut missing: Vec<_> = missing.into_iter().collect();
            missing.sort();

            result_string = format!(
                "{}\n\tchecking: {}, missing: {:?}",
                result_string, i, missing
            );
        }
    }
    if result_string.is_empty() {
        None
    } else {
        let mut rounds_total: Vec<_> = rounds_total.into_iter().collect();
        rounds_total.sort();
        Some(format!("{}\n\ttotal: {:?}", result_string, rounds_total))
    }
}

fn check_consensus(n: u64, stats: &[Vec<StateMessage<NxbftTEE>>]) -> Option<String> {
    let mut waves_total = Vec::new();
    let mut waves_by_replica = Vec::new();
    for i in 0u64..n {
        let mut my_waves = Vec::new();
        for waves in stats[i as usize].iter().filter_map(|e| {
            if let StateMessage::Consensus(Consensus::Decided { waves, .. }) = e {
                Some(waves)
            } else {
                None
            }
        }) {
            waves_total.extend_from_slice(waves);
            my_waves.extend_from_slice(waves);
        }
        waves_by_replica.push(my_waves);
    }

    let waves_total: HashSet<Wave> = HashSet::from_iter(waves_total);

    let mut result_string = String::new();
    let mut waves_total: Vec<_> = waves_total.into_iter().collect();
    waves_total.sort();
    let mut correct = 0;
    for i in 0u64..n {
        correct = i;
        if waves_by_replica[i as usize].len() != waves_total.len() {
            let mut missing = HashSet::new();
            for wave in waves_total.iter() {
                if !waves_by_replica[i as usize].contains(wave) {
                    missing.insert(*wave);
                }
            }
            let mut missing: Vec<_> = missing.into_iter().collect();
            missing.sort();
            result_string = format!(
                "{}\n\t-- Waves !! checking: {}, missing: {:?}, total: {:?}",
                result_string, i, missing, waves_total
            );
        }
    }

    let mut correct_order = Vec::new();
    for mut order in stats[correct as usize].iter().filter_map(|e| {
        if let StateMessage::Consensus(Consensus::Decided { order, .. }) = e {
            let vec: Vec<_> = order.iter().collect();
            Some(vec)
        } else {
            None
        }
    }) {
        correct_order.append(&mut order);
    }

    for i in 0u64..n {
        let mut my_order = Vec::new();
        for mut order in stats[i as usize].iter().filter_map(|e| {
            if let StateMessage::Consensus(Consensus::Decided { order, .. }) = e {
                let vec: Vec<_> = order.iter().collect();
                Some(vec)
            } else {
                None
            }
        }) {
            my_order.append(&mut order);
        }

        if my_order.len() != correct_order.len() {
            result_string = format!(
                "{}\n\t-- Order Length !! checking: {} (len: {}), correct: {} (len: {})",
                result_string,
                i,
                my_order.len(),
                correct,
                correct_order.len()
            );
            continue;
        }

        for j in 0..correct_order.len() {
            if my_order[j] != correct_order[j] {
                result_string = format!(
                    "{}\n\t-- Order Difers !! pos: {} of {}, checking: {} (is: {:?}), correct: {} (is: {:?})",
                    result_string, j, correct_order.len(), i, my_order[j], correct, correct_order[j]
                );
            }
        }
    }

    if result_string.is_empty() {
        None
    } else {
        Some(result_string)
    }
}

fn check_deduplication(n: u64, delivery: Vec<(ReplicaId, TestTx)>) -> Option<String> {
    let mut d_counter = HashMap::with_capacity(n as usize);
    for tx in delivery.into_iter() {
        let counter = if let Some(i_counter) = d_counter.get_mut(&tx.0) {
            i_counter
        } else {
            d_counter.insert(tx.0, HashMap::new());
            d_counter.get_mut(&tx.0).unwrap()
        };

        if let Some(value) = counter.get(&tx.1 .0) {
            counter.insert(tx.1 .0, value + 1);
        } else {
            counter.insert(tx.1 .0, 1u64);
        }
    }

    let mut all_txs = HashSet::new();
    for replica_count in d_counter.iter() {
        for tx_count in replica_count.1.iter() {
            all_txs.insert(*tx_count.0);
        }
    }

    let mut result_string = String::new();
    for replica_count in d_counter.into_iter() {
        for tx in all_txs.iter() {
            let tx_count = if let Some(count) = replica_count.1.get(tx) {
                *count
            } else {
                0u64
            };
            if tx_count != 1 {
                result_string = format!(
                    "{}, ({}, {}, {})",
                    result_string,
                    replica_count.0.as_u64(),
                    tx,
                    tx_count
                );
            }
        }
    }

    if result_string.is_empty() {
        None
    } else {
        Some(result_string)
    }
}

pub fn check(
    n: u64,
    stats: Vec<(ReplicaId, StateMessage<NxbftTEE>)>,
    delivery: Vec<(ReplicaId, TestTx)>,
) -> Option<String> {
    let stats_by_replica = split_filter_stats(n, stats);

    if let Some(e) = check_errors(n, &stats_by_replica) {
        return Some(format!("Errors !! {}", e));
    }
    if let Some(e) = check_peer_errors(n, &stats_by_replica) {
        return Some(format!("Peer Errors !! {}", e));
    }

    if let Some(e) = check_non_equivocation_broadcast(n, &stats_by_replica) {
        return Some(format!("Non Equivocation Broadcast !! {}", e));
    }

    if let Some(e) = check_buffer(n, &stats_by_replica) {
        return Some(format!("Buffer !! {}", e));
    }

    if let Some(e) = check_graph(n, &stats_by_replica) {
        return Some(format!("Graph !! {}", e));
    }

    if let Some(e) = check_round_transition(n, &stats_by_replica) {
        return Some(format!("Round Transition !! {}", e));
    }
    if let Some(e) = check_consensus(n, &stats_by_replica) {
        return Some(format!("Consensus !! {}", e));
    }

    if let Some(deduplication) = check_deduplication(n, delivery) {
        return Some(format!("Duplicate deliveries !! {}", deduplication));
    }
    None
}
