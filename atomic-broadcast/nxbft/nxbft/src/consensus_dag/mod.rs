use std::collections::{HashSet, VecDeque};
use std::fmt::Debug;
use std::iter;
use std::ops::{Deref, Index};

use either::Either;
use id_set::ReplicaSet;
use nxbft_base::vertex::Vertex;
use nxbft_base::{DagAddress, Round};
use shared_ids::ReplicaId;

mod dag_round;
mod vertex;

use dag_round::DagRound;

use vertex::VertexWithMeta;

#[derive(Debug, Clone)]
pub struct Dag<Tx, Sig> {
    n: u64,
    offset: Round,
    max_round: Round,
    rounds: Vec<DagRound<Tx, Sig>>,
    explored_from: Round,
}

impl<Tx, Sig> Dag<Tx, Sig> {
    pub fn new(n: u64) -> Self {
        let mut dag = Dag {
            n,
            offset: Round::from_u64(1),
            max_round: Round::from_u64(0),
            rounds: Vec::with_capacity(1),
            explored_from: Round::from_u64(0),
        };
        dag.new_round();
        dag
    }

    fn get_mut(&mut self, address: DagAddress) -> &mut VertexWithMeta<Tx, Sig> {
        let idx = self.round_to_index(address.round()).unwrap();
        self.rounds.get_mut(idx).unwrap().get_mut(address.creator())
    }

    pub fn get(&self, address: DagAddress) -> Option<&Vertex<Tx, Sig>> {
        self.get_with_meta(address).map(|v| v.deref())
    }

    fn get_with_meta(&self, address: DagAddress) -> Option<&VertexWithMeta<Tx, Sig>> {
        assert!(address.creator().as_u64() < self.n, "{:?}", address);
        if let Some(index) = self.round_to_index(address.round()) {
            self.rounds.get(index).unwrap().get(address.creator())
        } else {
            None
        }
    }

    pub fn knows(&self, address: DagAddress) -> bool {
        self.get(address).is_some()
    }

