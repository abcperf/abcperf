use std::fmt::Display;

use hashbar::{Hashbar, Hasher};
use serde::{Deserialize, Serialize};
use shared_ids::ReplicaId;

#[derive(
    Default, Copy, Clone, Serialize, Deserialize, Eq, Hash, PartialEq, PartialOrd, Ord, Debug,
)]
pub struct View(u64);

impl View {
    pub fn get_predecessor(&self) -> Option<Self> {
        if self.0 == 0 {
            None
        } else {
            Some(Self(self.0 - 1))
        }
    }

    pub fn get_successor(&self) -> Self {
        Self(self.0 + 1)
    }

    pub fn increment(&mut self) {
        self.0 += 1;
    }

    pub fn get_view_leader(&self, n: u64) -> ReplicaId {
        ReplicaId::from_u64((self.0 / 2) % n)
    }

    pub fn as_usize(&self) -> usize {
        self.0 as usize
    }

    pub fn as_u64(&self) -> u64 {
        self.0
    }

    pub fn new(view: u64) -> Self {
        Self(view)
    }
}

impl Display for View {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl Hashbar for View {
    fn hash<H: Hasher>(&self, hasher: &mut H) {
        hasher.update(&self.0.to_le_bytes());
    }
}
