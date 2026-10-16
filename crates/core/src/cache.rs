//! Insertion-ordered, stamp-invalidated cache. Callers supply real-path keys.
use std::{
    collections::VecDeque,
    path::{Path, PathBuf},
};

pub use plan_viewer_source::Stamp;
#[derive(Debug)]
pub struct Cache<Value> {
    entries: VecDeque<(PathBuf, Stamp, Value)>,
}
impl<Value> Default for Cache<Value> {
    fn default() -> Self {
        Self { entries: VecDeque::new() }
    }
}
impl<Value> Cache<Value> {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn get(&self, key: &Path, stamp: Stamp) -> Option<&Value> {
        self.entries
            .iter()
            .find(|(entry_key, entry_stamp, _)| entry_key == key && *entry_stamp == stamp)
            .map(|(_, _, value)| value)
    }
    /// Replacing a stamp does not promote its key, matching Map.set in Bun.
    pub fn insert(&mut self, key: PathBuf, stamp: Stamp, value: Value) {
        if let Some(entry) = self.entries.iter_mut().find(|(entry_key, _, _)| *entry_key == key) {
            *entry = (key, stamp, value);
        } else {
            self.entries.push_back((key, stamp, value));
            if self.entries.len() > 100 {
                self.entries.pop_front();
            }
        }
    }
    pub fn len(&self) -> usize {
        self.entries.len()
    }
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}
impl<Value: Clone> Cache<Value> {
    pub fn cached(&mut self, key: &Path, stamp: Stamp, compute: impl FnOnce() -> Value) -> Value {
        if let Some(value) = self.get(key, stamp) {
            return value.clone();
        }
        let value = compute();
        self.insert(key.into(), stamp, value.clone());
        value
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::time::SystemTime;
    #[test]
    fn hit_and_stamp_invalidation() {
        let mut cache = Cache::new();
        let key = Path::new("/real/doc.md");
        let stamp = Stamp {
            mtime: SystemTime::UNIX_EPOCH,
            size: 1,
        };
        assert_eq!(cache.cached(key, stamp, || 1), 1);
        assert_eq!(cache.cached(key, stamp, || panic!("cache miss")), 1);
        let bigger = Stamp { size: 2, ..stamp };
        assert_eq!(cache.cached(key, bigger, || 2), 2);
        assert!(cache.get(key, stamp).is_none());
        let later = Stamp {
            mtime: SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1),
            ..bigger
        };
        assert_eq!(cache.cached(key, later, || 3), 3);
    }
    #[test]
    fn insertion_order_eviction_at_100() {
        let mut cache = Cache::new();
        let stamp = Stamp {
            mtime: SystemTime::UNIX_EPOCH,
            size: 1,
        };
        for index in 0..100 {
            cache.insert(PathBuf::from(format!("/{index}")), stamp, index);
        }
        assert_eq!(cache.len(), 100);
        cache.insert("/0".into(), Stamp { size: 2, ..stamp }, 200);
        cache.insert("/100".into(), stamp, 100);
        assert_eq!(cache.len(), 100);
        assert!(cache.get(Path::new("/0"), Stamp { size: 2, ..stamp }).is_none());
        assert_eq!(cache.get(Path::new("/1"), stamp), Some(&1));
    }
}
