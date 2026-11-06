use std::collections::HashMap;
use std::fs::File;
use std::io;
use std::path::Path;
use std::sync::{Mutex, MutexGuard};

use crate::file_cache::CacheTable;

/// Per-entry LRU state: when it was last used, and its size for the byte budget.
struct LruState {
    last_use: u64,
    size: usize,
}

/// Everything guarded by the one lock: the entries, the bytes they hold, and the use clock.
#[derive(Default)]
struct LruInner {
    map: HashMap<String, LruState>,
    used: usize,
    tick: u64,
}

impl LruInner {
    fn next_tick(&mut self) -> u64 {
        self.tick += 1;
        self.tick
    }
}

/// LRU cache of files with a byte cap. `CacheTable` holds the bytes on disk; this holds the
/// policy state. File writes and deletes run outside the lock, so a slow write never blocks
/// `get`. Only entries in the map count toward `used`, so disk use can briefly exceed `byte_cap`
/// by files being written or victims not yet deleted.
pub struct LruCache {
    table: CacheTable,
    inner: Mutex<LruInner>,
    byte_cap: usize,
}

impl LruCache {
    pub fn new(base_path: impl AsRef<Path>, byte_cap: usize) -> io::Result<Self> {
        Ok(Self {
            table: CacheTable::new(base_path)?,
            inner: Mutex::new(LruInner::default()),
            byte_cap,
        })
    }

    /// Caches `buffer` under `key`, evicting least recently used entries until it fits.
    /// Errors: AlreadyExists if the key is cached (it counts as a use) or being added by
    /// someone else; InvalidInput if the item is bigger than the whole cache; or the I/O error
    /// from writing the file (nothing is evicted then).
    pub fn add(&self, key: &str, buffer: &[u8]) -> io::Result<()> {
        let size = buffer.len();
        if size > self.byte_cap {
            return Err(io::ErrorKind::InvalidInput.into());
        }

        // Lock 1: already cached -> just a use
        {
            let mut inner = self.lock();
            let tick = inner.next_tick();
            if let Some(state) = inner.map.get_mut(key) {
                state.last_use = tick;
                return Err(io::ErrorKind::AlreadyExists.into());
            }
        }

        // No lock: write the file. CacheTable lets only one concurrent add of a key succeed,
        // and `get` cannot see the key until it is in the map below.
        self.table.insert(key, buffer)?;

        // Lock 2: evict victims from the map, then record the new entry
        let victims = {
            let mut inner = self.lock();
            let mut victims = Vec::new();
            while inner.used + size > self.byte_cap {
                let Some(victim) = inner
                    .map
                    .iter()
                    .min_by_key(|(_, s)| s.last_use)
                    .map(|(k, _)| k.clone())
                else {
                    break; // empty map: cannot happen since size <= byte_cap
                };
                let state = inner.map.remove(&victim).expect("victim came from the map");
                inner.used -= state.size;
                victims.push(victim);
            }
            let last_use = inner.next_tick();
            inner
                .map
                .insert(key.to_string(), LruState { last_use, size });
            inner.used += size;
            victims
        };

        // No lock: delete the victims' files. They already left the map, so `get` misses them,
        // and readers that opened one earlier keep their handle.
        for victim in victims {
            self.table.remove(&victim);
        }
        Ok(())
    }

    /// Opens the cached file for `key` and marks it used. None on a miss.
    pub fn get(&self, key: &str) -> Option<(File, usize)> {
        {
            let mut inner = self.lock();
            let tick = inner.next_tick();
            inner.map.get_mut(key)?.last_use = tick;
        }
        // Outside the lock: if the entry was evicted just now, this is simply a miss
        self.table.get(key)
    }

    /// Bytes held by cached entries.
    pub fn used(&self) -> usize {
        self.lock().used
    }

    fn lock(&self) -> MutexGuard<'_, LruInner> {
        // A panic while holding the lock leaves the state consistent, so keep using it
        self.inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use std::fs;
    use std::io::Read;
    use std::path::PathBuf;

    fn test_dir(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("obm_lru_cache_{name}_{}", std::process::id()))
    }

    fn read_all(mut file: File) -> Vec<u8> {
        let mut out = Vec::new();
        file.read_to_end(&mut out).unwrap();
        out
    }

    #[test]
    fn test_add_get() {
        let dir = test_dir("basic");
        let cache = LruCache::new(&dir, 100).unwrap();

        assert!(cache.get("a").is_none());
        cache.add("a", b"hello").unwrap();
        let (file, size) = cache.get("a").unwrap();
        assert_eq!(size, 5);
        assert_eq!(read_all(file), b"hello");
        assert_eq!(cache.used(), 5);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn test_evicts_least_recently_used() {
        let dir = test_dir("evict");
        let cache = LruCache::new(&dir, 10).unwrap();

        cache.add("a", &[0; 4]).unwrap();
        cache.add("b", &[0; 4]).unwrap();
        cache.get("a").unwrap(); // now b is the oldest
        cache.add("c", &[0; 4]).unwrap();

        assert!(cache.get("a").is_some());
        assert!(cache.get("b").is_none());
        assert!(cache.get("c").is_some());
        assert_eq!(cache.used(), 8);
        assert_eq!(fs::read_dir(&dir).unwrap().count(), 2);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn test_big_item_evicts_several() {
        let dir = test_dir("several");
        let cache = LruCache::new(&dir, 10).unwrap();

        cache.add("a", &[0; 3]).unwrap();
        cache.add("b", &[0; 3]).unwrap();
        cache.add("c", &[0; 3]).unwrap();
        cache.add("big", &[0; 8]).unwrap(); // needs a, b and c gone

        assert!(cache.get("a").is_none());
        assert!(cache.get("b").is_none());
        assert!(cache.get("c").is_none());
        assert!(cache.get("big").is_some());
        assert_eq!(cache.used(), 8);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn test_too_big_rejected() {
        let dir = test_dir("too_big");
        let cache = LruCache::new(&dir, 10).unwrap();

        cache.add("a", &[0; 4]).unwrap();
        let err = cache.add("huge", &[0; 11]).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidInput);
        assert!(cache.get("a").is_some()); // nothing evicted
        assert_eq!(fs::read_dir(&dir).unwrap().count(), 1);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn test_duplicate_add_counts_as_use() {
        let dir = test_dir("dup");
        let cache = LruCache::new(&dir, 10).unwrap();

        cache.add("a", &[1; 4]).unwrap();
        cache.add("b", &[2; 4]).unwrap();
        let err = cache.add("a", &[9; 4]).unwrap_err(); // refreshes a, keeps old bytes
        assert_eq!(err.kind(), io::ErrorKind::AlreadyExists);
        cache.add("c", &[3; 4]).unwrap(); // evicts b, not a

        assert_eq!(read_all(cache.get("a").unwrap().0), [1; 4]);
        assert!(cache.get("b").is_none());
        assert_eq!(cache.used(), 8);
        fs::remove_dir_all(&dir).unwrap();
    }
}
