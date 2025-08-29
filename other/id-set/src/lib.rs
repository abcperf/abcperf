use std::{fmt::Debug, marker::PhantomData};

use bit_set::BitSet;
use serde::{Deserialize, Serialize};
use shared_ids::{IdType, ReplicaId};

pub type ReplicaSet = IdSet<ReplicaId>;

#[derive(Clone, Default, Serialize, Deserialize, Hash, PartialEq, Eq)]
#[serde(transparent)]
pub struct IdSet<I: IdType> {
    set: BitSet<u64>,
    phantom_data: PhantomData<I>,
}

impl<I: IdType> Debug for IdSet<I> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}Set{:?}", I::NAME, self.set)
    }
}

impl<I: IdType> IdSet<I> {
    pub fn with_capacity(size: u64) -> Self {
        let mut set = BitSet::default();

        set.reserve_len_exact(size as usize);

        Self {
            set,
            phantom_data: PhantomData,
        }
    }

    pub fn insert(&mut self, id: I) -> bool {
        self.set.insert(id.into() as usize)
    }

    pub fn remove(&mut self, id: I) -> bool {
        self.set.remove(id.into() as usize)
    }

    pub fn contains(&self, id: I) -> bool {
        self.set.contains(id.into() as usize)
    }

    pub fn merge(&mut self, other: &Self) {
        self.set.union_with(&other.set);
    }

    pub fn len(&self) -> u64 {
        self.set.len() as u64
    }

    pub fn is_empty(&self) -> bool {
        self.set.is_empty()
    }

    pub fn iter(&self) -> impl Iterator<Item = I> + '_ {
        self.set.iter().map(|id| I::from(id as u64))
    }

    pub fn clear(&mut self) {
        self.set.clear();
    }
}

#[cfg(test)]
mod test {
    use shared_ids::ReplicaId;

    use crate::ReplicaSet;

    #[test]
    fn test() {
        let mut set = ReplicaSet::default();
        assert!(!set.contains(ReplicaId::from_u64(111)));
        assert_eq!(set.len(), 0);
        assert!(set.is_empty());
        set.insert(ReplicaId::from_u64(5));
        assert!(!set.contains(ReplicaId::from_u64(111)));
        assert!(set.contains(ReplicaId::from_u64(5)));
        assert_eq!(set.len(), 1);
        assert!(!set.is_empty());
        let mut count = 0;
        for r in set.iter() {
            count += 1;
            assert_eq!(r, ReplicaId::from_u64(5));
        }
        assert_eq!(count, 1);
        assert_eq!(set.len(), 1);
        assert!(!set.is_empty());
        set.remove(ReplicaId::from_u64(111));
        assert_eq!(set.len(), 1);
        assert!(!set.is_empty());
        set.remove(ReplicaId::from_u64(5));
        assert_eq!(set.len(), 0);
        assert!(set.is_empty());
        set.remove(ReplicaId::from_u64(111));
        set.remove(ReplicaId::from_u64(5));
        assert_eq!(set.len(), 0);
        assert!(set.is_empty());
        set.insert(ReplicaId::from_u64(111));
        set.insert(ReplicaId::from_u64(5));
        assert_eq!(set.len(), 2);
        assert!(!set.is_empty());
        let mut count = 0;
        for r in set.iter() {
            count += 1;
            if count == 1 {
                assert_eq!(r, ReplicaId::from_u64(5));
            } else {
                assert_eq!(r, ReplicaId::from_u64(111));
            }
        }
        assert_eq!(count, 2);
        assert_eq!(set.len(), 2);
        assert!(!set.is_empty());
        set.remove(ReplicaId::from_u64(111));
        assert_eq!(set.len(), 1);
        set.remove(ReplicaId::from_u64(5));
        assert_eq!(set.len(), 0);
    }

    #[test]
    fn test2() {
        let mut set = ReplicaSet::default();
        assert!(!set.contains(ReplicaId::from_u64(40)));
        assert_eq!(set.len(), 0);
        assert!(set.is_empty());
        set.insert(ReplicaId::from_u64(5));
        assert!(!set.contains(ReplicaId::from_u64(40)));
        assert!(set.contains(ReplicaId::from_u64(5)));
        assert_eq!(set.len(), 1);
        assert!(!set.is_empty());
        let mut count = 0;
        for r in set.iter() {
            count += 1;
            assert_eq!(r, ReplicaId::from_u64(5));
        }
        assert_eq!(count, 1);
        assert_eq!(set.len(), 1);
        assert!(!set.is_empty());
        set.remove(ReplicaId::from_u64(40));
        assert_eq!(set.len(), 1);
        assert!(!set.is_empty());
        set.remove(ReplicaId::from_u64(5));
        assert_eq!(set.len(), 0);
        assert!(set.is_empty());
        set.remove(ReplicaId::from_u64(40));
        set.remove(ReplicaId::from_u64(5));
        assert_eq!(set.len(), 0);
        assert!(set.is_empty());
        set.insert(ReplicaId::from_u64(40));
        set.insert(ReplicaId::from_u64(5));
        assert_eq!(set.len(), 2);
        assert!(!set.is_empty());
        let mut count = 0;
        for r in set.iter() {
            count += 1;
            if count == 1 {
                assert_eq!(r, ReplicaId::from_u64(5));
            } else {
                assert_eq!(r, ReplicaId::from_u64(40));
            }
        }
        assert_eq!(count, 2);
        assert_eq!(set.len(), 2);
        assert!(!set.is_empty());
        set.remove(ReplicaId::from_u64(40));
        assert_eq!(set.len(), 1);
        set.remove(ReplicaId::from_u64(5));
        assert_eq!(set.len(), 0);
    }
}
