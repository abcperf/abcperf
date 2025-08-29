use std::{cmp::Reverse, collections::HashSet};

use crate::{HeapWrapper, Keyable, MinHeap};

#[derive(Clone)]
pub struct HeapBuffer<T>
where
    T: Keyable,
{
    buffer: MinHeap<HeapWrapper<T>>,
    known: HashSet<T::Key>,
}

impl<T> HeapBuffer<T>
where
    T: Keyable,
{
    pub fn peek(&self) -> Option<&T> {
        match self.buffer.peek() {
            Some(value) => Some(&value.0 .0),
            None => None,
        }
    }

    pub fn knows(&self, value: &T) -> bool {
        self.knows_key(&value.key())
    }

    pub fn knows_key(&self, key: &T::Key) -> bool {
        self.known.contains(key)
    }

    pub fn clear(&mut self) {
        self.buffer.clear();
        self.known.clear();
    }

    pub fn find_by_key(&self, key: &T::Key) -> Option<&T> {
        self.buffer
            .iter()
            .map(|v| &v.0 .0)
            .find(|&value| value.key() == *key)
    }
}

impl<T> HeapBuffer<T>
where
    T: Keyable,
{
    pub fn insert(&mut self, value: T) {
        debug_assert!(!self.knows(&value));
        self.known.insert(value.key());
        self.buffer.push(Reverse(HeapWrapper(value)));
    }

    pub fn pop(&mut self) -> Option<T> {
        match self.buffer.pop() {
            None => None,
            Some(value) => {
                self.known.remove(&value.0 .0.key());
                Some(value.0 .0)
            }
        }
    }
}

impl<T> Default for HeapBuffer<T>
where
    T: Keyable,
{
    fn default() -> Self {
        Self {
            buffer: MinHeap::new(),
            known: HashSet::new(),
        }
    }
}

#[cfg(test)]
mod tests {}
