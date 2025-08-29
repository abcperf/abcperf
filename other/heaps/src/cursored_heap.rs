use std::{cmp::Reverse, collections::HashSet};

use crate::{HeapWrapper, Keyable, MinHeap};

pub struct Cursor<'a, V>
where
    V: Keyable,
{
    current: Option<Reverse<HeapWrapper<V>>>,
    heap: &'a mut CursoredHeap<V>,
    kept: Vec<Reverse<HeapWrapper<V>>>,
    kept_set: HashSet<V::Key>,
}

impl<'a, V> Cursor<'a, V>
where
    V: Keyable,
{
    fn new(heap: &'a mut CursoredHeap<V>) -> Self {
        let size = heap.heap.len();
        Self {
            current: None,
            heap,
            kept: Vec::with_capacity(size),
            kept_set: HashSet::with_capacity(size),
        }
    }

    pub fn advance(&mut self) -> Option<&V> {
        if self.current.is_some() {
            self.keep();
        }
        self.current = self.heap.heap.pop();
        match &self.current {
            None => None,
            Some(v) => Some(&v.0 .0),
        }
    }

    fn keep(&mut self) {
        let c = self.current.take().unwrap();
        self.kept_set.insert(c.0 .0.key());
        self.kept.push(c);
    }

    pub fn remove(&mut self) -> V {
        self.current.take().unwrap().0 .0
    }

    pub fn finalize(mut self) {
        while self.advance().is_some() {}
        self.kept.reverse();
        self.heap.heap = MinHeap::from(self.kept);
        self.heap.known = self.kept_set
    }
}

#[derive(Debug, Clone)]
pub struct CursoredHeap<V>
where
    V: Keyable,
{
    heap: MinHeap<HeapWrapper<V>>,
    known: HashSet<V::Key>,
}

impl<V> CursoredHeap<V>
where
    V: Keyable,
{
    pub fn insert(&mut self, value: V) {
        self.known.insert(value.key());
        self.heap.push(Reverse(HeapWrapper(value)));
    }

    pub fn len(&self) -> usize {
        self.heap.len()
    }

    pub fn is_empty(&self) -> bool {
        self.heap.is_empty()
    }

    pub fn knows(&self, value: &V) -> bool {
        self.knows_key(&value.key())
    }

    pub fn knows_key(&self, key: &V::Key) -> bool {
        self.known.contains(key)
    }

    pub fn cursor(&mut self) -> Cursor<V> {
        Cursor::new(self)
    }
}

impl<V> Default for CursoredHeap<V>
where
    V: Keyable,
{
    fn default() -> Self {
        Self {
            heap: MinHeap::new(),
            known: HashSet::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::Keyable;

    use super::CursoredHeap;

    type Heap = CursoredHeap<Dummy>;

    #[derive(PartialEq, Eq, PartialOrd, Ord, Debug)]
    struct Dummy(u32);

    impl Keyable for Dummy {
        type Key = u32;
        fn key(&self) -> Self::Key {
            self.0
        }
    }

    #[test]
    fn empty_buffer() {
        let mut buffer = Heap::default();
        assert_eq!(buffer.len(), 0usize);
        let mut cursor = buffer.cursor();
        assert!(cursor.advance().is_none());
    }

    #[test]
    fn fill_and_advance() {
        let mut buffer = Heap::default();
        buffer.insert(Dummy(0));
        buffer.insert(Dummy(1));
        buffer.insert(Dummy(25));
        buffer.insert(Dummy(12));
        assert_eq!(buffer.len(), 4);
        assert!(buffer.knows(&Dummy(1)));
        assert!(buffer.knows(&Dummy(25)));

        let mut cursor = buffer.cursor();
        let v = cursor.advance();
        assert_eq!(*v.unwrap(), Dummy(0));
        let v = cursor.advance();
        assert_eq!(*v.unwrap(), Dummy(1));
        let v = cursor.advance();
        assert_eq!(*v.unwrap(), Dummy(12));
        let v = cursor.advance();
        assert_eq!(*v.unwrap(), Dummy(25));
        assert!(cursor.advance().is_none());
        cursor.finalize();
        assert_eq!(buffer.len(), 4);
        assert!(buffer.knows(&Dummy(1)));
        assert!(buffer.knows(&Dummy(25)));

        buffer.insert(Dummy(50));
        assert_eq!(buffer.len(), 5);

        let mut cursor = buffer.cursor();
        let v = cursor.advance();
        assert_eq!(*v.unwrap(), Dummy(0));
        assert_eq!(cursor.remove(), Dummy(0));
        let v = cursor.advance();
        assert_eq!(*v.unwrap(), Dummy(1));
        assert_eq!(cursor.remove(), Dummy(1));
        let v = cursor.advance();
        assert_eq!(*v.unwrap(), Dummy(12));
        let v = cursor.advance();
        assert_eq!(*v.unwrap(), Dummy(25));
        let v = cursor.advance();
        assert_eq!(*v.unwrap(), Dummy(50));
        assert!(cursor.advance().is_none());
        cursor.finalize();
        assert_eq!(buffer.len(), 3);
        assert!(!buffer.knows(&Dummy(1)));
        assert!(buffer.knows(&Dummy(50)));

        buffer.insert(Dummy(9));

        assert_eq!(buffer.len(), 4);

        let mut cursor = buffer.cursor();
        cursor.advance();
        assert_eq!(cursor.remove(), Dummy(9));
        cursor.advance();
        assert_eq!(cursor.remove(), Dummy(12));
        cursor.advance();
        assert_eq!(cursor.remove(), Dummy(25));
        cursor.advance();
        assert_eq!(cursor.remove(), Dummy(50));
        assert!(cursor.advance().is_none());
        cursor.finalize();
        assert_eq!(buffer.len(), 0);
        assert!(!buffer.knows(&Dummy(1)));
        assert!(!buffer.knows(&Dummy(50)));
    }
}
