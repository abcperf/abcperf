use super::vertex::VertexWithMeta;
use nxbft_base::vertex::Vertex;
use nxbft_base::DagAddress;
use shared_ids::ReplicaId;

#[derive(Debug, Clone)]
pub(crate) struct DagRound<Tx, Sig> {
    vertices: Vec<Option<VertexWithMeta<Tx, Sig>>>,
    valid_vertice_count: u64,
}

impl<Tx, Sig> DagRound<Tx, Sig> {
    pub(crate) fn new(n: u64) -> Self {
        DagRound {
            vertices: (0..n).map(|_| None).collect(),
            valid_vertice_count: 0,
        }
    }

    pub(crate) fn get_mut(&mut self, creator: ReplicaId) -> &mut VertexWithMeta<Tx, Sig> {
        self.vertices
            .get_mut(creator.as_u64() as usize)
            .unwrap()
            .as_mut()
            .unwrap()
    }

    pub(crate) fn get(&self, creator: ReplicaId) -> Option<&VertexWithMeta<Tx, Sig>> {
        match self.vertices.get(creator.as_u64() as usize) {
            Some(Some(v)) => Some(v),
            _ => None,
        }
    }

    pub(crate) fn all_valid_addresses(&self) -> impl Iterator<Item = DagAddress> + '_ {
        self.vertices
            .iter()
            .filter_map(|o| o.as_ref().map(|v| v.address()))
    }

    pub(crate) fn valid_vertice_count(&self) -> u64 {
        self.valid_vertice_count
    }

    pub(crate) fn insert(&mut self, vertex: VertexWithMeta<Tx, Sig>) {
        let creator = vertex.creator().as_u64() as usize;
        self.vertices[creator] = Some(vertex);
        self.valid_vertice_count += 1;
    }

    // pub(crate) fn remove(&mut self, creator: ReplicaId) {
    //     let creator = creator.as_u64() as usize;
    //     if self.vertices[creator].is_some() {
    //         self.vertices[creator] = None;
    //         self.valid_vertice_count -= 1;
    //     }
    // }

    pub(crate) fn prune(self) -> Vec<Vertex<Tx, Sig>> {
        self.vertices
            .into_iter()
            .flatten()
            .filter(|v| !v.is_delivered())
            .map(|v| v.into())
            .collect()
    }
}
