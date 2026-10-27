//! Space-Saving stream algorithm for tracking top-K frequent items in bounded memory.
//!
//! Based on Metwally, Agrawal, and El Abbadi (2005): "Efficient Computation of Frequent
//! and Top-k Elements in Data Streams".

use crate::models::TopItem;
use std::collections::HashMap;

/// Bounded-memory frequency counter implementing the Space-Saving algorithm.
#[derive(Debug, Clone)]
pub struct SpaceSavingCounter {
    capacity: usize,
    entries: HashMap<String, u64>,
}

impl SpaceSavingCounter {
    /// Create a new Space-Saving counter with the given maximum element capacity.
    pub fn new(capacity: usize) -> Self {
        Self {
            capacity: capacity.max(1),
            entries: HashMap::with_capacity(capacity.max(1)),
        }
    }

    /// Increment count of an item by 1.
    pub fn increment(&mut self, item: impl Into<String>) {
        self.increment_by(item, 1);
    }

    /// Increment count of an item by `count`.
    pub fn increment_by(&mut self, item: impl Into<String>, count: u64) {
        if count == 0 {
            return;
        }

        let key = item.into();

        if let Some(existing) = self.entries.get_mut(&key) {
            *existing += count;
            return;
        }

        if self.entries.len() < self.capacity {
            self.entries.insert(key, count);
            return;
        }

        // Bounded capacity reached: find the item with the minimum frequency
        if let Some((min_key, min_count)) = self
            .entries
            .iter()
            .min_by_key(|entry| *entry.1)
            .map(|(k, &v)| (k.clone(), v))
        {
            self.entries.remove(&min_key);
            self.entries.insert(key, min_count + count);
        }
    }

    /// Return top `k` most frequent items, sorted in descending order of frequency.
    pub fn top_k(&self, k: usize) -> Vec<TopItem> {
        let mut items: Vec<TopItem> = self
            .entries
            .iter()
            .map(|(name, &count)| TopItem {
                name: name.clone(),
                count,
            })
            .collect();

        items.sort_by(|a, b| b.count.cmp(&a.count).then_with(|| a.name.cmp(&b.name)));
        items.truncate(k);
        items
    }

    /// Merge counts from another Space-Saving counter.
    pub fn merge(&mut self, other: &SpaceSavingCounter) {
        for (item, &count) in &other.entries {
            self.increment_by(item.clone(), count);
        }
    }

    /// Clear all counters.
    pub fn clear(&mut self) {
        self.entries.clear();
    }

    /// Number of tracked items currently in memory.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Check if the counter is empty.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_space_saving_basic_counting() {
        let mut ss = SpaceSavingCounter::new(10);
        ss.increment("apple");
        ss.increment("banana");
        ss.increment("apple");
        ss.increment_by("orange", 5);

        let top = ss.top_k(3);
        assert_eq!(top.len(), 3);
        assert_eq!(top[0].name, "orange");
        assert_eq!(top[0].count, 5);
        assert_eq!(top[1].name, "apple");
        assert_eq!(top[1].count, 2);
        assert_eq!(top[2].name, "banana");
        assert_eq!(top[2].count, 1);
    }

    #[test]
    fn test_space_saving_bounded_capacity() {
        let mut ss = SpaceSavingCounter::new(3);
        ss.increment_by("item1", 100);
        ss.increment_by("item2", 50);
        ss.increment_by("item3", 10);

        // Capacity is full (3 items). Next insertion of "item4" should evict "item3" (lowest count 10)
        ss.increment_by("item4", 5);
        assert_eq!(ss.len(), 3);

        let top = ss.top_k(3);
        let names: Vec<&str> = top.iter().map(|t| t.name.as_str()).collect();
        assert!(names.contains(&"item1"));
        assert!(names.contains(&"item2"));
        assert!(names.contains(&"item4"));
        assert!(!names.contains(&"item3"));
    }

    #[test]
    fn test_space_saving_merge() {
        let mut ss1 = SpaceSavingCounter::new(5);
        ss1.increment_by("a", 10);
        ss1.increment_by("b", 5);

        let mut ss2 = SpaceSavingCounter::new(5);
        ss2.increment_by("b", 15);
        ss2.increment_by("c", 20);

        ss1.merge(&ss2);
        let top = ss1.top_k(3);
        assert_eq!(top[0].name, "b");
        assert_eq!(top[0].count, 20);
        assert_eq!(top[1].name, "c");
        assert_eq!(top[1].count, 20);
        assert_eq!(top[2].name, "a");
        assert_eq!(top[2].count, 10);
    }
}
