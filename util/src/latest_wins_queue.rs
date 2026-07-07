use std::collections::{HashMap, VecDeque};
use std::hash::Hash;

/// A FIFO queue that holds each value at most once, where re-enqueuing an existing value refreshes
/// its recency by moving it to the back.
///
/// Enqueue and dedup-removal are O(1) amortized: a hash index locates an existing value in constant
/// time, and its slot is vacated in place (rather than shifted out) so other entries keep their
/// position. Vacated slots are reclaimed lazily once they dominate the storage.
#[derive(Debug)]
pub struct LatestWinsQueue<T> {
    /// Physical storage; a vacated slot (a value moved to the back) holds `None`.
    entries: VecDeque<Option<T>>,
    /// Maps a live value to its absolute index, giving O(1) deduplication.
    live_index: HashMap<T, usize>,
    /// Absolute index of `entries`' front, so stored indices stay valid across pops.
    head: usize,
    /// Number of vacated (`None`) slots currently in `entries`.
    vacated: usize,
}

impl<T> Default for LatestWinsQueue<T> {
    fn default() -> Self {
        Self {
            entries: VecDeque::new(),
            live_index: HashMap::new(),
            head: 0,
            vacated: 0,
        }
    }
}

impl<T: Hash + Eq + Clone> LatestWinsQueue<T> {
    pub fn enqueue_all(&mut self, items: impl IntoIterator<Item = T>) {
        for item in items {
            self.enqueue(item);
        }
    }

    pub fn pop_next(&mut self) -> Option<T> {
        while let Some(slot) = self.entries.pop_front() {
            self.head += 1;
            match slot {
                Some(item) => {
                    self.live_index.remove(&item);
                    return Some(item);
                }
                None => self.vacated -= 1,
            }
        }

        // Fully drained: restart absolute indexing so `head` can't grow unbounded.
        self.head = 0;
        None
    }

    fn enqueue(&mut self, item: T) {
        let new_index = self.head + self.entries.len();
        if let Some(old_index) = self.live_index.insert(item.clone(), new_index) {
            // Vacate the earlier occurrence in place so other entries keep their position.
            self.entries[old_index - self.head] = None;
            self.vacated += 1;
        }

        self.entries.push_back(Some(item));
        self.compact_if_needed();
    }

    fn compact_if_needed(&mut self) {
        if self.vacated <= self.entries.len() / 2 {
            return;
        }

        // Drop vacated slots and reindex the survivors from zero.
        self.entries.retain(|slot| slot.is_some());
        self.head = 0;
        self.vacated = 0;
        self.live_index.clear();
        for (index, slot) in self.entries.iter().enumerate() {
            let value = slot.as_ref().expect("compacted entries are live");
            self.live_index.insert(value.clone(), index);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::LatestWinsQueue;

    #[test]
    fn pops_in_fifo_order() {
        let mut queue = LatestWinsQueue::default();
        queue.enqueue_all([1, 2, 3]);
        assert_eq!(drain(&mut queue), [1, 2, 3]);
    }

    #[test]
    fn keeps_each_value_once() {
        let mut queue = LatestWinsQueue::default();
        queue.enqueue_all([1, 1, 1]);
        assert_eq!(drain(&mut queue), [1]);
    }

    #[test]
    fn reenqueue_moves_value_to_back() {
        let mut queue = LatestWinsQueue::default();
        queue.enqueue_all([1, 2, 3, 1]);
        assert_eq!(drain(&mut queue), [2, 3, 1]);
    }

    #[test]
    fn reuses_queue_after_drain() {
        let mut queue = LatestWinsQueue::default();
        queue.enqueue_all([1, 2]);
        assert_eq!(drain(&mut queue), [1, 2]);
        queue.enqueue_all([3, 4, 3]);
        assert_eq!(drain(&mut queue), [4, 3]);
    }

    #[test]
    fn compacts_under_heavy_duplication() {
        let mut queue = LatestWinsQueue::default();
        // Re-enqueue the same set repeatedly to force vacated slots to be reclaimed.
        for _ in 0..100 {
            queue.enqueue_all([1, 2, 3]);
        }
        assert_eq!(drain(&mut queue), [1, 2, 3]);
    }

    fn drain<T: std::hash::Hash + Eq + Clone>(queue: &mut LatestWinsQueue<T>) -> Vec<T> {
        let mut items = Vec::new();
        while let Some(item) = queue.pop_next() {
            items.push(item);
        }
        items
    }
}
