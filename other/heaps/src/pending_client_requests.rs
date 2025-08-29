use std::collections::VecDeque;

pub struct PendingClientRequests<Tx> {
    blocks: VecDeque<Vec<Tx>>,
    maximum_block_size: usize,
}

impl<Tx: Clone> PendingClientRequests<Tx> {
    pub fn new(maximum_block_size: usize) -> Self {
        Self {
            blocks: VecDeque::new(),
            maximum_block_size,
        }
    }

    pub fn copy_front(&self) -> Vec<Tx> {
        if let Some(front) = self.blocks.front() {
            front.clone()
        } else {
            Vec::new()
        }
    }

    pub fn pop_front(&mut self) -> Vec<Tx> {
        self.blocks.pop_front().unwrap_or_default()
    }

    pub fn filter_front(&mut self, mut retain: impl FnMut(&Tx) -> bool) {
        while let Some(front) = self.blocks.front_mut() {
            front.retain(&mut retain);
            if front.is_empty() {
                self.blocks.pop_front();
            } else {
                break;
            }
        }
    }

    pub fn insert(&mut self, tx: Tx) {
        let length_of_last = self.blocks.back().map_or(usize::MAX, Vec::len);
        if length_of_last >= self.maximum_block_size {
            self.blocks.push_back(Vec::new());
        }
        self.blocks.back_mut().expect("we pushed if empty").push(tx);
    }

    pub fn full_block_or_at_least(&self, minium_block_size: usize) -> bool {
        assert!(minium_block_size > 0);
        if let Some(last) = self.blocks.back() {
            self.blocks.len() > 1 || last.len() >= minium_block_size
        } else {
            false
        }
    }
}
