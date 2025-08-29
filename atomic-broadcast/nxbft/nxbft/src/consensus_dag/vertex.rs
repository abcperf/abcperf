use std::ops::Deref;

use id_set::ReplicaSet;
use nxbft_base::vertex::Vertex;
use shared_ids::ReplicaId;

#[derive(Debug, Clone)]
pub(crate) struct VertexWithMeta<Tx, Sig> {
    vertex: Vertex<Tx, Sig>,
    delivered: bool,
    path_explored: bool,
    reaches: ReplicaSet,
}

impl<Tx, Sig> VertexWithMeta<Tx, Sig> {
    pub(crate) fn new(vertex: Vertex<Tx, Sig>, reaches: ReplicaSet) -> Self {
        Self {
            vertex,
            delivered: false,
            path_explored: false,
            reaches,
        }
    }
}

impl<Tx, Sig> Deref for VertexWithMeta<Tx, Sig> {
    type Target = Vertex<Tx, Sig>;

    fn deref(&self) -> &Self::Target {
        &self.vertex
    }
}

impl<Tx, Sig> From<VertexWithMeta<Tx, Sig>> for Vertex<Tx, Sig> {
    fn from(value: VertexWithMeta<Tx, Sig>) -> Self {
        value.vertex
    }
}

impl<Tx, Sig> VertexWithMeta<Tx, Sig> {
    pub(crate) fn is_delivered(&self) -> bool {
        self.delivered
    }

    pub(crate) fn deliver(&mut self) {
        self.delivered = true;
    }

    pub(crate) fn is_path_explored(&self) -> bool {
        self.path_explored
    }

    pub(crate) fn path_explore(&mut self) {
        self.path_explored = true;
    }

    pub(crate) fn reaches_set(&self) -> &ReplicaSet {
        &self.reaches
    }

    pub(crate) fn reaches(&self, replica: ReplicaId) -> bool {
        self.reaches.contains(replica)
    }
}
