use damysus_base::{
    enclave::{BlockHash, Justification},
    view::View,
    Parent,
};
use hashbar::Hashbar;
use heaps::Keyable;
use once_cell::sync::OnceCell;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use shared_ids::ReplicaId;

#[derive(Clone, Serialize, Deserialize, Debug)]
pub(crate) struct Block<Tx, Sig> {
    creator: ReplicaId,
    view: View,
    justification: Justification<Sig>,
    parent: Parent,
    transactions: Vec<Tx>,
    #[serde(skip)]
    precomputed_hash: OnceCell<BlockHash>,
    #[serde(skip)]
    executed: bool,
}

impl<Tx, Sig: AsRef<[u8]>> Block<Tx, Sig> {
    pub fn genesis() -> Self {
        Self {
            creator: ReplicaId::from_u64(0),
            view: View::default(),
            justification: Justification::Genesis,
            parent: Parent::None,
            transactions: Vec::new(),
            precomputed_hash: OnceCell::with_value(BlockHash::genesis()),
            executed: false,
        }
    }

    pub fn new(
        creator: ReplicaId,
        view: View,
        justification: Justification<Sig>,
        parent: Parent,
        transactions: Vec<Tx>,
    ) -> Self {
        Self {
            creator,
            view,
            justification,
            parent,
            transactions,
            precomputed_hash: OnceCell::new(),
            executed: false,
        }
    }

    pub fn get_justification(&self) -> &Justification<Sig> {
        &self.justification
    }

    pub fn get_view(&self) -> View {
        self.view
    }

    pub fn hash_value(&self) -> &BlockHash {
        self.precomputed_hash.get_or_init(|| {
            let mut hasher = Sha256::new();
            self.creator.hash(&mut hasher);
            self.view.hash(&mut hasher);
            self.justification.hash(&mut hasher);
            self.parent.hash(&mut hasher);
            hasher.finalize().into()
        })
    }

    pub fn get_parent(&self) -> &Parent {
        &self.parent
    }

    pub fn get_transactions(&self) -> &Vec<Tx> {
        &self.transactions
    }

    pub fn get_creator(&self) -> ReplicaId {
        self.creator
    }

    pub fn execute(&mut self) {
        self.executed = true;
    }

    pub fn not_executed(&mut self) {
        self.executed = false;
    }

    pub fn is_executed(&self) -> bool {
        self.executed
    }

    pub fn is_genesis(&self) -> bool {
        self.justification.is_genesis()
    }
}

impl<Tx, Sig> Keyable for Block<Tx, Sig> {
    type Key = View;
    fn key(&self) -> Self::Key {
        self.view
    }
}

impl<Tx, Sig: AsRef<[u8]>> Ord for Block<Tx, Sig> {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        if self.view.eq(&other.view) {
            self.hash_value().cmp(other.hash_value())
        } else {
            self.view.cmp(&other.view)
        }
    }
}

impl<Tx, Sig: AsRef<[u8]>> PartialOrd for Block<Tx, Sig> {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl<Tx, Sig: AsRef<[u8]>> Eq for Block<Tx, Sig> {}

impl<Tx, Sig: AsRef<[u8]>> PartialEq for Block<Tx, Sig> {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other).is_eq()
    }
}

pub(crate) struct Blockstore<Tx, Sig: AsRef<[u8]>> {
    blocks: Vec<Option<Block<Tx, Sig>>>,
}

impl<Tx, Sig: AsRef<[u8]>> Blockstore<Tx, Sig> {
    pub fn insert(&mut self, block: Block<Tx, Sig>) {
        let view = block.view.as_usize();

        if view > self.blocks.len() {
            let c_len = view - self.blocks.len();
            for _ in 0..c_len {
                self.blocks.push(None);
            }
        }

        if view == self.blocks.len() {
            self.blocks.push(Some(block));
        } else {
            assert!(self.blocks[view].is_none());
            self.blocks[view].replace(block);
        }
    }

    pub fn get(&self, view: View) -> Option<&Block<Tx, Sig>> {
        let view = view.as_usize();
        if view > self.blocks.len() - 1 {
            return None;
        }
        self.blocks.get(view).unwrap().as_ref()
    }

    pub fn get_mut(&mut self, view: View) -> Option<&mut Block<Tx, Sig>> {
        let view = view.as_usize();
        if view > self.blocks.len() - 1 {
            return None;
        }
        self.blocks.get_mut(view).unwrap().as_mut()
    }

    #[cfg(debug_assertions)]
    pub fn get_block_iterator(&self) -> impl Iterator<Item = Option<&Block<Tx, Sig>>> {
        self.blocks.iter().map(Option::as_ref)
    }
}

impl<Tx, Sig: AsRef<[u8]>> Default for Blockstore<Tx, Sig> {
    fn default() -> Self {
        let mut blocks = Vec::with_capacity(10000);
        blocks.push(Some(Block::genesis()));
        Blockstore { blocks }
    }
}
