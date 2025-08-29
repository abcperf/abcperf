use std::cmp::Ordering;
use std::{cmp::Reverse, collections::BinaryHeap};

mod cursored_heap;
mod heap_buffer;
pub mod pending_client_requests;
type MinHeap<T> = BinaryHeap<Reverse<T>>;

pub use cursored_heap::Cursor;
pub use cursored_heap::CursoredHeap;
pub use heap_buffer::HeapBuffer;

pub trait Keyable {
    type Key: std::cmp::Eq + std::hash::Hash + Copy + std::cmp::Ord;
    fn key(&self) -> Self::Key;
}

#[derive(Debug, Clone)]
struct HeapWrapper<V>(V);

impl<V: Keyable> PartialEq for HeapWrapper<V> {
    fn eq(&self, other: &Self) -> bool {
        self.0.key() == other.0.key()
    }
}

impl<V: Keyable> Eq for HeapWrapper<V> {}

impl<V: Keyable> PartialOrd for HeapWrapper<V> {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.0.key().cmp(&other.0.key()))
    }
}

impl<V: Keyable> Ord for HeapWrapper<V> {
    fn cmp(&self, other: &Self) -> Ordering {
        self.0.key().cmp(&other.0.key())
    }
}
