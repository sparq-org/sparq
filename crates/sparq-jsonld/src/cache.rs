//! Small, thread-local cache storage shared by framing and context processing.

// Eight entries keep linear lookup cheap while allowing small alternating workloads.
// Each cache retains at most this many input-sized keys and processed contexts (or
// expanded frames); inverse contexts scale with their processed terms. This bounds
// entry count, not bytes: large inputs can retain large entries until eviction or
// thread exit. Compaction's per-root memo/derived-context bounds apply to each entry.
const CAPACITY: usize = 8;

pub(crate) struct Lru<T> {
    entries: Vec<T>,
    #[cfg(test)]
    pub(crate) hits: usize,
}

impl<T> Lru<T> {
    pub(crate) const fn new() -> Self {
        Self {
            entries: Vec::new(),
            #[cfg(test)]
            hits: 0,
        }
    }

    // Most recent last: repeated calls inspect one key, without cloning or hashing it.
    pub(crate) fn get(&mut self, matches: impl FnMut(&T) -> bool) -> Option<&T> {
        let index = self.entries.iter().rposition(matches)?;
        self.entries[index..].rotate_left(1);
        #[cfg(test)]
        {
            self.hits += 1;
        }
        self.entries.last()
    }

    // Called only after a cache miss, with no cache borrow held during processing.
    pub(crate) fn insert(&mut self, entry: T) {
        if self.entries.len() == CAPACITY {
            self.entries.remove(0);
        }
        self.entries.push(entry);
    }

    #[cfg(test)]
    pub(crate) fn last(&self) -> Option<&T> {
        self.entries.last()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn evicts_least_recent_entry_at_eight_entries() {
        let mut cache = Lru::new();
        for i in 0..8 {
            cache.insert(i);
        }
        assert_eq!(cache.get(|&i| i == 0), Some(&0));
        cache.insert(8);
        assert_eq!(cache.entries.len(), 8);
        assert_eq!(cache.get(|&i| i == 1), None);
        for i in [0, 2, 3, 4, 5, 6, 7, 8] {
            assert_eq!(cache.get(|&entry| entry == i), Some(&i));
        }
        // A miss must not change recency; the previous loop left 0 least recent.
        cache.insert(9);
        assert_eq!(cache.get(|&i| i == 0), None);
        assert_eq!(cache.entries.len(), 8);
    }
}
