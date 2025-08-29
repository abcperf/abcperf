use std::{
    cmp::Ordering,
    collections::{binary_heap::PeekMut, BinaryHeap},
    convert::Infallible,
    future,
    time::Instant,
};
use tokio::time;

use trait_alias_macro::pub_trait_alias_macro;

#[derive(Debug, PartialEq, Eq, Clone)]
pub struct StopClass(pub u64);

pub_trait_alias_macro!(TimeoutType = Eq + Clone + Send + Sync + 'static);

struct TimeoutListEntry<T: TimeoutType> {
    timeout: Instant,
    typ: T,
    stop_class: StopClass,
}

impl<T: TimeoutType> Ord for TimeoutListEntry<T> {
    fn cmp(&self, other: &Self) -> Ordering {
        self.timeout.cmp(&other.timeout).reverse()
    }
}

impl<T: TimeoutType> PartialOrd for TimeoutListEntry<T> {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl<T: TimeoutType> PartialEq for TimeoutListEntry<T> {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other).is_eq()
    }
}

impl<T: TimeoutType> Eq for TimeoutListEntry<T> {}

pub(crate) struct TimeoutList<T: TimeoutType> {
    timeouts: BinaryHeap<TimeoutListEntry<T>>,
}

impl<T: TimeoutType> TimeoutList<T> {
    pub(crate) fn new() -> Self {
        TimeoutList {
            timeouts: BinaryHeap::new(),
        }
    }

    pub(crate) fn insert(&mut self, typ: T, timeout: Instant, stop_class: StopClass) {
        self.remove_all(&typ);
        self.timeouts.push(TimeoutListEntry {
            timeout,
            typ,
            stop_class,
        });
    }

    pub(crate) fn remove(&mut self, typ: &T, stop_class: StopClass) {
        self.timeouts
            .retain(|entry| entry.typ != *typ || entry.stop_class != stop_class);
    }

    pub(crate) fn remove_all(&mut self, typ: &T) {
        self.timeouts.retain(|entry| entry.typ != *typ);
    }

    pub(crate) async fn wait_for_timeout(&mut self) -> (Instant, T) {
        let Some(entry) = self.timeouts.peek_mut() else {
            match future::pending::<Infallible>().await {};
        };

        time::sleep_until(entry.timeout.into()).await;

        let poped = PeekMut::pop(entry);
        (poped.timeout, poped.typ)
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use crate::timeout::{StopClass, TimeoutList};

    #[tokio::test]
    async fn test_timeout_list() {
        let mut timeout_list = TimeoutList::<u32>::new();

        let now = Instant::now();
        timeout_list.insert(1, now + Duration::from_millis(5), StopClass(1));
        timeout_list.insert(2, now + Duration::from_millis(4), StopClass(1));
        timeout_list.insert(3, now + Duration::from_millis(1), StopClass(1));
        timeout_list.insert(4, now + Duration::from_millis(2), StopClass(1));

        assert_eq!(timeout_list.wait_for_timeout().await.1, 3);
        timeout_list.insert(5, now + Duration::from_millis(3), StopClass(1));
        assert_eq!(timeout_list.wait_for_timeout().await.1, 4);
        assert_eq!(timeout_list.wait_for_timeout().await.1, 5);
        assert_eq!(timeout_list.wait_for_timeout().await.1, 2);
        assert_eq!(timeout_list.wait_for_timeout().await.1, 1);
    }
}
