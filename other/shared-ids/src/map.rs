use std::{
    marker::PhantomData,
    ops::{Index, IndexMut},
};

use serde::{Deserialize, Serialize};

use crate::{IdIter, IdType, ReplicaId};

pub type ReplicaMap<T> = IdMap<ReplicaId, T>;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IdMap<I: IdType, T> {
    data: Box<[T]>,
    phantom_data: PhantomData<I>,
}

impl<I: IdType, T: Default> IdMap<I, T> {
    pub fn new(size: u64) -> Self {
        let data = (0..size).map(|_| T::default()).collect();
        Self {
            data,
            phantom_data: PhantomData,
        }
    }

    pub fn iter(&self) -> impl Iterator<Item = (I, &T)> {
        IdIter::default().zip(self.values())
    }

    pub fn iter_mut(&mut self) -> impl Iterator<Item = (I, &mut T)> {
        IdIter::default().zip(self.values_mut())
    }

    pub fn values(&self) -> impl Iterator<Item = &T> {
        self.data.iter()
    }

    pub fn values_mut(&mut self) -> impl Iterator<Item = &mut T> {
        self.data.iter_mut()
    }
}

impl<I: IdType, T> IdMap<I, Option<T>> {
    pub fn iter_some(&self) -> impl Iterator<Item = (I, &T)> {
        self.iter()
            .flat_map(|(id, option)| option.as_ref().map(|t| (id, t)))
    }

    pub fn iter_mut_some(&mut self) -> impl Iterator<Item = (I, &mut T)> {
        self.iter_mut()
            .flat_map(|(id, option)| option.as_mut().map(|t| (id, t)))
    }

    pub fn values_some(&self) -> impl Iterator<Item = &T> {
        self.values().filter_map(|o| o.as_ref())
    }

    pub fn values_mut_some(&mut self) -> impl Iterator<Item = &mut T> {
        self.values_mut().filter_map(|o| o.as_mut())
    }

    pub fn keys_some(&self) -> impl Iterator<Item = I> + '_ {
        self.iter_some().map(|(k, _v)| k)
    }

    pub fn clear_some(&mut self) {
        self.values_mut().for_each(|e| *e = None);
    }

    pub fn drain_lazy_some(&mut self) -> impl Iterator<Item = (I, T)> + '_ {
        self.iter_mut().flat_map(|(i, o)| o.take().map(|t| (i, t)))
    }
}

impl<I: IdType, T> IdMap<I, T> {
    pub fn into_boxed_slice(self) -> Box<[T]> {
        self.data
    }
}

impl<I: IdType, T> Index<I> for IdMap<I, T> {
    type Output = T;

    fn index(&self, index: I) -> &Self::Output {
        &self.data[index.into() as usize]
    }
}

impl<I: IdType, T> IndexMut<I> for IdMap<I, T> {
    fn index_mut(&mut self, index: I) -> &mut Self::Output {
        &mut self.data[index.into() as usize]
    }
}

impl<I: IdType, T> From<Vec<T>> for IdMap<I, T> {
    fn from(value: Vec<T>) -> Self {
        value.into_boxed_slice().into()
    }
}

impl<I: IdType, T> From<Box<[T]>> for IdMap<I, T> {
    fn from(value: Box<[T]>) -> Self {
        Self {
            data: value,
            phantom_data: PhantomData,
        }
    }
}
