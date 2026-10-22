use std::{collections::BTreeMap, fs::Metadata, sync::Arc, time::SystemTime};

const MAX_BYTES: usize = 48 * 1024 * 1024;
const MAX_ENTRIES: usize = 128;

#[derive(Clone, PartialEq, Eq)]
pub(super) struct Stamp {
    length: u64,
    modified: Option<SystemTime>,
    #[cfg(unix)]
    identity: (u64, u64, i64, i64),
}

impl Stamp {
    pub fn of(metadata: &Metadata) -> Self {
        Self {
            length: metadata.len(),
            modified: metadata.modified().ok(),
            #[cfg(unix)]
            identity: {
                use std::os::unix::fs::MetadataExt;
                (
                    metadata.dev(),
                    metadata.ino(),
                    metadata.ctime(),
                    metadata.ctime_nsec(),
                )
            },
        }
    }

    /// Rename changes ctime. Compare content metadata around publication, then
    /// retain the final ctime for all subsequent cache checks.
    pub fn published(before: &Metadata, after: &Metadata) -> Option<Self> {
        let before = Self::of(before);
        let after = Self::of(after);
        if before.length != after.length
            || before.modified.is_none()
            || before.modified != after.modified
        {
            return None;
        }
        #[cfg(unix)]
        if (before.identity.0, before.identity.1) != (after.identity.0, after.identity.1) {
            return None;
        }
        Some(after)
    }
}

struct Entry {
    revision: u64,
    stamp: Stamp,
    bytes: Arc<Vec<u8>>,
    used: u64,
}

/// Retain the read buffer itself: converting Vec to Arc<[u8]> copies its entire
/// allocation. Charge capacity so spare buffer space cannot escape the budget.
pub(super) struct Cache {
    entries: BTreeMap<String, Entry>,
    // The detail cache can contain many small entries; eviction must not scan
    // all of them on every insert once the byte or entry limit is reached.
    oldest: BTreeMap<u64, String>,
    bytes: usize,
    clock: u64,
    max_bytes: usize,
    max_entries: usize,
}

impl Default for Cache {
    fn default() -> Self {
        Self::new(MAX_BYTES, MAX_ENTRIES)
    }
}

impl Cache {
    pub(super) fn new(max_bytes: usize, max_entries: usize) -> Self {
        Self {
            entries: BTreeMap::new(),
            oldest: BTreeMap::new(),
            bytes: 0,
            clock: 0,
            max_bytes,
            max_entries,
        }
    }

    pub fn get(&mut self, id: &str, revision: u64, stamp: &Stamp) -> Option<Arc<Vec<u8>>> {
        let entry = self.entries.get_mut(id)?;
        if entry.revision > revision {
            // A read started before a save must not evict that save's view.
            return None;
        }
        if entry.revision != revision || &entry.stamp != stamp {
            self.remove(id);
            return None;
        }
        self.clock += 1;
        self.oldest.remove(&entry.used);
        self.oldest.insert(self.clock, id.to_owned());
        entry.used = self.clock;
        Some(entry.bytes.clone())
    }

    pub fn insert(&mut self, id: String, revision: u64, stamp: Stamp, bytes: Arc<Vec<u8>>) {
        if self
            .entries
            .get(&id)
            .is_some_and(|entry| entry.revision > revision)
        {
            return;
        }
        self.remove(&id);
        if bytes.capacity() > self.max_bytes || self.max_entries == 0 {
            return;
        }
        while self.bytes + bytes.capacity() > self.max_bytes
            || self.entries.len() >= self.max_entries
        {
            let oldest = self
                .oldest
                .first_key_value()
                .map(|(_, id)| id.clone())
                .expect("nonempty cache exceeds budget");
            self.remove(&oldest);
        }
        self.clock += 1;
        self.bytes += bytes.capacity();
        self.oldest.insert(self.clock, id.clone());
        self.entries.insert(
            id,
            Entry {
                revision,
                stamp,
                bytes,
                used: self.clock,
            },
        );
    }

    pub fn remove(&mut self, id: &str) {
        if let Some(entry) = self.entries.remove(id) {
            self.oldest.remove(&entry.used);
            self.bytes -= entry.bytes.capacity();
        }
    }

    #[cfg(test)]
    pub fn usage(&self) -> (usize, usize) {
        (self.bytes, self.entries.len())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn delayed_read_cannot_evict_or_replace_a_newer_saved_view() {
        let stamp = Stamp {
            length: 3,
            modified: None,
            #[cfg(unix)]
            identity: (0, 0, 0, 0),
        };
        let mut cache = Cache::new(6, 2);
        cache.insert("a".into(), 2, stamp.clone(), Arc::new(b"new".to_vec()));
        assert!(cache.get("a", 1, &stamp).is_none());
        cache.insert("a".into(), 1, stamp.clone(), Arc::new(b"old".to_vec()));
        assert_eq!(cache.get("a", 2, &stamp).unwrap().as_slice(), b"new");
        assert_eq!(cache.usage(), (3, 1));
    }

    #[test]
    fn lru_evicts_the_least_recent_entry_and_enforces_the_byte_budget() {
        let stamp = Stamp {
            length: 3,
            modified: None,
            #[cfg(unix)]
            identity: (0, 0, 0, 0),
        };
        for (bytes, entries) in [(6, 10), (100, 2)] {
            let mut cache = Cache::new(bytes, entries);
            for id in ["a", "b"] {
                cache.insert(id.into(), 1, stamp.clone(), Arc::new(b"abc".to_vec()));
            }
            assert!(cache.get("a", 1, &stamp).is_some());
            cache.insert("c".into(), 1, stamp.clone(), Arc::new(b"abc".to_vec()));
            assert!(cache.get("b", 1, &stamp).is_none());
            assert!(cache.get("a", 1, &stamp).is_some());
            assert!(cache.get("c", 1, &stamp).is_some());
            assert_eq!(cache.usage(), (6, 2));
            assert!(cache.get("a", 2, &stamp).is_none());
            assert_eq!(cache.usage(), (3, 1));
        }
    }

    #[test]
    fn spare_buffer_capacity_is_charged_without_copying_the_allocation() {
        let stamp = Stamp {
            length: 3,
            modified: None,
            #[cfg(unix)]
            identity: (0, 0, 0, 0),
        };
        let mut bytes = Vec::with_capacity(32);
        bytes.extend_from_slice(b"abc");
        let allocation = bytes.as_ptr();
        let bytes = Arc::new(bytes);
        let mut cache = Cache::new(bytes.capacity(), 2);
        cache.insert("a".into(), 1, stamp.clone(), bytes.clone());
        assert_eq!(cache.usage(), (bytes.capacity(), 1));
        assert_eq!(cache.get("a", 1, &stamp).unwrap().as_ptr(), allocation);
        cache.insert("b".into(), 1, stamp.clone(), Arc::new(b"def".to_vec()));
        assert!(cache.get("a", 1, &stamp).is_none());
        let mut too_small = Cache::new(bytes.capacity() - 1, 2);
        too_small.insert("a".into(), 1, stamp, bytes);
        assert_eq!(too_small.usage(), (0, 0));
    }
}