    pub fn get_round(&self, round: Round) -> impl Iterator<Item = DagAddress> + '_ {
        if let Some(index) = self.round_to_index(round) {
            Either::Left(self.rounds[index].all_valid_addresses())
        } else {
            Either::Right(iter::empty())
        }
    }

    pub fn get_round_valid_count(&self, round: Round) -> u64 {
        if let Some(index) = self.round_to_index(round) {
            self.rounds[index].valid_vertice_count()
        } else {
            0
        }
    }

    pub fn insert(&mut self, vertex: Vertex<Tx, Sig>) {
        assert!(!self.knows(vertex.address()));
        assert!(
            self.round_to_index(vertex.round()).is_some(),
            "offset: {}, rounds: {}, vertex round: {}",
            self.offset,
            self.rounds.len(),
            vertex.round()
        );
        let round = vertex.round();
        let mut reaches = ReplicaSet::with_capacity(self.n);
        if round.as_u64() % 4 == 3 {
            for creator in vertex.edges().iter() {
                reaches.merge(
                    self.get(DagAddress::new(round - 1, creator))
                        .unwrap()
                        .edges(),
                );
            }
        } else if vertex.round().as_u64() % 4 == 0 {
            for creator in vertex.edges().iter() {
                reaches.merge(
                    self.get_with_meta(DagAddress::new(round - 1, creator))
                        .unwrap()
                        .reaches_set(),
                );
            }
        }
        let vertex = VertexWithMeta::new(vertex, reaches);
        let index = self.round_to_index(vertex.round()).unwrap();
        self.rounds[index].insert(vertex);
    }

    pub fn new_round(&mut self) {
        self.rounds.push(DagRound::new(self.n));
        self.max_round += 1;
    }

    /// Depth-first search that `delivers' all vertices reachable from a start address which were not delivered yet.
    pub fn deliver_from(&mut self, address: DagAddress, max_depth: Round) -> Vec<DagAddress> {
        assert!(self.knows(address));
        assert!(address.round() >= max_depth);
        if self.get_with_meta(address).unwrap().is_delivered() {
            return Vec::new();
        }
        let mut delivered = Vec::new();
        let mut stack = Vec::with_capacity(2 * self.n as usize);
        stack.push(address);
        while let Some(v_a) = stack.pop() {
            if self.get_with_meta(v_a).unwrap().is_delivered() {
                // We would visit a vertex twice
                continue;
            }

            for edge in self.get(v_a).unwrap().edges().iter() {
                let neighbor = DagAddress::new(v_a.round() - 1, edge);

                if neighbor.round() < max_depth {
                    continue;
                }

                if self.get_with_meta(neighbor).unwrap().is_delivered() {
                    continue;
                }

                if !self.knows(neighbor) {
                    continue;
                }
                stack.push(neighbor);
            }
            delivered.push(v_a);
            self.get_mut(v_a).deliver();
        }
        delivered
    }

    /// Best first search to check if there is a path between two vertices. Asserts that both vertices are acutally part of the graph.
    pub fn has_path(&mut self, from: DagAddress, to: DagAddress) -> bool {
        assert!(self.knows(from));
        assert!(self.knows(to));
        assert!(from.round() > to.round());
        if self.explored_from < from.round() {
            self.explored_from = from.round();
            self.best_first_search_inplace(from, to)
        } else {
            self.best_first_search_expensive(from, to)
        }
    }

    fn best_first_search_inplace(&mut self, from: DagAddress, to: DagAddress) -> bool {
        let mut queue = VecDeque::with_capacity(2 * self.n as usize);
        queue.push_back(from);
        while let Some(v_a) = queue.pop_front() {
            if v_a == to {
                return true;
            }
            if self.get_with_meta(v_a).unwrap().is_path_explored() {
                // We would visit a vertex twice
                continue;
            }
            for edge in self.get(v_a).unwrap().edges().iter() {
                let neighbor = DagAddress::new(v_a.round() - 1, edge);
                if self.get_with_meta(neighbor).unwrap().is_path_explored() {
                    continue;
                }

                if !self.knows(neighbor) {
                    continue;
                }

                if edge == to.creator() {
                    queue.push_front(neighbor);
                } else {
                    queue.push_back(neighbor);
                }
            }
            self.get_mut(v_a).path_explore();
        }
        false
    }

    fn best_first_search_expensive(&self, from: DagAddress, to: DagAddress) -> bool {
        let mut queue = VecDeque::with_capacity(2 * self.n as usize);
        let mut seen = HashSet::with_capacity(2 * self.n as usize);
        queue.push_back(from);
        seen.insert(from);
        while let Some(v_a) = queue.pop_front() {
            if v_a == to {
                return true;
            }
            for edge in self.get(v_a).unwrap().edges().iter() {
                let neighbor = DagAddress::new(v_a.round() - 1, edge);
                if seen.contains(&neighbor) {
                    continue;
                }

                if !self.knows(neighbor) {
                    continue;
                }

                seen.insert(neighbor);

                if edge == to.creator() {
                    queue.push_front(neighbor);
                } else {
                    queue.push_back(neighbor);
                }
            }
        }
        false
    }

    pub fn prune(&mut self, limit: Round) -> (Round, Round, Vec<Vertex<Tx, Sig>>) {
        assert!(limit >= self.offset && limit <= self.max_round);
        let mut undelivered = Vec::new();
        for round in self.rounds.drain(0..(limit - self.offset) as usize) {
            undelivered.append(&mut round.prune());
        }
        let old_offset = self.offset;
        self.offset = limit;
        (old_offset, self.offset, undelivered)
    }

    // pub fn remove_dangling_vertices_starting_at(&mut self, address: DagAddress) {
    //     for round in (address.round.as_u64()..=self.max_round.as_u64())
    //         .rev()
    //         .map(Round::from_u64)
    //     {
    //         let Some(index) = self.round_to_index(round) else {
    //             continue;
    //         };

    //         if self.rounds[index].get(address.creator).is_some() {
    //             self.rounds[index].remove(address.creator);
    //         }
    //         if self.rounds[index].valid_vertice_count() > 0 {
    //             break;
    //         }
    //     }
    // }

    fn round_to_index(&self, round: Round) -> Option<usize> {
        if self.offset > round || round > self.max_round {
            return None;
        }
        Some((round - self.offset) as usize)
    }

    pub fn reaches(&self, address: DagAddress, replica: ReplicaId) -> bool {
        self.get_with_meta(address).unwrap().reaches(replica)
    }
}

impl<Tx, Sig> Index<DagAddress> for Dag<Tx, Sig> {
    type Output = Vertex<Tx, Sig>;
    fn index(&self, idx: DagAddress) -> &Self::Output {
        self.get(idx).unwrap()
    }
}
