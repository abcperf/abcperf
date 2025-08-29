use std::collections::{HashMap, HashSet};

use shared_ids::{map::ReplicaMap, ReplicaId};

use super::TestTx;

fn check_blocks(
    n: u64,
    faultys: &HashSet<ReplicaId>,
    blocks_by_replica: ReplicaMap<Vec<Option<u64>>>,
) -> Option<String> {
    let mut out = String::new();
    let mut all_blocks = HashSet::new();
    let mut filtered_blocks_by_replica = ReplicaMap::new(n);
    for (replica, blocks) in blocks_by_replica
        .iter()
        .filter(|(replica, _)| !faultys.contains(replica))
    {
        filtered_blocks_by_replica[replica] = Vec::new();
        for block in blocks.iter().filter_map(|b| b.as_ref()) {
            all_blocks.insert(*block);
            filtered_blocks_by_replica[replica].push(*block);
        }
        filtered_blocks_by_replica[replica].sort();
    }
    let mut min_size = usize::MAX;
    for (_, blocks) in filtered_blocks_by_replica.iter() {
        min_size = min_size.min(blocks.len());
    }

    let mut all_blocks = all_blocks.into_iter().collect::<Vec<_>>();
    all_blocks.sort();

    let mut faults_by_replica: ReplicaMap<String> = ReplicaMap::new(n);

    for (pos, block) in all_blocks
        .into_iter()
        .enumerate()
        .filter(|(pos, _)| *pos < min_size)
    {
        for (replica, blocks) in filtered_blocks_by_replica.iter() {
            if blocks[pos] != block {
                faults_by_replica[replica].push_str(&format!("{}, ", block));
            }
        }
    }

    for (replica, faults) in faults_by_replica.iter() {
        if !faults.is_empty() {
            out.push_str(&format!("\t{} misses {}\n", replica, faults));
        }
    }

    if !out.is_empty() {
        Some(out)
    } else {
        None
    }
}

fn check_deduplication(
    n: u64,
    faultys: &HashSet<ReplicaId>,
    delivery: &[(ReplicaId, TestTx)],
) -> Option<String> {
    let mut result_string = String::new();
    for i in (0..n).map(ReplicaId::from).filter(|i| !faultys.contains(i)) {
        let mut counts = HashMap::new();
        let mut inner_result_string = String::new();
        for tx in delivery
            .iter()
            .filter_map(|(sender, tx)| if *sender == i { Some(*tx) } else { None })
        {
            counts.entry(tx).and_modify(|c| *c += 1).or_insert(1u64);
        }

        for (tx, count) in counts.into_iter().filter(|(_, count)| *count > 1) {
            if inner_result_string.is_empty() {
                inner_result_string.push_str(&format!("{} ({})", tx.0, count));
            } else {
                inner_result_string.push_str(&format!(", {} ({})", tx.0, count));
            }
        }
        if !inner_result_string.is_empty() {
            if result_string.is_empty() {
                result_string.push('\n');
            }
            result_string.push_str(&format!("{}: {}", i, inner_result_string));
        }
    }

    if result_string.is_empty() {
        None
    } else {
        Some(result_string)
    }
}

fn check_liveness(
    n: u64,
    faulty_nodes: &HashSet<ReplicaId>,
    delivery: Vec<(ReplicaId, TestTx)>,
    last_tx: u64,
) -> Option<String> {
    let mut delivery_by_replica: ReplicaMap<Vec<TestTx>> = ReplicaMap::new(n);
    for (replica, tx) in delivery.into_iter() {
        delivery_by_replica[replica].push(tx);
    }

    delivery_by_replica
        .iter_mut()
        .for_each(|(_, txs)| txs.sort());

    let mut out = String::new();

    for (replica, txs) in delivery_by_replica
        .iter()
        .filter(|(replica, _)| !faulty_nodes.contains(replica))
    {
        let mut repl_out = String::new();
        for tx in (0..=last_tx).map(TestTx) {
            if !txs.contains(&tx) {
                repl_out.push_str(&format!("{} ", tx.0));
            }
        }
        if !repl_out.is_empty() {
            out.push_str(&format!("\t{} misses {}\n", replica, repl_out));
        }
    }

    if out.is_empty() {
        None
    } else {
        Some(out)
    }
}

pub fn check(
    n: u64,
    faulty_nodes: &HashSet<ReplicaId>,
    blocks_by_replica: ReplicaMap<Vec<Option<u64>>>,
    delivery: Vec<(ReplicaId, TestTx)>,
    last_tx: u64,
) -> Option<String> {
    if let Some(e) = check_blocks(n, faulty_nodes, blocks_by_replica) {
        return Some(format!("Blocks !!\n{}", e));
    }
    if let Some(deduplication) = check_deduplication(n, faulty_nodes, &delivery) {
        return Some(format!("Duplicate deliveries !! {}", deduplication));
    }

    if let Some(liveness) = check_liveness(n, faulty_nodes, delivery, last_tx) {
        return Some(format!("Liveness !! last tx: {}, \n{}", last_tx, liveness));
    }

    None
}
